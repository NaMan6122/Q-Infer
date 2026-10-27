//! Micro-operations: what this silicon actually does (T-013).
//!
//! The predictor needs three numbers it cannot read from a spec sheet: the
//! bandwidth the **device** achieves (not a CPU memcpy's opinion of it), the
//! throughput of the **compute-bound** phase, and whether the neural-engine path
//! is reachable at all. This module produces them.
//!
//! Measured on the reference machine (24 GiB M5 Pro, `applegpu_g17s`):
//!
//! | op | result | note |
//! |---|---|---|
//! | GPU bandwidth (`a + b`, 256 MiB fp32) | **255.6 GB/s** | matches the saturated CPU probe at 256.9 |
//! | GEMV q4, 144 MiB of weights | 208.8 GB/s | still cache-warming |
//! | GEMV q4, 288 MiB of weights | **250.3 GB/s** | saturated |
//! | GEMV q4, 9 MiB of weights | 51.9 GB/s | **5x low** — it is measuring cache |
//! | GEMM bf16, 1024x4096x4096 | **22.34 TFLOPs** | the compute-bound phase |
//!
//! That GEMV row is the whole reason this module exists rather than a constant:
//! the same cache-residency error that made the original single-threaded probe
//! read 131 GB/s instead of 228 silently applies to every micro-benchmark, and
//! a matrix small enough to fit in cache reports a plausible, wrong number.
//!
//! Two rules, enforced in types rather than documented:
//!
//! 1. **A failed measurement is `Refused`, never `0.0`.** A zero bandwidth once
//!    produced a published 0.48 tok/s "prediction" (verification finding 4).
//! 2. **Absence is not zero.** An op the probe did not report is refused with a
//!    reason, so the dashboard can print `—` and the planner can decline.
//!
//! The numeric work happens in one `python3` subprocess using MLX, because MLX
//! is the only routing that reaches the M5 GPU's neural accelerators (Metal 4
//! TensorOps). This crate never links it — an engine is a subprocess (ADR-0001).

use crate::collect::Runner;
use crate::engines::Argv;
use crate::space::DeviceInfo;

/// The micro-operations we know how to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpId {
    /// Device-side memory bandwidth. Replaces the `k_cal > 1` hypothesis with a
    /// measurement, closing PRD Q11.
    GpuBandwidth,
    /// Quantised matrix-vector product — the decode shape. Must be sized above
    /// last-level cache or it measures cache.
    GemvQ4,
    /// Batched matrix multiply in bf16 — the prefill/TTFT shape, and the direct
    /// test of whether the M5 neural accelerators are engaged.
    GemmBf16,
    /// Is the Neural Engine reachable for this workload at all?
    AneReachable,
}

impl OpId {
    pub const ALL: [OpId; 4] =
        [OpId::GpuBandwidth, OpId::GemvQ4, OpId::GemmBf16, OpId::AneReachable];

    /// Stable wire name. Used as the key in the probe's JSON and in receipts.
    pub fn key(self) -> &'static str {
        match self {
            OpId::GpuBandwidth => "gpu_bandwidth_gbs",
            OpId::GemvQ4 => "gemv_q4_gbs",
            OpId::GemmBf16 => "gemm_bf16_tflops",
            OpId::AneReachable => "ane_reachable",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OpId::GpuBandwidth => "GPU bandwidth",
            OpId::GemvQ4 => "GEMV q4 (decode shape)",
            OpId::GemmBf16 => "GEMM bf16 (prefill shape)",
            OpId::AneReachable => "Neural Engine reachability",
        }
    }

    pub fn parse(s: &str) -> Option<OpId> {
        OpId::ALL.into_iter().find(|o| o.key() == s)
    }
}

/// The result of one op. `Refused` carries the fix, because "no" without "do
/// this instead" is the error style PRD §10.5 forbids.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Measured {
        value: f64,
        unit: &'static str,
        reps: u32,
        spread_pct: f64,
        method: String,
    },
    Refused {
        reason: String,
        fix: Option<String>,
    },
}

