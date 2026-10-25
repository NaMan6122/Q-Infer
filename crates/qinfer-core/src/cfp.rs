//! Validation for `qinfer/cfp@1`.
//!
//! The fingerprint is the only input to planning (ADR-0002). A corrupt one does
//! not fail loudly downstream — it silently produces plausible, wrong plans, and
//! then poisons the calibration store with the error. So the invariants live
//! here and are checked on the way in.
//!
//! Every rule below exists because something concrete went wrong:
//!
//! - single-threaded probes under-report (1.74x on the reference M5 Pro), so a
//!   sweep that fails to beat one thread is a defect, not a data point;
//! - `triad` moves three arrays per two for `copy` and is therefore
//!   systematically lower — comparing across methods false-positives on a
//!   perfectly healthy machine (this repository's own `fingerprint.json`);
//! - a machine that is 96% through its swap reports numbers that look fine and
//!   are not, so pressure is measured as swap *headroom*, not swap-used-vs-RAM.
//!
//! Revision note: v0.2 after Critic review. The first version was inert — it had
//! no production caller and one of its five rules was provably wrong.

use crate::Fingerprint;

/// Highest `cfp@N` major this build understands.
pub const SUPPORTED_MAJOR: u32 = 1;

/// Above this run-to-run spread, a bandwidth figure is not a measurement.
pub const MAX_SPREAD_PCT: f64 = 15.0;

/// Below this fraction of free swap, the machine is thrashing.
pub const MIN_SWAP_HEADROOM: f64 = 0.10;

/// A multi-threaded sweep must beat one thread by at least this much to have
/// done anything at all. Deliberately loose — single-core hosts legitimately
/// report `best_threads == 1` and skip the check entirely.
pub const MIN_THREAD_GAIN: f64 = 1.05;

/// Below this many threads, "no gain" is not evidence of a defect: a 2-core
/// machine can legitimately be flat, and the probe's rounding makes the
/// difference indistinguishable from noise (Critic N6).
pub const MIN_SWEEP_THREADS: u32 = 4;

const SCHEMA_PREFIX: &str = "qinfer/cfp@";
const KNOWN_PROVENANCE: [&str; 4] = ["measured", "exact", "inferred", "unknown"];

#[derive(Debug, Clone, PartialEq)]
pub enum Violation {
    UnknownSchemaPrefix { found: String },
    UnknownSchemaMajor { found: String, supported: u32 },
    NonPositiveBandwidth { value: f64 },
    NonFiniteSingleThread { value: f64 },
    SaturatedBelowSingleThread { single_gbs: f64, saturated_gbs: f64 },
    ThreadSweepHadNoEffect { gain: f64, best_threads: u32, floor: f64 },
    UnstableBandwidth { sample_method: String, spread_pct: f64, threshold: f64 },
    /// The headline figure agrees with nothing that was measured (Critic N1).
    HeadlineNotCorroborated { headline_gbs: f64, expected_gbs: f64, source: String },
    /// No `headline_method`, so we cannot know what the headline measured (F1).
    MissingHeadlineMethod,
    InvalidHeadlineMethod { value: String },
    /// A declared method with no sample to back it (F2).
    HeadlineMethodWithoutSample { method: String },
    PressuredMachineAssertedMeasured { swap_free_pct: Option<f64>, swap_gib: f64, total_gib: f64 },
    /// Pressure cannot be established, so `measured` is not a claim we can honour.
    PressureUnknownAssertedMeasured,
    MissingProvenance,
    InvalidProvenance { value: String },
    NoSamplesForMeasured,
    BufferTooSmall { sample_method: String, buffer_bytes: u64, required_bytes: u64 },
}

