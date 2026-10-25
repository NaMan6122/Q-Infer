//! The two-parameter decoder model (ADR-0003, T-005).
//!
//! ```text
//! t_token ≈ t₀ + bytes_per_token / (k_cal × BW)
//! ```
//!
//! Two parameters, both learned from observations on the actual machine, and
//! both are load-bearing:
//!
//! - **`t₀`** is a fixed per-token cost (launch, sampling, routing). It is
//!   invisible on large dense models and dominant on small ones and on MoE. The
//!   M0 spike measured 1.22 ms for dense Metal against ~8.4 ms reported for MoE
//!   on Ultra-class parts — so `t₀` is keyed on model class, never global.
//! - **`k_cal`** is a *probe-relative calibration coefficient*, not an
//!   efficiency. It may exceed 1.0: a GPU can pull more bandwidth than a CPU
//!   memcpy demonstrates (measured 1.175 on Metal). It is never clamped.
//!
//! Two rules this module enforces rather than documents:
//!   1. A fit from fewer than three observations is **not** a fit. It is
//!      returned flagged `determined: false` and the caller must widen or
//!      refuse — a two-point fit is exact and proves nothing.
//!   2. Predictions are intervals. A single number pretends to a precision that
//!      the evidence does not support.

/// A fit needs at least this many points before it can claim to be one.
pub const MIN_OBSERVATIONS: usize = 3;

/// Relative half-width of the p10–p90 interval for a well-determined fit.
pub const CALIBRATED_SPREAD: f64 = 0.15;

/// Relative half-width when the fit is underdetermined or absent.
pub const UNCALIBRATED_SPREAD: f64 = 0.45;

/// q8_0 stores one fp16 scale per 32 elements: 32 × 0.5 B + 2 B = 18 B per
/// 32 elements = 0.5625 B/elem, i.e. 1.0625 B per element-pair. The commonly
/// quoted "half of fp16" is wrong by 6.25%, which matters at 128k context.
pub const Q8_0_BYTES_PER_ELEMENT: f64 = 1.0625;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeClass {
    Dense,
    Moe,
}


/// One measured (artifact, throughput) pair on a specific machine.
#[derive(Debug, Clone)]
pub struct Observation {
    /// Bytes read per token: the artifact size for dense models, or
    /// `active_params × bytes_per_param` for MoE.
    pub bytes_per_token: u64,
    pub measured_decode_tps: f64,
    pub shape: ShapeClass,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub shape: ShapeClass,
    pub t0_ms: f64,
    pub k_cal: f64,
    pub r2: f64,
    /// False when fewer than `MIN_OBSERVATIONS` points were supplied, or the
    /// fit is degenerate. Callers must not present an undetermined fit as fact.
    pub determined: bool,
    pub n: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
}

impl Interval {
    pub fn half_width_pct(&self) -> f64 {
        if self.p50 > 0.0 {
            (self.p90 - self.p10) / 2.0 / self.p50 * 100.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    pub decode_tps: Interval,
    pub ms_per_token: f64,
    pub determined: bool,
}

/// Ordinary least squares of `t` on `bytes`, giving both parameters at once:
/// the intercept is `t₀` and the slope is `1/(k_cal × BW)`.
pub fn fit(observations: &[Observation], bw_gbs: f64) -> Calibration {
    let shape = observations.first().map(|o| o.shape).unwrap_or(ShapeClass::Dense);
    let n = observations.len();

    let usable: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.measured_decode_tps.is_finite() && o.measured_decode_tps > 0.0)
        .collect();

    if usable.len() < MIN_OBSERVATIONS || bw_gbs <= 0.0 || !bw_gbs.is_finite() {
        return Calibration {
            shape,
            t0_ms: 0.0,
            k_cal: 1.0,
            r2: 0.0,
            determined: false,
            n,
        };
    }

    let xs: Vec<f64> = usable.iter().map(|o| o.bytes_per_token as f64).collect();
    let ys: Vec<f64> = usable.iter().map(|o| 1.0 / o.measured_decode_tps).collect();

    let mx = xs.iter().sum::<f64>() / xs.len() as f64;
    let my = ys.iter().sum::<f64>() / ys.len() as f64;

    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();

    if sxx <= 0.0 {
        return Calibration { shape, t0_ms: 0.0, k_cal: 1.0, r2: 0.0, determined: false, n };
    }

    let slope = sxy / sxx;
    let intercept = my - slope * mx;

    let k_cal = if slope > 0.0 { 1.0 / (slope * bw_gbs * 1e9) } else { f64::NAN };

    let ss_res: f64 = xs
        .iter()
        .zip(&ys)
        .map(|(x, y)| (y - (intercept + slope * x)).powi(2))
        .sum();
    let ss_tot: f64 = ys.iter().map(|y| (y - my).powi(2)).sum();
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 1.0 };