impl Outcome {
    pub fn value(&self) -> Option<f64> {
        match self {
            Outcome::Measured { value, .. } => Some(*value),
            Outcome::Refused { .. } => None,
        }
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, Outcome::Measured { .. })
    }

    /// A measurement whose spread exceeds the `cfp@1` bar is present but not
    /// trustworthy; surfaces must show it as such rather than hiding it.
    pub fn is_trustworthy(&self) -> bool {
        matches!(self, Outcome::Measured { spread_pct, value, .. }
            if *value > 0.0 && value.is_finite() && *spread_pct <= crate::MAX_SPREAD_PCT)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MicroOp {
    pub id: OpId,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub ops: Vec<MicroOp>,
    /// What the GPU said about itself, when the probe returned it. This is the
    /// input `space::resolve` needs for its ceiling.
    pub device: Option<DeviceInfo>,
}

impl Report {
    pub fn get(&self, id: OpId) -> Option<&Outcome> {
        self.ops.iter().find(|o| o.id == id).map(|o| &o.outcome)
    }

    pub fn measured(&self, id: OpId) -> Option<f64> {
        self.get(id).and_then(Outcome::value)
    }

    pub fn refusals(&self) -> Vec<(&'static str, &str)> {
        self.ops
            .iter()
            .filter_map(|o| match &o.outcome {
                Outcome::Refused { reason, .. } => Some((o.id.key(), reason.as_str())),
                Outcome::Measured { .. } => None,
            })
            .collect()
    }

    /// The bandwidth the device itself achieves. When present this is what
    /// `k_cal` should be measured against, because it removes the free
    /// parameter rather than absorbing it.
    pub fn gpu_bandwidth_gbs(&self) -> Option<f64> {
        self.measured(OpId::GpuBandwidth)
    }

    /// The compute-bound ceiling, in TFLOP/s, that TTFT divides into.
    pub fn prefill_tflops(&self) -> Option<f64> {
        self.measured(OpId::GemmBf16)
    }
}

/// The probe program. Kept as one string so the whole measurement can be
/// inspected, versioned and diffed; it is the only Python in the crate and it
/// is executed, never imported.
///
/// It must print exactly one JSON object whose keys are [`OpId::key`] values for
/// the ops it was asked for, plus optionally a `device` object with MLX's
/// `device_info()` payload. Ops it could not run are omitted — the Rust side
/// turns omission into `Refused`, so the script never has to invent a zero.
pub const PROBE_SCRIPT: &str = include_str!("microops_probe.py");

/// The command that runs the probe against the requested ops.
pub fn argv(ops: &[OpId]) -> Argv {
    let keys: Vec<&str> = ops.iter().map(|o| o.key()).collect();
    Argv {
        program: "python3".into(),
        args: vec!["-c".into(), PROBE_SCRIPT.into(), keys.join(",")],
    }
}

/// Run the probe. A probe that cannot start is reported as every op refused —
/// never as a set of zeroes, and never as a panic.
pub fn run(runner: &impl Runner, ops: &[OpId]) -> Report {
    let a = argv(ops);
    let args: Vec<&str> = a.args.iter().map(String::as_str).collect();
    match runner.run(&a.program, &args) {
        Some(out) => parse(&out, ops),
        None => Report {
            ops: ops
                .iter()
                .map(|id| MicroOp {
                    id: *id,
                    outcome: Outcome::Refused {
                        reason: "the measurement probe did not run".into(),
                        fix: Some(format!("{} — check that python3 and mlx are installed", a.render())),
                    },
                })
                .collect(),
            device: None,
        },
    }
}

/// Parse the probe's output. Pure, so every refusal path is testable without a
/// GPU.
///
/// Three demotions, all of them load-bearing:
///   * malformed JSON            -> every op refused, never an empty success
///   * a key that is absent      -> refused for *that* op only (absence is not zero)
///   * a value <= 0 or not finite-> refused (a zero bandwidth is a failed probe,
///                                  and publishing from it once produced a
///                                  0.48 tok/s "prediction")
pub fn parse(json: &str, ops: &[OpId]) -> Report {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Report {
            ops: ops
                .iter()
                .map(|id| MicroOp { id: *id, outcome: malformed() })
                .collect(),
            device: None,
        };
    };

    let device = v
        .get("device")
        .and_then(|d| crate::space::macos::parse_device_info(&d.to_string()));

    let ops_out = ops
        .iter()
        .map(|id| MicroOp {
            id: *id,
            outcome: outcome_for(&v, *id),
        })
        .collect();

    Report { ops: ops_out, device }
}