impl Violation {
    /// Stable codes, distinct per condition so callers never have to disambiguate.
    pub fn code(&self) -> &'static str {
        match self {
            Violation::UnknownSchemaPrefix { .. } => "E-DET-009",
            Violation::UnknownSchemaMajor { .. } => "E-DET-010",
            Violation::NonPositiveBandwidth { .. } => "E-DET-011",
            Violation::SaturatedBelowSingleThread { .. } => "E-DET-012",
            Violation::NonFiniteSingleThread { .. } => "E-DET-013",
            Violation::ThreadSweepHadNoEffect { .. } => "E-DET-014",
            Violation::UnstableBandwidth { .. } => "E-DET-002",
            Violation::HeadlineNotCorroborated { .. } => "E-DET-015",
            Violation::MissingHeadlineMethod => "E-DET-017",
            Violation::InvalidHeadlineMethod { .. } => "E-DET-017",
            Violation::HeadlineMethodWithoutSample { .. } => "E-DET-018",
            Violation::PressuredMachineAssertedMeasured { .. } => "E-DET-003",
            Violation::PressureUnknownAssertedMeasured => "E-DET-016",
            Violation::MissingProvenance => "E-DET-004",
            Violation::InvalidProvenance { .. } => "E-DET-005",
            Violation::NoSamplesForMeasured => "E-DET-006",
            Violation::BufferTooSmall { .. } => "E-DET-007",
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::UnknownSchemaPrefix { found } => {
                write!(f, "schema {found} is not a qinfer/cfp document")
            }
            Violation::UnknownSchemaMajor { found, supported } => write!(
                f,
                "fingerprint schema {found} is newer than this build (supported major {supported})"
            ),
            Violation::NonPositiveBandwidth { value } => {
                write!(f, "headline bandwidth must be positive and finite, got {value}")
            }
            Violation::NonFiniteSingleThread { value } => {
                write!(f, "single-thread bandwidth must be positive and finite, got {value}")
            }
            Violation::SaturatedBelowSingleThread { single_gbs, saturated_gbs } => write!(
                f,
                "saturated bandwidth ({saturated_gbs} GB/s) is below single-thread ({single_gbs} GB/s)"
            ),
            Violation::ThreadSweepHadNoEffect { gain, best_threads, floor } => write!(
                f,
                "{best_threads} threads produced only {gain:.2}x single-thread bandwidth (floor {floor:.2}x) — the sweep had no effect, so the saturated figure cannot be trusted"
            ),
            Violation::UnstableBandwidth { sample_method, spread_pct, threshold } => write!(
                f,
                "{sample_method} bandwidth spread {spread_pct:.1}% exceeds {threshold}% but provenance claims a measurement"
            ),
            Violation::HeadlineNotCorroborated { headline_gbs, expected_gbs, source } => write!(
                f,
                "headline bandwidth {headline_gbs} GB/s disagrees with {source} ({expected_gbs} GB/s) — the number the predictor uses is not backed by any measurement"
            ),
            Violation::MissingHeadlineMethod => write!(
                f,
                "no headline_method — without knowing which access pattern produced the headline, no ordering or corroboration check can run"
            ),
            Violation::InvalidHeadlineMethod { value } => write!(
                f,
                "headline_method {value:?} is not one of [\"copy\", \"triad\"]"
            ),
            Violation::HeadlineMethodWithoutSample { method } => write!(
                f,
                "headline_method is {method:?} but no {method} sample is present to substantiate it"
            ),
            Violation::PressuredMachineAssertedMeasured { swap_free_pct, swap_gib, total_gib } => match swap_free_pct {
                Some(pct) => write!(
                    f,
                    "swap is {:.0}% full ({swap_gib:.1} GiB used of {total_gib:.1} GiB RAM) but bandwidth provenance claims a measurement",
                    100.0 - pct
                ),
                None => write!(
                    f,
                    "swap is exhausted ({swap_gib:.1} GiB used of {total_gib:.1} GiB RAM) but bandwidth provenance claims a measurement"
                ),
            },
            Violation::PressureUnknownAssertedMeasured => write!(
                f,
                "memory pressure cannot be determined from this fingerprint, so bandwidth may not claim to be `measured`"
            ),
            Violation::MissingProvenance => write!(
                f,
                "no provenance for bandwidth — every measurement-bearing field must declare its source"
            ),
            Violation::InvalidProvenance { value } => write!(
                f,
                "provenance.bandwidth = {value:?} is not one of {KNOWN_PROVENANCE:?}"
            ),
            Violation::NoSamplesForMeasured => write!(
                f,
                "bandwidth claims to be measured but carries no sample to substantiate it"
            ),
            Violation::BufferTooSmall { sample_method, buffer_bytes, required_bytes } => write!(
                f,
                "{sample_method} probe used {buffer_bytes} B, below the 4x LLC floor ({required_bytes} B) — it measured cache, not DRAM"
            ),
        }
    }
}

