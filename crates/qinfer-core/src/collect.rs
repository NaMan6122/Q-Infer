//! Platform collectors (T-003).
//!
//! A collector turns OS probes into a `Fingerprint`. Two design rules:
//!
//! 1. **Every trap becomes a test.** The macOS probing surface has documented
//!    landmines (PRD Appendix D) — `hw.l3cachesize` does not exist on Apple
//!    silicon, the i8mm sysctl key is all-caps while dotprod is mixed case,
//!    `platform.processor()` returns an empty string. A collector that "works
//!    on my machine" is not evidence; a parser test is.
//! 2. **Process execution is injected.** Parsers are pure functions over text,
//!    so the whole collector is testable on a Linux CI runner with recorded
//!    macOS output.
//!
//! The bandwidth figure is *not* collected here — it comes from the T-002 probe
//! and is passed in, because a collector that silently skipped the probe would
//! reintroduce exactly the mistake `cfp@1` exists to catch.

use crate::probe::ProbeResult;
use crate::{Bandwidth, Fingerprint, Sample, MAX_SPREAD_PCT};
use std::collections::BTreeMap;

/// Runs a program and returns trimmed stdout, or `None` on any failure.
/// Injected so tests never shell out.
pub trait Runner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

/// The real thing.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(program).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CollectError {
    MissingArchitecture,
    MissingMemory,
}

impl std::fmt::Display for CollectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectError::MissingArchitecture => write!(f, "could not determine CPU architecture"),
            CollectError::MissingMemory => write!(f, "could not determine physical memory"),
        }
    }
}