    let determined = k_cal.is_finite() && k_cal > 0.0 && intercept.is_finite();

    Calibration {
        shape,
        // A negative intercept is physically meaningless (a token cannot take
        // negative time); clamp to zero rather than report nonsense.
        t0_ms: (intercept * 1000.0).max(0.0),
        k_cal,
        r2,
        determined,
        n,
    }
}

/// Predict decode throughput for an artifact of `bytes_per_token`.
pub fn predict(cal: &Calibration, bytes_per_token: u64, bw_gbs: f64) -> Prediction {
    let bw = if bw_gbs > 0.0 && bw_gbs.is_finite() { bw_gbs } else { 1.0 };
    let k = if cal.determined && cal.k_cal.is_finite() && cal.k_cal > 0.0 {
        cal.k_cal // never clamped to 1.0 — see the module doc
    } else {
        1.0
    };

    let seconds = cal.t0_ms / 1000.0 + (bytes_per_token as f64) / (k * bw * 1e9);
    let p50 = if seconds > 0.0 { 1.0 / seconds } else { 0.0 };

    let spread = if cal.determined { CALIBRATED_SPREAD } else { UNCALIBRATED_SPREAD };

    Prediction {
        decode_tps: Interval {
            p10: round2(p50 * (1.0 - spread)),
            p50: round2(p50),
            p90: round2(p50 * (1.0 + spread)),
        },
        ms_per_token: round3(seconds * 1000.0),
        determined: cal.determined,
    }
}

/// Mean absolute percentage error of the model against held-out observations.
pub fn mape(cal: &Calibration, observations: &[Observation], bw_gbs: f64) -> f64 {
    let errs: Vec<f64> = observations
        .iter()
        .filter(|o| o.measured_decode_tps > 0.0)
        .map(|o| {
            let p = predict(cal, o.bytes_per_token, bw_gbs).decode_tps.p50;
            (p - o.measured_decode_tps).abs() / o.measured_decode_tps
        })
        .collect();
    if errs.is_empty() {
        return f64::NAN;
    }
    errs.iter().sum::<f64>() / errs.len() as f64 * 100.0
}

// ---------------------------------------------------------------------------
// KV cache geometry
// ---------------------------------------------------------------------------

/// The fields of a model that decide how much KV state a context costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvGeometry {
    pub layers: u32,
    pub kv_heads: u32,
    pub head_dim: u32,
}

impl KvGeometry {
    /// Bytes per token of context, for both K and V.
    pub fn bytes_per_token(&self, bytes_per_element: f64) -> u64 {
        let per = 2.0 * self.layers as f64 * self.kv_heads as f64 * self.head_dim as f64 * bytes_per_element;
        per as u64
    }