/// `qinfer/cfp@1` -> Some(1). Tolerates a trailing minor/patch.
pub fn parse_major(schema: &str) -> Option<u32> {
    let version = schema.strip_prefix(SCHEMA_PREFIX)?;
    version.split('.').next()?.parse().ok()
}

impl Fingerprint {
    /// Returns every violation, not just the first — a fingerprint is usually
    /// wrong in more than one way, and fixing them one report at a time is cruel.
    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        let mut v = Vec::new();

        if !self.schema.starts_with(SCHEMA_PREFIX) {
            v.push(Violation::UnknownSchemaPrefix { found: self.schema.clone() });
        } else if parse_major(&self.schema).map_or(true, |m| m > SUPPORTED_MAJOR) {
            v.push(Violation::UnknownSchemaMajor {
                found: self.schema.clone(),
                supported: SUPPORTED_MAJOR,
            });
        }

        let saturated = self.bandwidth.headline_gbs;
        if !saturated.is_finite() || saturated <= 0.0 {
            v.push(Violation::NonPositiveBandwidth { value: saturated });
        }

        let single = self.bandwidth.single_thread_gbs;
        if !single.is_finite() || single <= 0.0 {
            v.push(Violation::NonFiniteSingleThread { value: single });
        }

        // Fail closed on the method (Critic round 3, F1). An absent or
        // unrecognised method previously switched off *every* ordering and
        // corroboration check, which made a stricter validator more permissive
        // than its predecessor.
        let method_raw = self.bandwidth.headline_method.clone();
        let method_norm = method_raw.as_deref().map(|m| m.trim().to_ascii_lowercase());
        let recognised: Option<&str> = match method_norm.as_deref() {
            Some("copy") => Some("copy"),
            Some("triad") => Some("triad"),
            Some(_) => {
                v.push(Violation::InvalidHeadlineMethod {
                    value: method_raw.clone().unwrap_or_default(),
                });
                None
            }
            None => {
                v.push(Violation::MissingHeadlineMethod);
                None
            }
        };

        // Like-for-like only: `triad` is systematically slower than `copy`.
        let comparable = recognised == Some("copy");
        // `single > 0.0` matters: a negative or NaN single-thread value already
        // has its own violation, and must not also produce a bogus gain check.
        if comparable && single.is_finite() && single > 0.0 && saturated.is_finite() {
            if saturated < single {
                v.push(Violation::SaturatedBelowSingleThread {
                    single_gbs: single,
                    saturated_gbs: saturated,
                });
            } else if let Some(best) = self.bandwidth.best_threads {
                // Gate on sweep *width*, not on where the producer says the knee
                // was: a 15-core machine reporting a flat 2-thread sweep used to
                // slip through (Critic round 3, N6).
                let sweep_width = best.max(self.bandwidth.thread_sweep.len() as u32);
                if sweep_width >= MIN_SWEEP_THREADS {
                    let gain = saturated / single;
                    if gain < MIN_THREAD_GAIN {
                        v.push(Violation::ThreadSweepHadNoEffect {
                            gain,
                            best_threads: best,
                            floor: MIN_THREAD_GAIN,
                        });
                    }
                }
            }
        }

