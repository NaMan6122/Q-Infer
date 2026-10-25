//! Validation for `qinfer/cfp@1`.
//!
//! The fingerprint is the only input to planning (ADR-0002). A corrupt one does
//! not fail loudly downstream — it silently produces plausible, wrong plans.
//! So the invariants live here and are enforced at load, not trusted by
//! convention.
//!
//! Two of these invariants exist because the M0 spike proved they matter:
//! a single-threaded bandwidth probe under-reports by 1.74x, and a machine in
//! deep swap reports numbers that look fine and are not (spike REPORT, F1).

use crate::Fingerprint;

/// Highest `cfp@N` major this build understands.
pub const SUPPORTED_MAJOR: u32 = 1;

/// Above this run-to-run spread, a bandwidth figure is not a measurement.
pub const MAX_SPREAD_PCT: f64 = 15.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Violation {
    UnknownSchemaMajor { found: String, supported: u32 },
    NonPositiveBandwidth { value: f64 },
    SaturatedBelowSingleThread { single_gbs: f64, saturated_gbs: f64 },
    UnstableBandwidthAssertedMeasured { spread_pct: f64, threshold: f64 },
    PressuredMachineAssertedMeasured { swap_gib: f64, total_gib: f64 },
}

impl Violation {
    /// Stable codes, so callers (CLI, TUI, tests) can match without stringly typing.
    pub fn code(&self) -> &'static str {
        match self {
            Violation::UnknownSchemaMajor { .. } => "E-DET-010",
            Violation::NonPositiveBandwidth { .. } => "E-DET-011",
            Violation::SaturatedBelowSingleThread { .. } => "E-DET-012",
            Violation::UnstableBandwidthAssertedMeasured { .. } => "E-DET-002",
            Violation::PressuredMachineAssertedMeasured { .. } => "E-DET-002",
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::UnknownSchemaMajor { found, supported } => write!(
                f,
                "fingerprint schema {found} is newer than this build (supported major {supported})"
            ),
            Violation::NonPositiveBandwidth { value } => {
                write!(f, "headline bandwidth must be positive, got {value}")
            }
            Violation::SaturatedBelowSingleThread { single_gbs, saturated_gbs } => write!(
                f,
                "saturated bandwidth ({saturated_gbs} GB/s) is below single-thread ({single_gbs} GB/s) — the thread sweep did not run"
            ),
            Violation::UnstableBandwidthAssertedMeasured { spread_pct, threshold } => write!(
                f,
                "bandwidth spread {spread_pct}% exceeds {threshold}% but provenance claims a measurement"
            ),
            Violation::PressuredMachineAssertedMeasured { swap_gib, total_gib } => write!(
                f,
                "machine is swapping ({swap_gib:.1} of {total_gib:.1} GiB) but bandwidth provenance claims a measurement"
            ),
        }
    }
}

/// `qinfer/cfp@1` -> Some(1). Tolerates a trailing minor/patch, rejects garbage.
pub fn parse_major(schema: &str) -> Option<u32> {
    let (_, version) = schema.rsplit_once('@')?;
    let major = version.split('.').next()?;
    major.parse().ok()
}

impl Fingerprint {
    /// Returns every violation, not just the first — a fingerprint is usually
    /// wrong in more than one way, and fixing them one report at a time is cruel.
    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        let mut v = Vec::new();

        match parse_major(&self.schema) {
            Some(m) if m <= SUPPORTED_MAJOR => {}
            _ => v.push(Violation::UnknownSchemaMajor {
                found: self.schema.clone(),
                supported: SUPPORTED_MAJOR,
            }),
        }

        let saturated = self.bandwidth.headline_gbs;
        if !(saturated > 0.0) || !saturated.is_finite() {
            v.push(Violation::NonPositiveBandwidth { value: saturated });
        }

        if let Some(single) = self.bandwidth.single_thread_gbs {
            if single > saturated {
                v.push(Violation::SaturatedBelowSingleThread {
                    single_gbs: single,
                    saturated_gbs: saturated,
                });
            }
        }

        let measured = self
            .provenance
            .get("bandwidth")
            .map(|s| s.eq_ignore_ascii_case("measured"))
            .unwrap_or(false);