    pub fn total_bytes(&self, ctx_tokens: u32, bytes_per_element: f64) -> u64 {
        self.bytes_per_token(bytes_per_element)
            .saturating_mul(ctx_tokens as u64)
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // The three artifacts measured in the M0 spike on an M5 Pro at 227.99 GB/s.
    const SPIKE_BW: f64 = 227.99;
    const Q05: u64 = 491_400_032;
    const Q3_Q4: u64 = 2_104_932_768;
    const Q3_Q8: u64 = 3_616_088_480;

    fn spike_observations() -> Vec<Observation> {
        vec![
            Observation { bytes_per_token: Q05, measured_decode_tps: 337.63, shape: ShapeClass::Dense },
            Observation { bytes_per_token: Q3_Q4, measured_decode_tps: 108.75, shape: ShapeClass::Dense },
            Observation { bytes_per_token: Q3_Q8, measured_decode_tps: 68.4, shape: ShapeClass::Dense },
        ]
    }

    #[test]
    fn reproduces_the_spike_parameters() {
        let cal = fit(&spike_observations(), SPIKE_BW);
        assert!(cal.determined);
        assert!((cal.k_cal - 1.175).abs() < 0.05, "k_cal = {}", cal.k_cal);
        assert!((cal.t0_ms - 1.22).abs() < 0.5, "t0 = {}", cal.t0_ms);
        assert!(cal.r2 > 0.99, "r2 = {}", cal.r2);
    }

    #[test]
    fn meets_the_accuracy_bar_against_the_spike_fixtures() {
        let obs = spike_observations();
        let cal = fit(&obs, SPIKE_BW);
        let e = mape(&cal, &obs, SPIKE_BW);
        assert!(e <= 10.0, "MAPE {e:.2}% exceeds the 10% bar");
    }

    #[test]
    fn k_cal_above_one_is_accepted_not_clamped() {
        let cal = fit(&spike_observations(), SPIKE_BW);
        assert!(cal.k_cal > 1.0, "the reference machine calibrates above 1.0");
        // And the prediction actually uses it.
        let p = predict(&cal, Q3_Q8, SPIKE_BW);
        assert!(p.decode_tps.p50 > 60.0);
    }

    #[test]
    fn fewer_than_three_observations_is_not_a_fit() {
        let obs = &spike_observations()[..2];
        let cal = fit(obs, SPIKE_BW);
        assert!(!cal.determined, "a two-point fit proves nothing");
        assert_eq!(cal.n, 2);
    }

    #[test]
    fn an_undetermined_fit_predicts_a_wide_interval() {
        let cal = fit(&spike_observations()[..1], SPIKE_BW);
        let p = predict(&cal, Q3_Q4, SPIKE_BW);
        assert!(!p.determined);
        assert!(p.decode_tps.half_width_pct() > 30.0, "uncalibrated must look uncalibrated");
    }

    #[test]
    fn a_determined_fit_predicts_a_narrow_interval() {
        let cal = fit(&spike_observations(), SPIKE_BW);
        let p = predict(&cal, Q3_Q4, SPIKE_BW);
        assert!(p.decode_tps.half_width_pct() < 20.0);
        assert!(p.decode_tps.p10 < p.decode_tps.p50 && p.decode_tps.p50 < p.decode_tps.p90);
    }

    #[test]
    fn a_bigger_artifact_decodes_slower() {
        let cal = fit(&spike_observations(), SPIKE_BW);
        let small = predict(&cal, Q05, SPIKE_BW).decode_tps.p50;
        let large = predict(&cal, Q3_Q8, SPIKE_BW).decode_tps.p50;
        assert!(small > large, "{small} should exceed {large}");
    }

    #[test]
    fn more_bandwidth_is_never_worse() {
        let cal = fit(&spike_observations(), SPIKE_BW);
        let slow = predict(&cal, Q3_Q4, 100.0).decode_tps.p50;
        let fast = predict(&cal, Q3_Q4, 400.0).decode_tps.p50;
        assert!(fast > slow);
    }

    #[test]
    fn a_degenerate_observation_set_does_not_panic() {
        let obs = vec![
            Observation { bytes_per_token: 0, measured_decode_tps: 10.0, shape: ShapeClass::Dense },
            Observation { bytes_per_token: 0, measured_decode_tps: 10.0, shape: ShapeClass::Dense },
            Observation { bytes_per_token: 0, measured_decode_tps: 10.0, shape: ShapeClass::Dense },
        ];
        let cal = fit(&obs, SPIKE_BW);
        assert!(!cal.determined);
    }

    #[test]
    fn a_zero_bandwidth_does_not_panic_or_invent_throughput() {
        let cal = fit(&spike_observations(), 0.0);
        assert!(!cal.determined);
        let p = predict(&cal, Q3_Q4, 0.0);
        assert!(p.decode_tps.p50.is_finite());
    }

    // --- KV geometry (PRD §5.2.3) -----------------------------------------

    const LLAMA_70B: KvGeometry = KvGeometry { layers: 80, kv_heads: 8, head_dim: 128 };

    #[test]
    fn kv_matches_the_prd_table_for_70b() {
        // PRD §5.2.3: 320 KiB/token fp16, 10.0 GiB at 32k, 40.0 GiB at 128k.
        let per_token = LLAMA_70B.bytes_per_token(2.0);
        assert_eq!(per_token, 327_680);
        let at_32k = LLAMA_70B.total_bytes(32_768, 2.0) as f64 / (1u64 << 30) as f64;
        assert!((at_32k - 10.0).abs() < 0.05, "got {at_32k} GiB");
        let at_128k = LLAMA_70B.total_bytes(131_072, 2.0) as f64 / (1u64 << 30) as f64;
        assert!((at_128k - 40.0).abs() < 0.2, "got {at_128k} GiB");
    }

    #[test]
    fn q8_0_kv_is_53_percent_not_50() {
        let fp16 = LLAMA_70B.bytes_per_token(2.0) as f64;
        // Q8_0_BYTES_PER_ELEMENT is already the per-element figure (1.0625 B),
        // against fp16's 2.0 B.
        let q8 = LLAMA_70B.bytes_per_token(Q8_0_BYTES_PER_ELEMENT) as f64;
        assert!((q8 / fp16 - 0.53125).abs() < 1e-6, "got {}", q8 / fp16);
    }

    #[test]
    fn a_70b_at_128k_needs_more_kv_than_its_own_weights() {
        let kv = LLAMA_70B.total_bytes(131_072, 2.0);
        let weights_4bit = 35u64 << 30; // ~35 GiB
        assert!(kv > weights_4bit, "the PRD's headline arithmetic must hold");
    }
}
