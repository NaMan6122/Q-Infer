//! Shared types for every Q-Infer surface.
//!
//! The rule from ADR-0002: this crate knows *nothing* about how a value was
//! obtained. It deserialises what the fingerprint layer produced and hands it
//! to the predictor, the planner and the TUI. Nothing here shells out.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};


pub mod bench;
pub mod catalog;
pub mod cfp;
pub mod collect;
pub mod engines;
pub mod microops;
pub mod planner;
pub mod predict;
pub mod probe;
pub mod session;
pub mod setup;
pub mod space;
pub mod spec;
pub mod sweetspot;

pub use bench::{BenchError, CodeScore};
pub use catalog::{Artifact, ByteSource, Catalog, CatalogEntry, CatalogViolation, Coverage, ModelMeta};
pub use cfp::{parse_major, Violation, MAX_SPREAD_PCT, SUPPORTED_MAJOR};
pub use engines::{adapter_for_format, detect, Adapter, BenchRequest, BenchResult, Engine};
pub use microops::{MicroOp, OpId, Outcome, Report};
pub use planner::{Constraints, Objective, Plan, PlanRequest, PlanSet, Planner, Rejection};
pub use predict::{Calibration, KvGeometry, Observation, Prediction, ShapeClass};
pub use probe::{probe, ProbeConfig, ProbeError, ProbeResult};
pub use session::{Session, SessionError};
pub use setup::{Measurements, SetupEvent, Stage};
pub use space::{DeviceInfo, Space, SpaceSource};
pub use sweetspot::{CapabilitySource, Point, SweetSpot, DEFAULT_USABLE_TPS};