/// `Apple M5 Pro` -> `apple-m5-pro`. Facts, not brands; stable across reboots.
pub fn slugify(brand: &str) -> String {
    brand
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Days since the Unix epoch -> `YYYY-MM-DDTHH:MM:SSZ`, without a date library.
fn iso8601(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs = epoch_secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub mod macos {
    use super::*;
    use crate::{Cpu, Gpu, Identity, Memory, PerfLevel, Power};
    use serde::Deserialize;

    /// The sysctl keys worth asking for. Casing is not uniform in the OS —
    /// `FEAT_DotProd` is mixed case, `FEAT_I8MM` is all caps — so these are
    /// literal, and a lowercase probe silently returns nothing.
    pub const ISA_KEYS: [&str; 7] = [
        "hw.optional.arm.FEAT_DotProd",
        "hw.optional.arm.FEAT_I8MM",
        "hw.optional.arm.FEAT_BF16",
        "hw.optional.arm.FEAT_SME",
        "hw.optional.arm.FEAT_SME2",
        "hw.optional.arm.FEAT_SME2p1",
        "hw.optional.arm.FEAT_FP8",
    ];

    fn sysctl(r: &impl Runner, key: &str) -> Option<String> {
        let v = r.run("sysctl", &["-n", key])?;
        if v.is_empty() { None } else { Some(v) }
    }

    fn sysctl_i64(r: &impl Runner, key: &str) -> Option<i64> {
        sysctl(r, key)?.parse().ok()
    }

    #[derive(Deserialize)]
    struct DisplayReport {
        #[serde(rename = "SPDisplaysDataType", default)]
        entries: Vec<Display>,
    }

    #[derive(Deserialize)]
    struct Display {
        #[serde(default)]
        sppci_model: Option<String>,
        /// A *string* in the plist, e.g. "16", but treated as an opaque value:
        /// one unexpected type must not discard the whole GPU record
        /// (verification finding 2).
        #[serde(default)]
        sppci_cores: Option<serde_json::Value>,
        #[serde(default)]
        spdisplays_mtlgpufamilysupport: Option<String>,
        #[serde(default)]
        sppci_vendor: Option<String>,
    }

    pub fn parse_gpu(json: &str) -> Gpu {
        let Ok(report) = serde_json::from_str::<DisplayReport>(json) else {
            return Gpu::default();
        };
        let Some(e) = report.entries.into_iter().next() else {
            return Gpu::default();
        };
        // Coerce rather than require a single type: a numeric `sppci_cores`
        // must still yield the model name, not an empty GPU record.
        let cores = e.sppci_cores.and_then(|v| match v {
            serde_json::Value::String(s) => s.trim().parse::<u32>().ok(),
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            _ => None,
        });
        Gpu {
            model: e.sppci_model,
            cores,
            metal_support: e.spdisplays_mtlgpufamilysupport,
            vendor: e.sppci_vendor,
            unified_memory: true,
        }
    }

    pub fn parse_power(pmset: &str) -> Power {
        let low = pmset.lines().find_map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some("lowpowermode"), Some(v)) => Some(v == "1"),
                _ => None,
            }
        });
        Power {
            low_power_mode: low,
            thermal_pressure: None,
            thermal_source: Some("unavailable-without-root".into()),
        }
    }

    fn collect_isa(r: &impl Runner) -> BTreeMap<String, i64> {
        let mut isa = BTreeMap::new();
        for key in ISA_KEYS {
            // Absence means "not supported", which is a fact, not an error.
            let present = sysctl_i64(r, key).unwrap_or(0);
            if let Some(name) = key.rsplit('.').next() {
                isa.insert(name.to_string(), present);
            }
        }
        isa
    }

    fn collect_perf_levels(r: &impl Runner) -> Vec<PerfLevel> {
        let Some(n) = sysctl_i64(r, "hw.nperflevels") else {
            return Vec::new();
        };
        (0..n)
            .map(|i| PerfLevel {
                name: sysctl(r, &format!("hw.perflevel{i}.name")),
                physical_cores: sysctl_i64(r, &format!("hw.perflevel{i}.physicalcpu"))
                    .and_then(|v| u32::try_from(v).ok()),
                l2_bytes: sysctl_i64(r, &format!("hw.perflevel{i}.l2cachesize"))
                    .and_then(|v| u64::try_from(v).ok()),
            })
            .collect()
    }

    /// Collect a fingerprint on macOS. `probe` is the T-002 bandwidth result.
    pub fn collect(r: &impl Runner, probe: &ProbeResult) -> Result<Fingerprint, CollectError> {
        let arch = r.run("uname", &["-m"]).ok_or(CollectError::MissingArchitecture)?;

        let brand = sysctl(r, "machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into());
        let physical_cores = sysctl_i64(r, "hw.physicalcpu").unwrap_or(1).max(1) as u32;
        let logical_cores = sysctl_i64(r, "hw.logicalcpu").unwrap_or(physical_cores as i64).max(1) as u32;

        let total_bytes = sysctl_i64(r, "hw.memsize")
            .and_then(|v| u64::try_from(v).ok())
            .ok_or(CollectError::MissingMemory)?;

        let swap_raw = sysctl(r, "vm.swapusage");
        let swap_used_bytes = swap_raw.as_deref().and_then(|raw| {
            let idx = raw.find("used")?;
            let rest = &raw[idx..];
            let eq = rest.find('=')?;
            let num: String = rest[eq + 1..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num.parse::<f64>().ok().map(|mib| (mib * 1024.0 * 1024.0) as u64)
        });

        let gpu = r
            .run("system_profiler", &["SPDisplaysDataType", "-json"])
            .map(|j| parse_gpu(&j))
            .unwrap_or_default();

        let power = r.run("pmset", &["-g"]).map(|s| parse_power(&s)).unwrap_or(Power {
            low_power_mode: None,
            thermal_pressure: None,
            thermal_source: Some("unavailable-without-root".into()),
        });

        let collected_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| iso8601(d.as_secs()))
            .unwrap_or_default();

        let mut fp = Fingerprint {
            schema: "qinfer/cfp@1".into(),
            collected_at,
            identity: Identity {
                os: "macos".into(),
                os_version: r.run("sw_vers", &["-productVersion"]),
                os_build: r.run("sw_vers", &["-buildVersion"]),
                arch,
                kernel: r.run("uname", &["-r"]),
                hostname: r.run("uname", &["-n"]),
            },
            cpu: Cpu {
                arch_id: Some(slugify(&brand)),
                brand,
                physical_cores,
                logical_cores,
                // Absent on Apple silicon. Must stay None, not 0.
                l3_bytes: sysctl_i64(r, "hw.l3cachesize").and_then(|v| u64::try_from(v).ok()),
                // Also absent on Apple silicon; recorded as None rather than invented.
                max_freq_mhz: None,
                perf_levels: collect_perf_levels(r),
                isa: collect_isa(r),
            },
            gpu,
            memory: Memory { total_bytes, available_bytes: None, swap_used_bytes, swap_raw },
            bandwidth: Bandwidth {
                copy: Some(Sample {
                    method: "copy".into(),
                    best_gbs: probe.saturated_gbs,
                    median_gbs: probe.median_gbs,
                    spread_pct: probe.spread_pct,
                    reps: Some(probe.reps as u32),
                    buffer_bytes: Some(probe.buffer_bytes as u64),
                }),
                triad: None,
                single_thread_gbs: probe.single_thread_gbs,
                thread_sweep: probe
                    .sweep
                    .iter()
                    .map(|(k, v)| (k.to_string(), *v))
                    .collect(),
                best_threads: Some(probe.best_threads as u32),
                headline_gbs: probe.saturated_gbs,
                headline_method: Some("copy".into()),
                buffer_mib: Some((probe.buffer_bytes / (1024 * 1024)) as u32),
            },
            power,
            provenance: BTreeMap::new(),
        };

        // Honesty at the source: if the probe was unstable or the machine is
        // paging, the number is not a measurement and must not claim to be.
        let stable = probe.spread_pct.is_finite() && probe.spread_pct <= MAX_SPREAD_PCT;
        let pressured = fp.memory_pressured();
        let claim = if stable && !pressured { "measured" } else { "unknown" };
        fp.provenance.insert("bandwidth".into(), claim.into());
        fp.provenance.insert("cpu".into(), "exact".into());
        fp.provenance.insert("gpu".into(), "exact".into());
        fp.provenance.insert("thermal".into(), "unknown".into());

        Ok(fp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ProbeResult;

    struct Fake(BTreeMap<String, String>);

    impl Fake {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Fake(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
        }
    }

    impl Runner for Fake {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            self.0.get(&format!("{} {}", program, args.join(" "))).cloned()
        }
    }

    fn probe(spread: f64) -> ProbeResult {
        let mut sweep = BTreeMap::new();
        sweep.insert(1usize, 131.4);
        sweep.insert(15usize, 228.0);
        ProbeResult {
            single_thread_gbs: 131.4,
            saturated_gbs: 228.0,
            median_gbs: 224.0,
            best_threads: 15,
            sweep,
            spread_pct: spread,
            buffer_bytes: 256 * 1024 * 1024,
            reps: 5,
        }
    }

    fn healthy_runner() -> Fake {
        Fake::new(&[
            ("uname -m", "arm64"),
            ("uname -r", "25.0.0"),
            ("uname -n", "spark.local"),
            ("sw_vers -productVersion", "27.0.1"),
            ("sw_vers -buildVersion", "26A434"),
            ("sysctl -n machdep.cpu.brand_string", "Apple M5 Pro"),
            ("sysctl -n hw.physicalcpu", "15"),
            ("sysctl -n hw.logicalcpu", "15"),
            ("sysctl -n hw.memsize", "25769803776"),
            ("sysctl -n hw.nperflevels", "2"),
            ("sysctl -n hw.perflevel0.name", "Super"),
            ("sysctl -n hw.perflevel0.physicalcpu", "5"),
            ("sysctl -n hw.perflevel0.l2cachesize", "16777216"),
            ("sysctl -n hw.perflevel1.name", "Performance"),
            ("sysctl -n hw.perflevel1.physicalcpu", "10"),
            ("sysctl -n hw.perflevel1.l2cachesize", "8388608"),
            ("sysctl -n hw.optional.arm.FEAT_DotProd", "1"),
            ("sysctl -n hw.optional.arm.FEAT_I8MM", "1"),
            ("sysctl -n hw.optional.arm.FEAT_BF16", "1"),
            ("sysctl -n hw.optional.arm.FEAT_SME", "1"),
            ("sysctl -n hw.optional.arm.FEAT_SME2", "1"),
            ("sysctl -n hw.optional.arm.FEAT_SME2p1", "1"),
            ("sysctl -n hw.optional.arm.FEAT_FP8", "0"),
            ("sysctl -n vm.swapusage", "total = 8192.00M  used = 0.00M  free = 8192.00M"),
            (
                "system_profiler SPDisplaysDataType -json",
                r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M5 Pro","sppci_cores":"16","spdisplays_mtlgpufamilysupport":"spdisplays_metal4"}]}"#,
            ),
            ("pmset -g", "System-wide power settings:\n lowpowermode         0"),
        ])
    }

    #[test]
    fn slugify_produces_a_stable_arch_id() {
        assert_eq!(slugify("Apple M5 Pro"), "apple-m5-pro");
        assert_eq!(slugify("Apple M3 Ultra"), "apple-m3-ultra");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn iso8601_formats_a_known_epoch() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn collects_a_complete_fingerprint() {
        let fp = macos::collect(&healthy_runner(), &probe(3.0)).expect("collect");
        assert_eq!(fp.identity.arch, "arm64");
        assert_eq!(fp.cpu.brand, "Apple M5 Pro");
        assert_eq!(fp.cpu.arch_id.as_deref(), Some("apple-m5-pro"));
        assert_eq!(fp.cpu.physical_cores, 15);
        assert_eq!(fp.gpu.cores, Some(16));
        assert_eq!(fp.memory.total_bytes, 25_769_803_776);
        assert_eq!(fp.bandwidth.best_threads, Some(15));
        assert_eq!(fp.bandwidth.headline_method.as_deref(), Some("copy"));
    }

    #[test]
    fn a_healthy_machine_is_marked_measured_and_validates() {
        let fp = macos::collect(&healthy_runner(), &probe(3.0)).expect("collect");
        assert_eq!(fp.provenance.get("bandwidth").map(String::as_str), Some("measured"));
        assert_eq!(fp.validate(), Ok(()), "collector output must pass cfp@1");
    }

    #[test]
    fn an_unstable_probe_degrades_to_unknown_rather_than_lying() {
        let fp = macos::collect(&healthy_runner(), &probe(64.2)).expect("collect");
        assert_eq!(fp.provenance.get("bandwidth").map(String::as_str), Some("unknown"));
        assert_eq!(fp.validate(), Ok(()), "an honest unknown must still validate");
    }

    #[test]
    fn missing_l3_cache_is_none_not_zero() {
        // No `hw.l3cachesize` entry in the fake: Apple silicon does not have one.
        let fp = macos::collect(&healthy_runner(), &probe(3.0)).expect("collect");
        assert_eq!(fp.cpu.l3_bytes, None);
    }

    #[test]
    fn isa_flags_are_detected_despite_inconsistent_casing() {
        let fp = macos::collect(&healthy_runner(), &probe(3.0)).expect("collect");
        assert_eq!(fp.cpu.isa.get("FEAT_DotProd"), Some(&1));
        assert_eq!(fp.cpu.isa.get("FEAT_I8MM"), Some(&1), "all-caps key must be found");
        assert_eq!(fp.cpu.isa.get("FEAT_FP8"), Some(&0), "absent capability is 0, not missing");
    }

    #[test]
    fn missing_architecture_is_an_error_not_a_default() {
        let runner = Fake::new(&[("sysctl -n hw.memsize", "1024")]);
        assert_eq!(macos::collect(&runner, &probe(3.0)).unwrap_err(), CollectError::MissingArchitecture);
    }

    #[test]
    fn missing_memory_is_an_error_not_a_guess() {
        let mut map = healthy_runner().0;
        map.remove("sysctl -n hw.memsize");
        let err = macos::collect(&Fake(map), &probe(3.0)).unwrap_err();
        assert_eq!(err, CollectError::MissingMemory);
    }

    #[test]
    fn gpu_cores_are_parsed_from_a_string() {
        let json = r#"{"SPDisplaysDataType":[{"sppci_model":"X","sppci_cores":"40"}]}"#;
        assert_eq!(macos::parse_gpu(json).cores, Some(40));
    }

    #[test]
    fn malformed_gpu_json_does_not_panic() {
        let gpu = macos::parse_gpu("not json");
        assert_eq!(gpu.model, None);
        assert!(!gpu.unified_memory);
    }

    #[test]
    fn power_mode_is_parsed() {
        let p = macos::parse_power("System-wide power settings:\n lowpowermode         1");
        assert_eq!(p.low_power_mode, Some(true));
    }

    #[test]
    fn perf_levels_keep_their_names_and_core_counts() {
        let fp = macos::collect(&healthy_runner(), &probe(3.0)).expect("collect");
        assert_eq!(fp.cpu.perf_levels.len(), 2);
        assert_eq!(fp.cpu.perf_levels[0].name.as_deref(), Some("Super"));
        assert_eq!(fp.cpu.perf_levels[0].physical_cores, Some(5));
        assert_eq!(fp.cpu.perf_levels[1].l2_bytes, Some(8_388_608));
    }

    #[test]
    fn the_live_collector_runs_on_this_machine() {
        // Integration smoke: only meaningful on macOS, where it must produce a
        // fingerprint that passes its own validation or explains why not.
        if std::env::consts::OS != "macos" {
            return;
        }
        let probe = crate::probe::probe(&crate::ProbeConfig {
            buffer_bytes: 8 * 1024 * 1024,
            reps: 1,
            thread_counts: vec![1],
        });
        let fp = macos::collect(&SystemRunner, &probe).expect("collect on host");
        assert!(!fp.cpu.brand.is_empty());
        assert!(fp.memory.total_bytes > 0);
        assert!(fp.cpu.arch_id.is_some());
        // No carve-outs: the collector's output must survive its own validator.
        // The earlier `E-DET-014` exception was an admission that the T-001 and
        // T-002 contracts disagreed (Critic N6); that is fixed, so the
        // exception is gone.
        if let Err(v) = fp.validate() {
            let codes: Vec<String> = v.iter().map(|x| format!("{}: {}", x.code(), x)).collect();
            panic!("live fingerprint failed validation: {codes:#?}");
        }
    }
}