        // N1: the headline is what the predictor consumes (ADR-0003). It must be
        // corroborated against evidence produced by the *same* access pattern —
        // the sweep is copy-derived, so it only corroborates a copy headline.
        let named = match recognised {
            Some("copy") => self.bandwidth.copy.as_ref(),
            Some("triad") => self.bandwidth.triad.as_ref(),
            _ => None,
        };
        if recognised.is_some() && named.is_none() {
            v.push(Violation::HeadlineMethodWithoutSample {
                method: recognised.unwrap_or("unknown").to_string(),
            });
        }

        let corroboration: Option<(f64, String)> = match recognised {
            Some("copy") => self
                .bandwidth
                .thread_sweep
                .values()
                .cloned()
                .filter(|v| v.is_finite() && *v > 0.0)
                .fold(None, |acc: Option<f64>, x| Some(acc.map_or(x, |a| a.max(x))))
                .map(|m| (m, "thread_sweep".to_string()))
                .or_else(|| self.bandwidth.copy.as_ref().map(|s| (s.best_gbs, "copy".to_string()))),
            Some("triad") => self.bandwidth.triad.as_ref().map(|s| (s.best_gbs, "triad".to_string())),
            _ => None,
        };

        if let Some((expected, source)) = corroboration {
            if expected.is_finite() && expected > 0.0 && saturated.is_finite() {
                let delta = (saturated - expected).abs() / expected;
                if delta > 0.01 {
                    v.push(Violation::HeadlineNotCorroborated {
                        headline_gbs: saturated,
                        expected_gbs: expected,
                        source,
                    });
                }
            }
        }

        // Provenance fails closed: absent or unrecognised is a violation, not a pass.
        let provenance = self.provenance.get("bandwidth").map(|s| s.trim().to_ascii_lowercase());
        let measured = match provenance.as_deref() {
            None => {
                v.push(Violation::MissingProvenance);
                false
            }
            Some(p) if KNOWN_PROVENANCE.contains(&p) => p == "measured",
            Some(other) => {
                v.push(Violation::InvalidProvenance { value: other.to_string() });
                false
            }
        };

        let samples: Vec<&crate::Sample> =
            [self.bandwidth.copy.as_ref(), self.bandwidth.triad.as_ref()].into_iter().flatten().collect();

        if measured {
            if samples.is_empty() {
                v.push(Violation::NoSamplesForMeasured);
            }

            for s in &samples {
                // NaN compares false against everything, so test it explicitly.
                if !s.spread_pct.is_finite() || s.spread_pct > MAX_SPREAD_PCT {
                    v.push(Violation::UnstableBandwidth {
                        sample_method: s.method.clone(),
                        spread_pct: s.spread_pct,
                        threshold: MAX_SPREAD_PCT,
                    });
                }

                // PRD §20.2.1, now checkable because `Sample` keeps the buffer size.
                if let (Some(llc), Some(buf)) = (self.cpu.l3_bytes, s.buffer_bytes) {
                    let required = llc.saturating_mul(4);
                    if buf < required {
                        v.push(Violation::BufferTooSmall {
                            sample_method: s.method.clone(),
                            buffer_bytes: buf,
                            required_bytes: required,
                        });
                    }
                }
            }

            // N5: pressure that cannot be established is not "fine". A Linux
            // producer emitting an unparseable `swap_raw` used to sail through.
            match self.pressure() {
                crate::PressureState::Pressured { swap_free_pct } => {
                    v.push(Violation::PressuredMachineAssertedMeasured {
                        swap_free_pct,
                        swap_gib: self.swap_gib(),
                        total_gib: self.total_gib(),
                    });
                }
                crate::PressureState::Unknown => v.push(Violation::PressureUnknownAssertedMeasured),
                crate::PressureState::Relaxed { .. } => {}
            }
        }

