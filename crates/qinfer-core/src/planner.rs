//! The planner (T-006).
//!
//! Turns a catalogue and a calibration into a **ranked, explained** set of
//! plans. Three properties are non-negotiable:
//!
//! 1. **Hard constraints prune, they do not score.** An artifact that does not
//!    fit, has no coverage for this architecture, or whose size is a guess is
//!    removed with a stated reason — never ranked last and quietly offered.
//! 2. **Every plan explains itself.** `rationale` is generated from the
//!    fingerprint and the catalogue. A plan the planner cannot explain, it does
//!    not emit (PRD §5.3).
//! 3. **Rank ordering is the product** (ADR-0003). Point accuracy can drift;
//!    if the order is wrong the tool has no reason to exist. The acceptance
//!    test for this module is that it reproduces the measured ordering of the
//!    four artifacts in this repository.

use crate::catalog::{ByteSource, Catalog, CatalogEntry, CoverageStatus};
use crate::predict::{self, Calibration, Interval, ShapeClass};

/// How much to trust a quantisation label when nothing better exists.
///
/// PLACEHOLDER. The PRD's quality floor is supposed to come from measured
/// perplexity/KL, and the catalogue has none yet (`quality.value` is `None` for
/// every entry). These constants are a crude monotonic stand-in so that
/// `Balanced` is not silently identical to `MaxSpeed`. Any plan that leans on
/// them says so in its rationale.
fn quality_placeholder(quant: &str) -> f64 {
    let q = quant.to_ascii_uppercase();
    if q.contains("FP16") || q.contains("BF16") {
        1.00
    } else if q.starts_with("Q8") {
        0.99
    } else if q.starts_with("Q6") {
        0.97
    } else if q.starts_with("Q5") {
        0.95
    } else if q.starts_with("Q4") {
        0.93
    } else if q.starts_with("Q3") {
        0.84
    } else if q.starts_with("Q2") {
        0.70
    } else {
        0.90
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// Highest predicted decode throughput.
    MaxSpeed,
    /// Throughput weighted by the quantisation placeholder.
    Balanced,
    /// Quantisation quality first, throughput as the tie-break.
    MaxQuality,
    /// Longest context first, subject to fitting.
    MaxContext,
    /// Anything that fits and runs.
    Fit,
}

impl Objective {
    pub fn parse(s: &str) -> Option<Objective> {
        Some(match s.to_ascii_lowercase().as_str() {
            "max_speed" | "max-speed" | "speed" => Objective::MaxSpeed,
            "balanced" => Objective::Balanced,
            "max_quality" | "max-quality" | "quality" => Objective::MaxQuality,
            "max_context" | "max-context" | "context" => Objective::MaxContext,
            "fit" => Objective::Fit,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Objective::MaxSpeed => "max_speed",
            Objective::Balanced => "balanced",
            Objective::MaxQuality => "max_quality",
            Objective::MaxContext => "max_context",
            Objective::Fit => "fit",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanRequest {
    /// Restrict to one model family. `None` considers the whole catalogue,
    /// which is what "what should I run on this box?" actually asks.
    pub family: Option<String>,
    pub ctx: u32,
    pub concurrency: u32,
    pub objective: Objective,
}

impl Default for PlanRequest {
    fn default() -> Self {
        PlanRequest { family: None, ctx: 4096, concurrency: 1, objective: Objective::Balanced }
    }
}

#[derive(Debug, Clone)]
pub struct Constraints {
    pub arch_id: String,
    /// Bytes available to this workload (already net of OS/runtime headroom).
    pub available_bytes: u64,
    pub kv_bytes_per_element: f64,
}

impl Constraints {
    pub fn new(arch_id: impl Into<String>, available_bytes: u64) -> Self {
        Constraints { arch_id: arch_id.into(), available_bytes, kv_bytes_per_element: 2.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub id: String,
    pub entry_id: String,
    pub display: String,
    pub objective: Objective,
    pub predicted: Interval,
    pub peak_bytes: u64,
    pub weights_bytes: u64,
    pub kv_bytes: u64,
    pub rationale: Vec<String>,
    pub risks: Vec<String>,
    pub fallbacks: Vec<String>,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rejection {
    NotPlannable { entry_id: String },
    NoCoverage { entry_id: String, arch_id: String },
    DoesNotFit { entry_id: String, peak_bytes: u64, available_bytes: u64, weights_bytes: u64, kv_bytes: u64 },
    WrongFamily { entry_id: String, requested: String },
}

impl Rejection {
    pub fn entry_id(&self) -> &str {
        match self {
            Rejection::NotPlannable { entry_id }
            | Rejection::NoCoverage { entry_id, .. }
            | Rejection::DoesNotFit { entry_id, .. }
            | Rejection::WrongFamily { entry_id, .. } => entry_id,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Rejection::NotPlannable { .. } => "E-FIT-000",
            Rejection::NoCoverage { .. } => "E-ARCH-001",
            Rejection::DoesNotFit { .. } => "E-FIT-001",
            Rejection::WrongFamily { .. } => "E-CAT-012",
        }
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::NotPlannable { entry_id } => write!(
                f,
                "{entry_id}: artifact size is estimated, not measured — cannot plan on a guess"
            ),
            Rejection::NoCoverage { entry_id, arch_id } => {
                write!(f, "{entry_id}: no coverage for {arch_id}")
            }
            Rejection::DoesNotFit { entry_id, peak_bytes, available_bytes, weights_bytes, kv_bytes } => {
                write!(
                    f,
                    "{entry_id}: needs {:.1} GiB ({:.1} weights + {:.1} KV) but only {:.1} GiB available",
                    gib(*peak_bytes),
                    gib(*weights_bytes),
                    gib(*kv_bytes),
                    gib(*available_bytes)
                )
            }
            Rejection::WrongFamily { entry_id, requested } => {
                write!(f, "{entry_id}: not in family {requested}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanSet {
    pub plans: Vec<Plan>,
    pub rejected: Vec<Rejection>,
    pub objective: Objective,
    pub ctx: u32,
}

impl PlanSet {
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    /// The constraint that removed the most candidates — what to tell the user
    /// when nothing survives.
    pub fn binding_constraint(&self) -> Option<&Rejection> {
        let mut counts: Vec<(&str, usize, &Rejection)> = Vec::new();
        for r in &self.rejected {
            let code = r.code();
            if let Some(slot) = counts.iter_mut().find(|(c, _, _)| *c == code) {
                slot.1 += 1;
            } else {
                counts.push((code, 1, r));
            }
        }
        counts.into_iter().max_by_key(|(_, n, _)| *n).map(|(_, _, r)| r)
    }
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

pub struct Planner<'a> {
    catalog: &'a Catalog,
    calibration: &'a Calibration,
    bw_gbs: f64,
}

impl<'a> Planner<'a> {
    pub fn new(catalog: &'a Catalog, calibration: &'a Calibration, bw_gbs: f64) -> Self {
        Planner { catalog, calibration, bw_gbs }
    }

    pub fn plan(&self, req: &PlanRequest, constraints: &Constraints) -> PlanSet {
        let mut candidates: Vec<Plan> = Vec::new();
        let mut rejected: Vec<Rejection> = Vec::new();

        for entry in &self.catalog.entries {
            // Family filter
            if let Some(want) = &req.family {
                if &entry.family != want {
                    rejected.push(Rejection::WrongFamily {
                        entry_id: entry.id.clone(),
                        requested: want.clone(),
                    });
                    continue;
                }
            }

            // Coverage
            if !entry.coverage.iter().any(|c| c.arch_id == constraints.arch_id) {
                rejected.push(Rejection::NoCoverage {
                    entry_id: entry.id.clone(),
                    arch_id: constraints.arch_id.clone(),
                });
                continue;
            }

            // A guess is not a fact
            if !entry.is_plannable() {
                rejected.push(Rejection::NotPlannable { entry_id: entry.id.clone() });
                continue;
            }

            // Fit — resident bytes, not bytes-per-token. A MoE reads a fraction
            // of its experts per token but still holds the whole file.
            let weights_bytes = entry.artifact.bytes;
            let kv_bytes = entry.model.kv.total_bytes(req.ctx, constraints.kv_bytes_per_element);
            let headroom = weights_bytes / 20; // 5% for activations and fragmentation
            let peak_bytes = weights_bytes.saturating_add(kv_bytes).saturating_add(headroom);

            if peak_bytes > constraints.available_bytes {
                rejected.push(Rejection::DoesNotFit {
                    entry_id: entry.id.clone(),
                    peak_bytes,
                    available_bytes: constraints.available_bytes,
                    weights_bytes,
                    kv_bytes,
                });
                continue;
            }

            let prediction = predict::predict(self.calibration, entry.bytes_per_token(), self.bw_gbs);
            let quality = quality_placeholder(&entry.artifact.quant);
            let score = match req.objective {
                Objective::MaxSpeed | Objective::Fit => prediction.decode_tps.p50,
                Objective::Balanced => prediction.decode_tps.p50 * quality,
                Objective::MaxQuality => quality * 1_000_000.0 + prediction.decode_tps.p50,
                Objective::MaxContext => entry.model.ctx_max as f64 * 1_000_000.0 + prediction.decode_tps.p50,
            };

            candidates.push(Plan {
                id: format!("pln_{}", entry.id),
                entry_id: entry.id.clone(),
                display: if entry.display.is_empty() { entry.id.clone() } else { entry.display.clone() },
                objective: req.objective,
                predicted: prediction.decode_tps.clone(),
                peak_bytes,
                weights_bytes,
                kv_bytes,
                rationale: self.rationale(entry, req, constraints, &prediction, peak_bytes, weights_bytes, kv_bytes),
                risks: self.risks(entry, constraints, req),
                fallbacks: Vec::new(),
                score,
            });
        }

        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Pre-compute fallbacks: the next two plans, best-first.
        let ids: Vec<String> = candidates.iter().map(|p| p.id.clone()).collect();
        for (i, plan) in candidates.iter_mut().enumerate() {
            plan.fallbacks = ids.iter().skip(i + 1).take(2).cloned().collect();
        }

        PlanSet { plans: candidates, rejected, objective: req.objective, ctx: req.ctx }
    }

    fn rationale(
        &self,
        entry: &CatalogEntry,
        req: &PlanRequest,
        constraints: &Constraints,
        prediction: &predict::Prediction,
        peak: u64,
        weights: u64,
        kv: u64,
    ) -> Vec<String> {
        let mut r = Vec::new();

        r.push(format!(
            "reads {:.0} MB/token ({} {}), so decode is bandwidth-bound at {:.0} GB/s",
            entry.bytes_per_token() as f64 / 1e6,
            entry.artifact.quant,
            entry.artifact.format,
            self.bw_gbs
        ));

        r.push(format!(
            "predicted {:.0} tok/s (p10 {:.0} – p90 {:.0}), from t₀ {:.2} ms and k_cal {:.3} calibrated on this machine",
            prediction.decode_tps.p50,
            prediction.decode_tps.p10,
            prediction.decode_tps.p90,
            self.calibration.t0_ms,
            self.calibration.k_cal
        ));

        r.push(format!(
            "peak {:.1} GiB at {} tokens: {:.1} weights + {:.1} KV + 5% headroom, against {:.1} GiB available",
            gib(peak),
            req.ctx,
            gib(weights),
            gib(kv),
            gib(constraints.available_bytes)
        ));

        r.push(format!("ranked by objective `{}` for {} concurrent request(s)", req.objective.label(), req.concurrency));

        if !prediction.determined {
            r.push("calibration is underdetermined — the interval is widened to ±45%".to_string());
        }

        r
    }

    fn risks(&self, entry: &CatalogEntry, constraints: &Constraints, req: &PlanRequest) -> Vec<String> {
        let mut risks = Vec::new();

        let verified = entry
            .coverage
            .iter()
            .any(|c| c.arch_id == constraints.arch_id && c.status == CoverageStatus::Verified);
        if !verified {
            risks.push(format!(
                "coverage for {} is declared, not verified here — the first run may not match the prediction",
                constraints.arch_id
            ));
        }

        if entry.measurements.is_empty() {
            risks.push("no on-device measurement for this artifact yet".to_string());
        }

        if entry.model.shape == ShapeClass::Moe {
            risks.push(format!(
                "MoE: prediction assumes {:.1} B active parameters per token; routing imbalance moves this by up to ±20%",
                entry.model.active_params as f64 / 1e9
            ));
        }

        if req.ctx > 8192 {
            risks.push(format!("at {} tokens the KV cache is {:.1} GiB and grows linearly", req.ctx, gib(entry.model.kv.total_bytes(req.ctx, 2.0))));
        }

        if entry.artifact.gated {
            risks.push(format!("{} is a gated model — licence acceptance required", entry.artifact.licence));
        }

        if entry.artifact.bytes_source == ByteSource::Estimated {
            risks.push("artifact size is estimated".to_string());
        }

        if entry.artifact.sha256.is_none() {
            risks.push("artifact is not content-addressed yet".to_string());
        }

        // A verified, measured, ungated artifact at small context genuinely has
        // no elevated risk — but the contract requires the field to be filled,
        // and "no risks" is a claim worth stating explicitly rather than
        // leaving blank (PRD §3.3).
        if risks.is_empty() {
            risks.push(
                "no elevated risks: coverage verified on this machine, artifact measured and hashed, context within the model's native window"
                    .to_string(),
            );
        }

        risks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predict::{fit, Observation};

    const BW: f64 = 227.99;
    const SEED: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.26.json");

    fn catalog() -> Catalog {
        Catalog::from_json(SEED).expect("catalog")
    }

    /// Calibration fitted on the three dense spike models — nothing else.
    fn calibration() -> Calibration {
        fit(
            &[
                Observation { bytes_per_token: 491_400_032, measured_decode_tps: 337.63, shape: ShapeClass::Dense },
                Observation { bytes_per_token: 2_104_932_768, measured_decode_tps: 108.75, shape: ShapeClass::Dense },
                Observation { bytes_per_token: 3_616_088_480, measured_decode_tps: 68.4, shape: ShapeClass::Dense },
            ],
            BW,
        )
    }

    fn planner<'a>(c: &'a Catalog, cal: &'a Calibration) -> Planner<'a> {
        Planner::new(c, cal, BW)
    }

    const BIG: u64 = 24 * (1 << 30);

    #[test]
    fn plans_only_the_artifacts_we_can_size() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", BIG));
        assert_eq!(set.plans.len(), 4, "only the four measured artifacts are plannable");
        let rejected_estimates = set
            .rejected
            .iter()
            .filter(|r| matches!(r, Rejection::NotPlannable { .. }))
            .count();
        assert_eq!(rejected_estimates, 8);
    }

    /// The acceptance test that matters: predicted order == measured order.
    #[test]
    fn rank_order_matches_the_measurements() {
        let c = catalog();
        let cal = calibration();
        let req = PlanRequest { objective: Objective::MaxSpeed, ..Default::default() };
        let set = planner(&c, &cal).plan(&req, &Constraints::new("apple-m5-pro", BIG));

        let predicted: Vec<&str> = set.plans.iter().map(|p| p.entry_id.as_str()).collect();
        // Measured on this machine: 0.5B 337.6 > OLMoE 254.4 > 3B-Q4 108.8 > 3B-Q8 68.4
        assert_eq!(
            predicted,
            vec!["qwen2.5-0.5b-q4_k_m", "olmoe-1b-7b-q4_k_m", "qwen2.5-3b-q4_k_m", "qwen2.5-3b-q8_0"]
        );
    }

    #[test]
    fn every_plan_explains_itself() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", BIG));
        for p in &set.plans {
            assert!(p.rationale.len() >= 4, "{} has only {} rationale lines", p.entry_id, p.rationale.len());
            assert!(p.rationale.iter().all(|s| !s.trim().is_empty()));
            assert!(!p.risks.is_empty(), "{} has no risks", p.entry_id);
        }
    }

    #[test]
    fn the_top_plan_carries_fallbacks() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", BIG));
        assert_eq!(set.plans[0].fallbacks.len(), 2, "top plan needs two fallbacks");
        assert_eq!(set.plans[1].fallbacks.len(), 2);
        assert_eq!(set.plans[2].fallbacks.len(), 1);
        assert!(set.plans[3].fallbacks.is_empty(), "last plan has nothing to fall back to");
    }

    #[test]
    fn a_machine_too_small_rejects_with_arithmetic() {
        let c = catalog();
        let cal = calibration();
        let three_gib = 3 * (1 << 30);
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", three_gib));

        // 0.5B (~0.54 GiB) and 3B-Q4 (~2.28 GiB incl. headroom) fit in 3 GiB;
        // the 3.6 and 4.2 GiB artifacts do not.
        assert_eq!(set.plans.len(), 2);
        let fit = set
            .rejected
            .iter()
            .find(|r| matches!(r, Rejection::DoesNotFit { .. }))
            .expect("a fit rejection");
        let msg = fit.to_string();
        assert!(msg.contains("needs") && msg.contains("GiB"), "reason must show arithmetic: {msg}");
    }

    #[test]
    fn when_nothing_fits_the_reason_shows_the_arithmetic() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", 1 << 20));
        assert!(set.is_empty());

        // Every plannable artifact is rejected for fit, and the message carries
        // the numbers a user needs to act on ("buy more RAM" vs "use a smaller quant").
        let fit: Vec<&Rejection> = set
            .rejected
            .iter()
            .filter(|r| matches!(r, Rejection::DoesNotFit { .. }))
            .collect();
        assert_eq!(fit.len(), 4, "all four plannable artifacts must be rejected for fit");
        let msg = fit[0].to_string();
        assert!(msg.contains("needs") && msg.contains("available"), "unhelpful reason: {msg}");

        // The binding constraint is still surfaced — here it is "size is a guess",
        // because 8 of the 12 entries cannot be sized at all.
        let binding = set.binding_constraint().expect("binding constraint");
        assert!(!binding.to_string().is_empty());
    }

    #[test]
    fn an_architecture_with_no_coverage_yields_nothing_but_says_why() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("sm_121", BIG));
        assert!(set.is_empty());
        assert!(set.rejected.iter().all(|r| matches!(r, Rejection::NoCoverage { .. })));
    }

    #[test]
    fn family_filter_restricts_candidates() {
        let c = catalog();
        let cal = calibration();
        let req = PlanRequest { family: Some("qwen2.5".into()), ..Default::default() };
        let set = planner(&c, &cal).plan(&req, &Constraints::new("apple-m5-pro", BIG));
        assert!(!set.is_empty());
        assert!(set.plans.iter().all(|p| p.entry_id.starts_with("qwen2.5")));
        assert!(set.rejected.iter().any(|r| matches!(r, Rejection::WrongFamily { .. })));
    }

    #[test]
    fn max_quality_prefers_the_higher_bit_quant_over_the_faster_one() {
        let c = catalog();
        let cal = calibration();
        let fast = planner(&c, &cal).plan(
            &PlanRequest { objective: Objective::MaxSpeed, family: Some("qwen2.5".into()), ..Default::default() },
            &Constraints::new("apple-m5-pro", BIG),
        );
        let quality = planner(&c, &cal).plan(
            &PlanRequest { objective: Objective::MaxQuality, family: Some("qwen2.5".into()), ..Default::default() },
            &Constraints::new("apple-m5-pro", BIG),
        );
        assert_ne!(
            fast.plans[0].entry_id, quality.plans[0].entry_id,
            "the objective must change the answer, or it is decoration"
        );
        assert_eq!(quality.plans[0].entry_id, "qwen2.5-3b-q8_0");
    }

    #[test]
    fn a_long_context_plan_accounts_for_kv() {
        let c = catalog();
        let cal = calibration();
        let req = PlanRequest { ctx: 32_768, ..Default::default() };
        let set = planner(&c, &cal).plan(&req, &Constraints::new("apple-m5-pro", BIG));
        let three_b = set.plans.iter().find(|p| p.entry_id == "qwen2.5-3b-q4_k_m").expect("3b plan");
        // 36 layers * 2 kv heads * 128 * 2 (K,V) * 2 bytes * 32768
        assert_eq!(three_b.kv_bytes, 36 * 2 * 128 * 2 * 2 * 32_768);
        assert!(three_b.rationale.iter().any(|r| r.contains("KV")));
    }

    #[test]
    fn a_moe_plan_says_it_is_a_moe_and_quantifies_the_risk() {
        let c = catalog();
        let cal = calibration();
        let set = planner(&c, &cal).plan(&PlanRequest::default(), &Constraints::new("apple-m5-pro", BIG));
        let moe = set.plans.iter().find(|p| p.entry_id == "olmoe-1b-7b-q4_k_m").expect("moe plan");
        assert!(moe.risks.iter().any(|r| r.contains("MoE") && r.contains("active")));
        // The fit must use resident size, not the active fraction.
        assert_eq!(moe.weights_bytes, 4_211_730_432);
        assert!(moe.peak_bytes > moe.weights_bytes);
    }

    #[test]
    fn objective_parsing_is_strict() {
        assert_eq!(Objective::parse("max_speed"), Some(Objective::MaxSpeed));
        assert_eq!(Objective::parse("Balanced"), Some(Objective::Balanced));
        assert_eq!(Objective::parse("turbo"), None);
    }
}