        if measured {
            if let Some(sample) = self.bandwidth.copy.as_ref() {
                if sample.spread_pct > MAX_SPREAD_PCT {
                    v.push(Violation::UnstableBandwidthAssertedMeasured {
                        spread_pct: sample.spread_pct,
                        threshold: MAX_SPREAD_PCT,
                    });
                }
            }
            if self.memory_pressured() {
                v.push(Violation::PressuredMachineAssertedMeasured {
                    swap_gib: self.swap_gib(),
                    total_gib: self.total_gib(),
                });
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

    fn fixture() -> Fingerprint {
        let mut isa = BTreeMap::new();
        isa.insert("FEAT_DotProd".to_string(), 1);

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
                isa,
                l3_bytes: None,
                max_freq_mhz: None,
            },
            gpu: Gpu { unified_memory: true, ..Gpu::default() },
            memory: Memory {
                total_bytes: 24 << 30,
                swap_used_bytes: Some(1 << 30), // well under the pressure threshold
                swap_raw: None,
            },
            bandwidth: Bandwidth {
                copy: Some(Sample { method: "copy".into(), best_gbs: 228.0, median_gbs: 220.0, spread_pct: 3.0 }),
                triad: None,
                single_thread_gbs: Some(131.0),
                thread_sweep: BTreeMap::new(),
                best_threads: Some(15),
                headline_gbs: 228.0,
                buffer_mib: Some(256),
            },
            power: Power::default(),
            provenance,
        }
    }

    #[test]
    fn accepts_the_reference_machine() {
        assert!(fixture().validate().is_ok());
    }

    #[test]
    fn schema_major_is_parsed() {
        assert_eq!(parse_major("qinfer/cfp@1"), Some(1));
        assert_eq!(parse_major("qinfer/cfp@1.4.2"), Some(1));
        assert_eq!(parse_major("qinfer/cfp@2"), Some(2));
        assert_eq!(parse_major("nonsense"), None);
    }

    #[test]
    fn rejects_a_newer_schema_major() {
        let mut fp = fixture();
        fp.schema = "qinfer/cfp@2".into();
        let err = fp.validate().unwrap_err();
        assert!(err.iter().any(|v| matches!(v, Violation::UnknownSchemaMajor { .. })));
        assert_eq!(err[0].code(), "E-DET-010");
    }

    #[test]
    fn rejects_saturated_below_single_thread() {
        let mut fp = fixture();
        fp.bandwidth.single_thread_gbs = Some(300.0);
        fp.bandwidth.headline_gbs = 228.0;
        let err = fp.validate().unwrap_err();
        assert!(err.iter().any(|v| matches!(v, Violation::SaturatedBelowSingleThread { .. })));
    }

    #[test]
    fn rejects_unstable_bandwidth_claimed_as_measured() {
        let mut fp = fixture();
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.spread_pct = 77.0; // the real value from the first spike run
        }
        let err = fp.validate().unwrap_err();
        assert!(err.iter().any(|v| matches!(v, Violation::UnstableBandwidthAssertedMeasured { .. })));
    }

    #[test]
    fn tolerant_of_unstable_bandwidth_when_provenance_is_honest() {
        let mut fp = fixture();
        if let Some(c) = fp.bandwidth.copy.as_mut() {
            c.spread_pct = 77.0;
        }
        fp.provenance.insert("bandwidth".into(), "unknown".into());
        assert!(fp.validate().is_ok(), "an honest 'unknown' must not be a violation");
    }

    #[test]
    fn rejects_pressured_machine_claimed_as_measured() {
        let mut fp = fixture();
        fp.memory.swap_used_bytes = Some(20 << 30); // > 50% of 24 GiB
        let err = fp.validate().unwrap_err();
        assert!(err.iter().any(|v| matches!(v, Violation::PressuredMachineAssertedMeasured { .. })));
    }

    #[test]
    fn rejects_non_positive_bandwidth() {
        let mut fp = fixture();
        fp.bandwidth.headline_gbs = 0.0;
        let err = fp.validate().unwrap_err();
        assert!(err.iter().any(|v| matches!(v, Violation::NonPositiveBandwidth { .. })));
    }

    #[test]
    fn reports_every_violation_not_just_the_first() {
        let mut fp = fixture();
        fp.schema = "qinfer/cfp@9".into();
        fp.bandwidth.single_thread_gbs = Some(999.0);
        fp.memory.swap_used_bytes = Some(23 << 30);
        let err = fp.validate().unwrap_err();
        assert!(err.len() >= 3, "expected multiple violations, got {err:?}");
    }
}