fn malformed() -> Outcome {
    Outcome::Refused {
        reason: "the measurement probe produced output that is not JSON".into(),
        fix: Some("run the probe by hand: python3 -c \"$(cat src/microops_probe.py)\" gpu_bandwidth_gbs".into()),
    }
}

fn outcome_for(root: &serde_json::Value, id: OpId) -> Outcome {
    let Some(entry) = root.get(id.key()) else {
        // The probe omits an op it could not perform rather than emitting a
        // zero, so absence is the normal encoding of "not available here" and
        // each op deserves its own sentence rather than a generic one.
        let reason = match id {
            OpId::AneReachable => {
                "the Neural Engine is not reachable from this toolchain — expected on Apple silicon, \
                 where neither MLX nor llama.cpp routes LLM decode through the ANE"
                    .to_string()
            }
            _ => format!("{} was not reported by the probe", id.label()),
        };
        return Outcome::Refused { reason, fix: Some(refusal_fix(id)) };
    };

    let Some(value) = entry.get("value").and_then(|x| x.as_f64()) else {
        return Outcome::Refused {
            reason: format!("{} was reported without a numeric value", id.label()),
            fix: None,
        };
    };

    // A measurement of zero is not a measurement. This is the rule that the
    // older `predict` code learned the hard way; it is enforced here at the
    // boundary so no caller has to remember it.
    if !value.is_finite() || value <= 0.0 {
        return Outcome::Refused {
            reason: format!("{} reported {value}, which is not a measurement", id.label()),
            fix: Some(refusal_fix(id)),
        };
    }

    Outcome::Measured {
        value,
        unit: match entry.get("unit").and_then(|x| x.as_str()) {
            Some("GB/s") => "GB/s",
            Some("TFLOP/s") => "TFLOP/s",
            Some("bool") => "bool",
            _ => "unknown",
        },
        reps: entry.get("reps").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        spread_pct: entry.get("spread_pct").and_then(|x| x.as_f64()).unwrap_or(0.0),
        method: entry
            .get("method")
            .and_then(|x| x.as_str())
            .unwrap_or("unspecified")
            .to_string(),
    }
}