        if v.is_empty() { Ok(()) } else { Err(v) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bandwidth, Cpu, Gpu, Identity, Memory, Power, Sample};
    use std::collections::BTreeMap;

    fn sample(method: &str, spread: f64) -> Sample {
        Sample {
            method: method.into(),
            best_gbs: 228.0,
            median_gbs: 220.0,
            spread_pct: spread,
            reps: Some(7),
            buffer_bytes: Some(256 * 1024 * 1024),
        }
    }

    /// A machine that actually passes every rule — the baseline the negative
    /// tests mutate one field at a time.
    fn fixture() -> Fingerprint {
        let mut provenance = BTreeMap::new();
        provenance.insert("bandwidth".to_string(), "measured".to_string());

        Fingerprint {
            schema: "qinfer/cfp@1".into(),
            collected_at: "2026-10-26T00:00:00Z".into(),
            identity: Identity {
                os: "macos".into(),
                os_version: Some("27.0.1".into()),
                arch: "arm64".into(),
                kernel: None,
                hostname: None,
            },
            cpu: Cpu {
                brand: "Apple M5 Pro".into(),
                physical_cores: 15,
                logical_cores: 15,
                perf_levels: vec![],
                isa: BTreeMap::new(),
                l3_bytes: None,
                max_freq_mhz: None,
            },
            gpu: Gpu { unified_memory: true, ..Gpu::default() },
            memory: Memory {
                total_bytes: 24 << 30,
                swap_used_bytes: Some(1024 * 1024 * 1024),
                // 8 GiB free of 8 GiB => healthy headroom
                swap_raw: Some("total = 8192.00M  used = 0.00M  free = 8192.00M".into()),
            },
            bandwidth: Bandwidth {
                copy: Some(sample("copy", 3.0)),
                triad: None,
                single_thread_gbs: 131.4,
                thread_sweep: BTreeMap::new(),
                best_threads: Some(15),
                headline_gbs: 228.0,
                headline_method: Some("copy".into()),
                buffer_mib: Some(256),
            },
            power: Power::default(),
            provenance,
        }
    }