// ---------------------------------------------------------------------------
// qinfer/cfp@1 — Capability Fingerprint
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Fingerprint {
    pub schema: String,
    #[serde(default)]
    pub collected_at: String,
    pub identity: Identity,
    pub cpu: Cpu,
    #[serde(default)]
    pub gpu: Gpu,
    pub memory: Memory,
    pub bandwidth: Bandwidth,
    #[serde(default)]
    pub power: Power,
    #[serde(default)]
    pub provenance: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Identity {
    pub os: String,
    #[serde(default)]
    pub os_version: Option<String>,
    /// Build number, e.g. macOS `26A434`. Distinguishes two machines on the
    /// same point release, which is where firmware regressions hide.
    #[serde(default)]
    pub os_build: Option<String>,
    pub arch: String,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cpu {
    pub brand: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    /// `sm_121`, `gfx1151`, `apple-m5-pro`, `X1E80100`. Facts, not brands
    /// (ADR-0002) — the planner keys on this, so it is a first-class field.
    #[serde(default)]
    pub arch_id: Option<String>,
    #[serde(default)]
    pub perf_levels: Vec<PerfLevel>,
    #[serde(default)]
    pub isa: BTreeMap<String, i64>,
    #[serde(default)]
    pub l3_bytes: Option<u64>,
    #[serde(default)]
    pub max_freq_mhz: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PerfLevel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub physical_cores: Option<u32>,
    #[serde(default)]
    pub l2_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Gpu {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub metal_support: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub unified_memory: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Memory {
    pub total_bytes: u64,
    /// What the planner is actually allowed to use. Specified in PRD §5.1 and
    /// required before fit arithmetic can be honest — `None` here means the
    /// planner must fall back to a stated assumption, not to the total.
    #[serde(default)]
    pub available_bytes: Option<u64>,
    #[serde(default)]
    pub swap_used_bytes: Option<u64>,
    #[serde(default)]
    pub swap_raw: Option<String>,
}

/// Whether the machine is thrashing — and when we genuinely cannot say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PressureState {
    /// `None` when pressure was inferred from swap-in-use rather than parsed
    /// headroom — we do not fabricate a percentage we did not measure.
    Relaxed { swap_free_pct: Option<f64> },
    Pressured { swap_free_pct: Option<f64> },
    Unknown,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Power {
    #[serde(default)]
    pub low_power_mode: Option<bool>,
    #[serde(default)]
    pub thermal_pressure: Option<String>,
    #[serde(default)]
    pub thermal_source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Bandwidth {
    #[serde(default)]
    pub copy: Option<Sample>,
    #[serde(default)]
    pub triad: Option<Sample>,
    /// Required, not optional: charter §4.2 and PRD §20.2.2 both demand the
    /// single-thread figure, and an `Option` here let consumers silently
    /// substitute the saturated number (Critic finding, v0.2).
    pub single_thread_gbs: f64,
    /// Thread count -> GB/s. Keys arrive as strings from JSON.
    #[serde(default)]
    pub thread_sweep: BTreeMap<String, f64>,
    #[serde(default)]
    pub best_threads: Option<u32>,
    pub headline_gbs: f64,
    /// Which access pattern produced `headline_gbs`. Comparing a `triad`
    /// headline against a `copy` single-thread figure is a guaranteed false
    /// positive — `triad` moves 3 arrays per 2 for `copy`.
    ///
    /// Deliberately **not** defaulted: assuming "copy" would be the plausible
    /// default ADR-0002 forbids, and it silently enabled both ordering rules
    /// for documents that never declared a method.
    #[serde(default)]
    pub headline_method: Option<String>,
    #[serde(default)]
    pub buffer_mib: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sample {
    pub method: String,
    pub best_gbs: f64,
    pub median_gbs: f64,
    pub spread_pct: f64,
    /// Retained so "measured" with `reps: 1` is distinguishable from best-of-7,
    /// and so the 4x-LLC floor is checkable (PRD §20.2.1).
    #[serde(default)]
    pub reps: Option<u32>,
    #[serde(default)]
    pub buffer_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// M0 spike results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpikeResults {
    pub bandwidth_gbs: f64,
    #[serde(default)]
    pub bandwidth_single_thread_gbs: Option<f64>,
    #[serde(default)]
    pub reps: u32,
    pub fit: Fit,
    pub mape: Mape,
    pub rows: Vec<SpikeRow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Fit {
    pub t0_ms: f64,
    pub eps: f64,
    pub r2: f64,
    #[serde(default)]
    pub determined: bool,
    #[serde(default)]
    pub n_models: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Mape {
    pub naive_pct: f64,
    pub two_param_pct: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpikeRow {
    pub model: String,
    pub gib: f64,
    pub measured_tps: f64,
    #[serde(default)]
    pub pp_tps: f64,
    pub naive_tps: f64,
    pub naive_err_pct: f64,
    pub two_param_tps: f64,
    pub two_param_err_pct: f64,
    #[serde(default)]
    pub implied_effective_gbs: f64,
    #[serde(default)]
    pub implied_eps: f64,
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub backend: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Fingerprint {
    /// Saturated bandwidth is the number the predictor uses (PRD §5.2).
    pub fn saturated_gbs(&self) -> f64 {
        self.bandwidth.headline_gbs
    }

    pub fn single_thread_gbs(&self) -> f64 {
        self.bandwidth.single_thread_gbs
    }

    /// How much the single-threaded probe under-reports. Anything above ~1.3
    /// means a naive probe would have halved the prediction (spike F1).
    pub fn thread_gain(&self) -> Option<f64> {
        let st = self.single_thread_gbs();
        if st > 0.0 && st.is_finite() {
            Some(self.saturated_gbs() / st)
        } else {
            None
        }
    }

    pub fn total_gib(&self) -> f64 {
        self.memory.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn swap_gib(&self) -> f64 {
        self.memory.swap_used_bytes.unwrap_or(0) as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    /// Free fraction of *configured swap*, parsed from `swap_raw`.
    ///
    /// The Critic was right that `swap_used / RAM` is the wrong denominator: the
    /// reference machine sat at 48.1% and passed while swap was 96.3% full with
    /// 463 MB left. Headroom is the honest signal.
    pub fn swap_headroom(&self) -> Option<f64> {
        let raw = self.memory.swap_raw.as_deref()?;
        let total = parse_mib(raw, "total")?;
        let free = parse_mib(raw, "free")?;
        if total > 0.0 { Some(free / total) } else { None }
    }

    /// Three-valued: "I cannot tell" must never collapse into "fine".
    ///
    /// The second Critic showed that an unparseable `swap_raw` (Linux style)
    /// with no `swap_used_bytes` made a machine at 100% swap look healthy.
    pub fn pressure(&self) -> PressureState {
        if let Some(headroom) = self.swap_headroom() {
            return if headroom < crate::cfp::MIN_SWAP_HEADROOM {
                PressureState::Pressured { swap_free_pct: Some(headroom) }
            } else {
                PressureState::Relaxed { swap_free_pct: Some(headroom) }
            };
        }

        // A non-empty `swap_raw` we could not parse means the format is unknown,
        // not that the machine is idle.
        if self.memory.swap_raw.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
            return PressureState::Unknown;
        }

        if let Some(used) = self.memory.swap_used_bytes {
            let total = self.total_gib();
            if total > 0.0 {
                let ratio = (used as f64 / (1024.0 * 1024.0 * 1024.0)) / total;
                // No parsed headroom here, so the percentage is genuinely
                // unknown — report `None` rather than inventing 0 % or 100 %.
                return if ratio > 0.5 {
                    PressureState::Pressured { swap_free_pct: None }
                } else {
                    PressureState::Relaxed { swap_free_pct: None }
                };
            }
        }

        PressureState::Unknown
    }

    /// A machine in deep swap produces garbage bandwidth numbers. The
    /// fingerprint is still valid, but predictions built on it are not.
    pub fn memory_pressured(&self) -> bool {
        matches!(self.pressure(), PressureState::Pressured { .. })
    }

    pub fn isa_enabled(&self) -> Vec<&str> {
        self.cpu
            .isa
            .iter()
            .filter(|(_, v)| **v != 0)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// e.g. "5 Super + 10 Performance". Names come from the OS and are not
    /// stable across generations (M5 uses "Super", M1–M4 use "Performance"),
    /// so this is display-only and never parsed.
    pub fn perf_summary(&self) -> String {
        let parts: Vec<String> = self
            .cpu
            .perf_levels
            .iter()
            .filter_map(|p| {
                let n = p.physical_cores?;
                Some(match p.name.as_deref() {
                    Some(name) if !name.is_empty() => format!("{} {}", n, name),
                    _ => format!("{} cores", n),
                })
            })
            .collect();
        if parts.is_empty() {
            format!("{} cores", self.cpu.physical_cores)
        } else {
            parts.join(" + ")
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Where the dashboard looks for data. M0 reads the spike artefacts directly so
/// the UI can be built and tested before the planner exists.
/// Sealed module: the tuple field is private to this module, so no other module
/// in the crate can construct a `Validated` without going through `try_from`.
/// (The second Critic found that a crate-root `Validated(fp)` compiled fine from
/// child modules — an invariant with a hole is not an invariant.)
mod validated {
    use super::{Fingerprint, Violation};
    use std::ops::Deref;

    /// A fingerprint that has passed [`Fingerprint::validate`].
    ///
    /// The first Critic's finding was that a voluntary `validate()` call is not
    /// an invariant — nothing called it. A component that needs trustworthy
    /// hardware facts takes `&Validated<Fingerprint>` and therefore *cannot* be
    /// handed an unvalidated one.
    ///
    /// Construction is sealed to this module:
    ///
    /// ```compile_fail
    /// use qinfer_core::Validated;
    /// struct Unvalidated;
    /// let _skip_validation = Validated(Unvalidated);
    /// ```
    #[derive(Debug, Clone)]
    pub struct Validated<T>(T);

    impl<T> Deref for Validated<T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }

    impl TryFrom<Fingerprint> for Validated<Fingerprint> {
        type Error = Vec<Violation>;
        fn try_from(fp: Fingerprint) -> Result<Self, Self::Error> {
            fp.validate()?;
            Ok(Validated(fp))
        }
    }

    impl<'de> serde::Deserialize<'de> for Validated<Fingerprint> {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let fp = Fingerprint::deserialize(deserializer)?;
            Validated::try_from(fp).map_err(|violations| {
                let msg = violations
                    .iter()
                    .map(|v| format!("{}: {}", v.code(), v))
                    .collect::<Vec<_>>()
                    .join("; ");
                serde::de::Error::custom(msg)
            })
        }
    }
}

pub use validated::Validated;

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    /// Raw fingerprint, for display only. Never feed this to the planner.
    pub fingerprint: Option<Fingerprint>,
    /// Present only if the fingerprint passed validation. This is what the
    /// planner and predictor are expected to take.
    pub trusted: Option<Validated<Fingerprint>>,
    pub violations: Vec<Violation>,
    pub results: Option<SpikeResults>,
    pub error: Option<String>,
}

impl Workspace {
    pub fn discover() -> Self {
        let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let root = find_spike_dir(&start).unwrap_or(start);
        Self::load(root)
    }

    pub fn load(root: PathBuf) -> Self {
        let mut ws = Workspace {
            root: root.clone(),
            fingerprint: None,
            trusted: None,
            violations: Vec::new(),
            results: None,
            error: None,
        };

        let fp_path = root.join("fingerprint.json");
        match std::fs::read_to_string(&fp_path) {
            Ok(s) => {
                // Trusted path goes through `Validated`'s Deserialize impl, so
                // the type system — not this function — decides what is trusted.
                match serde_json::from_str::<Validated<Fingerprint>>(&s) {
                    Ok(v) => {
                        ws.fingerprint = Some((*v).clone());
                        ws.trusted = Some(v);
                    }
                    Err(trust_err) => match serde_json::from_str::<Fingerprint>(&s) {
                        // Rejected: keep the raw document for display, record why.
                        Ok(fp) => {
                            ws.violations = fp.validate().err().unwrap_or_default();
                            ws.fingerprint = Some(fp);
                            if ws.violations.is_empty() {
                                ws.error = Some(format!("{}: {}", fp_path.display(), trust_err));
                            }
                        }
                        Err(e) => ws.error = Some(format!("{}: {}", fp_path.display(), e)),
                    },
                }
            }
            Err(_) => {
                ws.error = Some(format!(
                    "no fingerprint at {} — run `python3 spikes/m0/fingerprint.py`",
                    fp_path.display()
                ))
            }
        }

        let res_path = root.join("results.json");
        if let Ok(s) = std::fs::read_to_string(&res_path) {
            match serde_json::from_str::<SpikeResults>(&s) {
                Ok(r) => ws.results = Some(r),
                Err(e) => {
                    if ws.error.is_none() {
                        ws.error = Some(format!("{}: {}", res_path.display(), e));
                    }
                }
            }
        }

        ws
    }

    pub fn reload(&mut self) {
        *self = Self::load(self.root.clone());
    }

    pub fn has_data(&self) -> bool {
        self.fingerprint.is_some()
    }

    /// True only when the fingerprint passed every invariant.
    pub fn is_trusted(&self) -> bool {
        self.trusted.is_some()
    }
}

/// Pull `<key> = <number>M` out of macOS's `vm.swapusage` string.
fn parse_mib(raw: &str, key: &str) -> Option<f64> {
    let idx = raw.find(key)?;
    let rest = &raw[idx + key.len()..];
    let eq = rest.find('=')?;
    let rest = rest[eq + 1..].trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().ok()
}

fn find_spike_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join("spikes").join("m0");
        if candidate.join("fingerprint.json").exists() {
            return Some(candidate);
        }
        if dir.join("fingerprint.json").exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_gain_reflects_single_vs_saturated() {
        let mut sweep = BTreeMap::new();
        sweep.insert("15".to_string(), 228.0);
        let fp = Fingerprint {
            schema: "qinfer/cfp@1".into(),
            collected_at: String::new(),
            identity: Identity {
                os: "macos".into(),
                os_version: None,
                os_build: None,
                arch: "arm64".into(),
                kernel: None,
                hostname: None,
            },
            cpu: Cpu {
                brand: "Apple M5 Pro".into(),
                physical_cores: 15,
                logical_cores: 15,
                arch_id: Some("apple-m5-pro".into()),
                perf_levels: vec![],
                isa: BTreeMap::new(),
                l3_bytes: None,
                max_freq_mhz: None,
            },
            gpu: Gpu::default(),
            memory: Memory { total_bytes: 24 * 1024 * 1024 * 1024, available_bytes: None, swap_used_bytes: None, swap_raw: None },
            bandwidth: Bandwidth {
                copy: None,
                triad: None,
                single_thread_gbs: 131.0,
                thread_sweep: sweep,
                best_threads: Some(15),
                headline_gbs: 228.0,
                headline_method: Some("copy".into()),
                buffer_mib: None,
            },
            power: Power::default(),
            provenance: BTreeMap::new(),
        };

        let gain = fp.thread_gain().expect("gain");
        assert!((gain - 1.74).abs() < 0.01, "got {gain}");
        assert!(!fp.memory_pressured());
    }

    /// N3: the boundary where validation actually happens had no test at all.
    #[test]
    fn workspace_load_refuses_the_committed_fingerprint() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../spikes/m0/fixtures");
        let ws = Workspace::load(root);
        assert!(ws.fingerprint.is_some(), "raw document is kept for display");
        assert!(ws.trusted.is_none(), "but it must never be exposed as trusted");
        let codes: Vec<&'static str> = ws.violations.iter().map(|v| v.code()).collect();
        assert_eq!(codes.len(), 5, "expected five violations, got {codes:?}");
        assert!(codes.contains(&"E-DET-009"), "{codes:?}");
        assert!(codes.contains(&"E-DET-003"), "{codes:?}");
        // The artefact predates `headline_method`, so the method is unknowable.
        assert!(codes.contains(&"E-DET-017"), "{codes:?}");
    }

    #[test]
    fn heavy_swap_marks_memory_pressured() {
        let fp = Fingerprint {
            schema: "x".into(),
            collected_at: String::new(),
            identity: Identity { os: "macos".into(), os_version: None, os_build: None, arch: "arm64".into(), kernel: None, hostname: None },
            cpu: Cpu { brand: "x".into(), physical_cores: 1, logical_cores: 1, arch_id: None, perf_levels: vec![], isa: BTreeMap::new(), l3_bytes: None, max_freq_mhz: None },
            gpu: Gpu::default(),
            memory: Memory { total_bytes: 8 << 30, available_bytes: None, swap_used_bytes: Some(6 << 30), swap_raw: None },
            bandwidth: Bandwidth { copy: None, triad: None, single_thread_gbs: 10.0, thread_sweep: BTreeMap::new(), best_threads: Some(1), headline_gbs: 100.0, headline_method: Some("copy".into()), buffer_mib: None },
            power: Power::default(),
            provenance: BTreeMap::new(),
        };
        assert!(fp.memory_pressured());
    }
}