fn refusal_fix(id: OpId) -> String {
    match id {
        OpId::GpuBandwidth | OpId::GemvQ4 | OpId::GemmBf16 => {
            "install MLX for the interpreter that runs the probe: `pip install mlx` or `uv pip install mlx`".into()
        }
        OpId::AneReachable => {
            "nothing to install: this is a platform limit, recorded so the planner cannot offer an ANE plan".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_keys_round_trip() {
        for id in OpId::ALL {
            assert_eq!(OpId::parse(id.key()), Some(id));
        }
        assert_eq!(OpId::parse("nonsense"), None);
    }

    #[test]
    fn op_keys_are_unique() {
        let mut keys: Vec<&str> = OpId::ALL.iter().map(|o| o.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), OpId::ALL.len());
    }

    #[test]
    fn argv_names_only_the_requested_ops() {
        let a = argv(&[OpId::GpuBandwidth, OpId::GemmBf16]);
        assert_eq!(a.program, "python3");
        assert_eq!(a.args.last().map(String::as_str), Some("gpu_bandwidth_gbs,gemm_bf16_tflops"));
    }

    /// Captured verbatim from the reference machine (24 GiB M5 Pro).
    const LIVE: &str = r#"{
      "device": {
        "architecture": "applegpu_g17s",
        "device_name": "Apple M5 Pro",
        "max_buffer_length": 14302248960,
        "max_recommended_working_set_size": 19069665280,
        "memory_size": 25769803776
      },
      "gpu_bandwidth_gbs": {"value": 255.6, "unit": "GB/s", "reps": 5, "spread_pct": 2.1, "method": "add_fp32@256MiB"},
      "gemv_q4_gbs": {"value": 250.3, "unit": "GB/s", "reps": 5, "spread_pct": 3.4, "method": "quantized_matmul_q4@16384x32768"},
      "gemm_bf16_tflops": {"value": 22.34, "unit": "TFLOP/s", "reps": 5, "spread_pct": 1.8, "method": "matmul_bf16@1024x4096x4096"},
      "ane_reachable": {"value": 0.0, "unit": "bool", "reps": 1, "spread_pct": 0.0, "method": "import coremltools failed: ModuleNotFoundError"}
    }"#;

    #[test]
    fn a_live_payload_yields_the_measured_figures() {
        let r = parse(LIVE, &OpId::ALL);
        assert_eq!(r.gpu_bandwidth_gbs(), Some(255.6));
        assert_eq!(r.prefill_tflops(), Some(22.34));
        assert_eq!(r.measured(OpId::GemvQ4), Some(250.3));
        match r.get(OpId::GpuBandwidth).expect("gpu bandwidth") {
            Outcome::Measured { unit, reps, method, .. } => {
                assert_eq!(*unit, "GB/s");
                assert_eq!(*reps, 5);
                assert!(method.contains("add_fp32"), "method must name the shape: {method}");
            }
            other => panic!("expected measured, got {other:?}"),
        }
    }

    #[test]
    fn the_device_block_feeds_the_memory_budget() {
        let r = parse(LIVE, &OpId::ALL);
        let d = r.device.expect("device info");
        assert_eq!(d.recommended_working_set, Some(19_069_665_280));
        assert_eq!(d.architecture.as_deref(), Some("applegpu_g17s"));
    }

    /// The ANE reports `0.0` because it is genuinely unreachable, not because it
    /// is fast. A zero must never be published as a measurement.
    #[test]
    fn a_zero_is_refused_rather_than_published() {
        let r = parse(LIVE, &OpId::ALL);
        let ane = r.get(OpId::AneReachable).expect("ane");
        assert!(!ane.is_measured());
        assert!(!ane.is_trustworthy());
        match ane {
            Outcome::Refused { reason, .. } => assert!(reason.contains("not a measurement")),
            other => panic!("expected refused, got {other:?}"),
        }
    }

    #[test]
    fn an_absent_key_refuses_only_that_op() {
        let partial = r#"{"gpu_bandwidth_gbs": {"value": 255.6, "unit": "GB/s", "reps": 5, "spread_pct": 2.1, "method": "x"}}"#;
        let r = parse(partial, &[OpId::GpuBandwidth, OpId::GemmBf16]);
        assert_eq!(r.measured(OpId::GpuBandwidth), Some(255.6));
        assert!(r.get(OpId::GemmBf16).unwrap().value().is_none(), "absence is not zero");
        let refusals = r.refusals();
        assert_eq!(refusals.len(), 1);
        assert_eq!(refusals[0].0, "gemm_bf16_tflops");
    }

    #[test]
    fn malformed_output_refuses_every_op() {
        let r = parse("Traceback (most recent call last):\nModuleNotFoundError: mlx", &OpId::ALL);
        assert_eq!(r.ops.len(), OpId::ALL.len());
        assert!(r.ops.iter().all(|o| !o.outcome.is_measured()));
        assert_eq!(r.refusals().len(), OpId::ALL.len());
        assert!(r.device.is_none());
    }

    #[test]
    fn a_value_without_a_number_is_refused() {
        let r = parse(r#"{"gpu_bandwidth_gbs": {"unit": "GB/s"}}"#, &[OpId::GpuBandwidth]);
        assert!(r.get(OpId::GpuBandwidth).unwrap().value().is_none());
    }

    #[test]
    fn a_wide_spread_is_measured_but_not_trustworthy() {
        let j = r#"{"gpu_bandwidth_gbs": {"value": 257.0, "unit": "GB/s", "reps": 5, "spread_pct": 79.0, "method": "x"}}"#;
        let r = parse(j, &[OpId::GpuBandwidth]);
        let o = r.get(OpId::GpuBandwidth).unwrap();
        assert!(o.is_measured(), "the number exists");
        assert!(!o.is_trustworthy(), "but it is not evidence — this is the 79% case");
    }
}