    fn codes(fp: &Fingerprint) -> Vec<&'static str> {
        fp.validate().unwrap_err().iter().map(|v| v.code()).collect()
    }

    #[test]
    fn accepts_a_healthy_machine() {
        assert_eq!(fixture().validate(), Ok(()));
    }

    #[test]
    fn parses_only_the_cfp_prefix() {
        assert_eq!(parse_major("qinfer/cfp@1"), Some(1));
        assert_eq!(parse_major("qinfer/cfp@1.4.2"), Some(1));
        assert_eq!(parse_major("qinfer/cfp@2"), Some(2));
        assert_eq!(parse_major("qinfer/cfp-spike@1"), None);
        assert_eq!(parse_major("nonsense"), None);
    }

    #[test]
    fn rejects_a_foreign_schema_prefix() {
        let mut fp = fixture();
        fp.schema = "qinfer/cfp-spike@1".into();
        assert_eq!(codes(&fp), vec!["E-DET-009"]);
    }

    #[test]
    fn rejects_a_newer_schema_major() {
        let mut fp = fixture();
        fp.schema = "qinfer/cfp@2".into();
        assert_eq!(codes(&fp), vec!["E-DET-010"]);
    }

    #[test]
    fn rejects_saturated_below_single_thread() {
        let mut fp = fixture();
        fp.bandwidth.single_thread_gbs = 300.0;
        assert_eq!(codes(&fp), vec!["E-DET-012"]);
    }

    #[test]
    fn rejects_a_sweep_that_had_no_effect() {
        let mut fp = fixture();
        fp.bandwidth.headline_gbs = 228.0;
        fp.bandwidth.single_thread_gbs = 227.0;
        assert_eq!(codes(&fp), vec!["E-DET-014"]);
    }

    #[test]
    fn skips_ordering_checks_when_headline_comes_from_triad() {
        let mut fp = fixture();
        fp.bandwidth.headline_method = Some("triad".into());
        fp.bandwidth.headline_gbs = 120.1;
        fp.bandwidth.single_thread_gbs = 131.4;
        // A triad headline needs a triad sample to corroborate against.
        fp.bandwidth.triad = Some(Sample {
            method: "triad".into(),
            best_gbs: 120.1,
            median_gbs: 118.0,
            spread_pct: 4.0,
            reps: Some(7),
            buffer_bytes: Some(256 * 1024 * 1024),
        });
        assert_eq!(fp.validate(), Ok(()), "cross-method comparison must not fire");
    }

    #[test]
    fn accepts_a_single_core_sweep() {
        let mut fp = fixture();
        fp.bandwidth.best_threads = Some(1);
        fp.bandwidth.headline_gbs = 131.4;
        fp.bandwidth.single_thread_gbs = 131.4;
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.best_gbs = 131.4; // corroboration must still hold
        }
        assert_eq!(fp.validate(), Ok(()), "one thread cannot beat itself");
    }

    #[test]
    fn rejects_unstable_copy_bandwidth_claimed_as_measured() {
        let mut fp = fixture();
        fp.bandwidth.copy = Some(sample("copy", 77.81));
        assert_eq!(codes(&fp), vec!["E-DET-002"]);
    }

    #[test]
    fn rejects_unstable_triad_bandwidth_too() {
        let mut fp = fixture();
        fp.bandwidth.triad = Some(sample("triad", 30.28));
        assert_eq!(codes(&fp), vec!["E-DET-002"]);
    }

    #[test]
    fn rejects_nan_spread() {
        let mut fp = fixture();
        fp.bandwidth.copy = Some(sample("copy", f64::NAN));
        assert_eq!(codes(&fp), vec!["E-DET-002"]);
    }

    #[test]
    fn rejects_nan_single_thread() {
        let mut fp = fixture();
        fp.bandwidth.single_thread_gbs = f64::NAN;
        assert_eq!(codes(&fp), vec!["E-DET-013"]);
    }

    #[test]
    fn rejects_negative_single_thread() {
        let mut fp = fixture();
        fp.bandwidth.single_thread_gbs = -5.0;
        assert_eq!(codes(&fp), vec!["E-DET-013"]);
    }

    #[test]
    fn tolerant_of_unstable_bandwidth_when_provenance_is_honest() {
        let mut fp = fixture();
        fp.bandwidth.copy = Some(sample("copy", 77.81));
        fp.provenance.insert("bandwidth".into(), "unknown".into());
        assert_eq!(fp.validate(), Ok(()), "an honest 'unknown' must not be a violation");
    }

    #[test]
    fn rejects_missing_provenance() {
        let mut fp = fixture();
        fp.provenance.clear();
        assert_eq!(codes(&fp), vec!["E-DET-004"]);
    }

    #[test]
    fn rejects_unrecognised_provenance_rather_than_failing_open() {
        let mut fp = fixture();
        fp.provenance.insert("bandwidth".into(), "measured by hand".into());
        assert_eq!(codes(&fp), vec!["E-DET-005"]);
    }

    #[test]
    fn rejects_a_measurement_with_no_samples() {
        let mut fp = fixture();
        fp.bandwidth.copy = None;
        fp.bandwidth.triad = None;
        let c = codes(&fp);
        assert!(c.contains(&"E-DET-006"), "expected no-samples, got {c:?}");
        // A declared method with no matching sample is its own contradiction.
        assert!(c.contains(&"E-DET-018"), "expected method-without-sample, got {c:?}");
    }

    #[test]
    fn rejects_a_buffer_below_the_llc_floor() {
        let mut fp = fixture();
        fp.cpu.l3_bytes = Some(64 * 1024 * 1024); // 4x = 256 MiB required
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.buffer_bytes = Some(8 * 1024 * 1024);
        }
        assert_eq!(codes(&fp), vec!["E-DET-007"]);
    }

    #[test]
    fn rejects_a_machine_that_has_exhausted_swap() {
        let mut fp = fixture();
        // The reference machine's real numbers: 24 GiB RAM, swap 96.3% full.
        fp.memory.swap_used_bytes = Some(12_399_285_370);
        fp.memory.swap_raw =
            Some("total = 12288.00M  used = 11824.88M  free = 463.12M".into());
        assert_eq!(codes(&fp), vec!["E-DET-003"]);
    }

    #[test]
    fn accepts_a_machine_with_swap_headroom_even_if_swap_is_large() {
        let mut fp = fixture();
        fp.memory.swap_used_bytes = Some(20 << 30); // 20 GiB "used"
        fp.memory.swap_raw =
            Some("total = 12288.00M  used = 0.00M  free = 12288.00M".into());
        assert_eq!(fp.validate(), Ok(()), "headroom is the signal, not swap-used");
    }

    #[test]
    fn reports_every_violation_not_just_the_first() {
        let mut fp = fixture();
        fp.schema = "qinfer/cfp@9".into();
        fp.bandwidth.single_thread_gbs = f64::NAN;
        fp.provenance.clear();
        let err = fp.validate().unwrap_err();
        assert_eq!(err.len(), 3, "expected exactly three, got {err:?}");
    }

    /// The regression test the Critic asked for: the one real fingerprint in the
    /// repository is NOT trustworthy, and the type must say so.
    #[test]
    fn rejects_the_committed_reference_fingerprint() {
        let raw = include_str!("../../../spikes/m0/fingerprint.json");
        let fp: Fingerprint = serde_json::from_str(raw).expect("fixture must parse");
        let err = fp.validate().expect_err("the reference fingerprint is known-bad");
        let codes: Vec<&str> = err.iter().map(|v| v.code()).collect();
        assert!(codes.contains(&"E-DET-002"), "expected unstable-bandwidth, got {codes:?}");
        assert!(codes.contains(&"E-DET-003"), "expected swap pressure, got {codes:?}");
        assert!(codes.contains(&"E-DET-009"), "expected schema prefix, got {codes:?}");
    }

    #[test]
    fn swap_headroom_is_parsed_from_the_real_string() {
        let mut fp = fixture();
        fp.memory.swap_raw = Some("total = 12288.00M  used = 11824.88M  free = 463.12M".into());
        let h = fp.swap_headroom().expect("headroom");
        assert!((h - 463.12 / 12288.0).abs() < 1e-6, "got {h}");
        assert!(fp.memory_pressured());
    }

    // --- findings from Critic round 2 -------------------------------------

    /// N1: the headline is what the predictor consumes. It must be corroborated.
    #[test]
    fn rejects_a_headline_that_contradicts_its_measurements() {
        let mut fp = fixture();
        fp.bandwidth.headline_gbs = 999.0; // copy sample says 228.0
        assert_eq!(codes(&fp), vec!["E-DET-015"]);
    }

    #[test]
    fn corroborates_a_copy_headline_against_the_thread_sweep() {
        let mut fp = fixture();
        fp.bandwidth.thread_sweep.insert("15".into(), 300.0); // headline says 228
        let c = codes(&fp);
        assert!(c.contains(&"E-DET-015"), "sweep comparison must fire, got {c:?}");
    }

    /// N5: a Linux-style swap string must not read as "no pressure".
    #[test]
    fn rejects_measured_when_pressure_cannot_be_determined() {
        let mut fp = fixture();
        fp.memory.swap_raw = Some("SwapTotal: 8388604 kB".into()); // Linux format
        fp.memory.swap_used_bytes = None;
        assert_eq!(codes(&fp), vec!["E-DET-016"]);
    }

    #[test]
    fn rejects_an_unrecognised_headline_method_rather_than_skipping_the_checks() {
        let mut fp = fixture();
        fp.bandwidth.headline_method = Some("Copy (best of 7)".into());
        fp.bandwidth.single_thread_gbs = 500.0;
        fp.bandwidth.headline_gbs = 100.0;
        // Fail closed: an unrecognised method used to switch every check off.
        assert_eq!(codes(&fp), vec!["E-DET-017"]);
    }

    #[test]
    fn rejects_a_missing_headline_method() {
        let mut fp = fixture();
        fp.bandwidth.headline_method = None;
        assert_eq!(codes(&fp), vec!["E-DET-017"]);
    }

    /// N6: a flat 2-thread sweep is noise, not a defect.
    #[test]
    fn tolerant_of_a_two_thread_sweep_that_adds_nothing() {
        let mut fp = fixture();
        fp.bandwidth.best_threads = Some(2);
        fp.bandwidth.single_thread_gbs = 131.42;
        fp.bandwidth.headline_gbs = 131.50;
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.best_gbs = 131.50;
        }
        assert_eq!(fp.validate(), Ok(()));
    }

    #[test]
    fn four_threads_that_add_nothing_are_still_a_defect() {
        let mut fp = fixture();
        fp.bandwidth.best_threads = Some(4);
        fp.bandwidth.single_thread_gbs = 131.42;
        fp.bandwidth.headline_gbs = 131.50;
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.best_gbs = 131.50;
        }
        assert_eq!(codes(&fp), vec!["E-DET-014"]);
    }

    // --- the trust token itself (Critic N3) -------------------------------

    #[test]
    fn validated_try_from_rejects_an_invalid_fingerprint() {
        let mut fp = fixture();
        fp.bandwidth.copy = Some(sample("copy", 77.81));
        assert!(crate::Validated::try_from(fp).is_err());
    }

    #[test]
    fn validated_try_from_accepts_a_valid_fingerprint() {
        assert!(crate::Validated::try_from(fixture()).is_ok());
    }

    #[test]
    fn validated_deserialize_rejects_an_invalid_document() {
        let mut fp = fixture();
        fp.bandwidth.copy = Some(sample("copy", 77.81));
        let json = serde_json::to_string(&serde_json::json!({
            "schema": fp.schema, "identity": {"os":"macos","arch":"arm64"},
            "cpu": {"brand":"x","physical_cores":1,"logical_cores":1},
            "gpu": {}, "memory": {"total_bytes": 1024},
            "bandwidth": {
                "copy": {"method":"copy","best_gbs":228.0,"median_gbs":220.0,"spread_pct":77.81},
                "single_thread_gbs": 131.4, "headline_gbs": 228.0,
                "headline_method": "copy", "thread_sweep": {"15": 228.0}, "best_threads": 15
            },
            "provenance": {"bandwidth": "measured"}
        }))
        .unwrap();
        assert!(serde_json::from_str::<crate::Validated<crate::Fingerprint>>(&json).is_err());
    }

    /// AC 7: the field is required, so a document without it cannot even parse.
    #[test]
    fn a_document_without_single_thread_bandwidth_fails_to_deserialize() {
        let raw = include_str!("../../../spikes/m0/fingerprint.json");
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        v["bandwidth"].as_object_mut().unwrap().remove("single_thread_gbs");
        assert!(serde_json::from_str::<Fingerprint>(&v.to_string()).is_err());
    }

    /// N12: a zero-RAM document must be a violation, not a shrug.
    #[test]
    fn a_zero_ram_fingerprint_cannot_claim_a_measurement() {
        let mut fp = fixture();
        fp.memory.total_bytes = 0;
        fp.memory.swap_raw = None;
        fp.memory.swap_used_bytes = None;
        assert_eq!(codes(&fp), vec!["E-DET-016"]);
    }
}
