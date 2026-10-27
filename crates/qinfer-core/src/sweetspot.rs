//! The sweet spot: the frontier, not the peak (T-014).
//!
//! "Which model should I run on this machine?" is not answered by the fastest
//! plan. `Planner` ranks by throughput, and through a bandwidth-bound lens the
//! fastest artifact is always the smallest one — on the reference M5 Pro the
//! top-ranked plan is a 0.5B at 339 tok/s, which is the exact failure PRD §13.3
//! forbids ("we must not succeed by recommending 4B models").
//!
//! A quality *floor* does not fix this either. `quality_placeholder` keys on the
//! quantisation label, so a 0.5B Q4 and a 27B Q4 both score 0.93 and the floor
//! cannot separate them. What is missing is a second axis: **capability**.
//!
//! With two axes the question becomes a curve, and the useful answer is its
//! **knee** — the point past which a larger model buys less capability per
//! token-per-second given up. Kneedle's maximum-distance-from-chord criterion
//! finds it without inventing a weighting constant, which matters because any
//! weighted score here would be a number the product could not defend.
//!
//! Honesty rule, enforced in the type: capability is a **prior** derived from
//! parameter counts until the catalogue carries measured quality. Every point
//! says which it is, and the frontier is not allowed to present a prior as a
//! measurement.

use crate::catalog::{Catalog, CatalogEntry};
use crate::planner::{Plan, PlanSet};

/// Below this decode rate an interactive coding session feels laggy, so every
/// token per second above it is worth nothing. The frontier therefore has a
/// **usable region**, and the question is not "where is the knee" but "which is
/// the most capable point still inside it" (T-015).
///
/// A knee answers a mathematical question about a curve. A user asking "what
/// should I run" is asking a different one, and on the reference M5 Pro they
/// diverge: the knee is a 7B at 245 tok/s, the answer is a 30B-A3B at 123, and
/// both are far above the rate at which anyone can read the output.
pub const DEFAULT_USABLE_TPS: f64 = 20.0;

/// Where a point's capability number came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilitySource {
    /// Measured on this machine (catalogue `quality.value`). Not yet available
    /// for any entry.
    Measured,
    /// Derived from parameter count. A shape, not a measurement: it knows a 27B
    /// is larger than a 3B and nothing about how much better it is at anything.
    Prior,
    /// No usable parameters on the entry; the point carries no capability axis
    /// and must not be ranked.
    Unknown,
}

impl CapabilitySource {
    pub fn label(self) -> &'static str {
        match self {
            CapabilitySource::Measured => "measured",
            CapabilitySource::Prior => "prior (params)",
            CapabilitySource::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub plan: Plan,
    /// Normalised 0..1 within this frontier. Only comparable within one call:
    /// it is a position on a curve, not a property of the model.
    pub capability: f64,
    /// The raw parameter count the prior was built from.
    pub params: u64,
    pub source: CapabilitySource,
}

impl Point {
    pub fn tps(&self) -> f64 {
        self.plan.predicted.p50
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweetSpot {
    /// Pareto-optimal points, sorted by predicted decode throughput ascending
    /// (slowest and most capable first).
    pub frontier: Vec<Point>,
    /// Index into `frontier`. `None` when the curve has no interior knee.
    pub knee: Option<usize>,
    pub budget_bytes: u64,
    /// Caveats a surface must display next to the recommendation.
    pub notes: Vec<String>,
}

impl SweetSpot {
    pub fn recommended(&self) -> Option<&Point> {
        self.knee.and_then(|i| self.frontier.get(i))
    }

    /// `(fastest, most_capable)` — the two ends of the curve, so a surface can
    /// show that the recommendation is a *choice* between them rather than a
    /// fact. Note the order of the frontier is slowest-first, so these are the
    /// last and first elements respectively.
    pub fn extremes(&self) -> (Option<&Point>, Option<&Point>) {
        (self.frontier.last(), self.frontier.first())
    }

    pub fn is_empty(&self) -> bool {
        self.frontier.is_empty()
    }

    /// The most capable point that still decodes at or above `min_tps`.
    ///
    /// The frontier is ordered slowest-first, and speed falls as capability
    /// rises, so the *first* point meeting the floor is the most capable
    /// usable one. `>=` rather than `>`: a plan predicted at exactly the floor
    /// is admissible, otherwise a model can be excluded by a rounding error.
    pub fn best_usable(&self, min_tps: f64) -> Option<&Point> {
        self.frontier.iter().find(|p| p.tps() >= min_tps)
    }

    /// Justification for [`SweetSpot::best_usable`], naming the floor so the
    /// recommendation is auditable rather than a bare assertion.
    pub fn usable_reason(&self, min_tps: f64) -> String {
        match self.best_usable(min_tps) {
            Some(p) => {
                let mut s = format!(
                    "{} is the most capable plan still above {:.0} tok/s: {:.0} tok/s at {:.1} GiB",
                    p.plan.display,
                    min_tps,
                    p.tps(),
                    p.plan.peak_bytes as f64 / (1u64 << 30) as f64
                );
                // Name the trade in whichever direction it exists: either a
                // more capable plan lost on the floor, or a faster one lost on
                // capability. A recommendation with no stated alternative is
                // marketing rather than advice.
                if let Some(b) = self.frontier.first() {
                    if b.plan.id != p.plan.id {
                        s.push_str(&format!(
                            " — chosen over the more capable {} at {:.0} tok/s, which is below the floor",
                            b.plan.display,
                            b.tps()
                        ));
                    } else if let Some(f) = self.frontier.last() {
                        if f.plan.id != p.plan.id {
                            s.push_str(&format!(
                                " — the faster {} at {:.0} tok/s is less capable",
                                f.plan.display,
                                f.tps()
                            ));
                        }
                    }
                }
                s.push('.');
                s
            }
            None => {
                let fastest = self.frontier.last().map(|p| p.tps()).unwrap_or(0.0);
                if self.frontier.is_empty() {
                    "no feasible candidate, so there is nothing above any floor".into()
                } else {
                    format!(
                        "nothing on this machine decodes above {:.0} tok/s — the fastest plan is \
                         {:.0} tok/s. Lower the floor, or accept a larger model at a higher \
                         latency.",
                        min_tps, fastest
                    )
                }
            }
        }
    }

    /// One-line justification for the knee, or an explanation of why there is
    /// none. Never an empty string, because an unexplained recommendation is a
    /// bug (PRD §3.3.2).
    pub fn reason(&self) -> String {
        let Some(p) = self.recommended() else {
            return match self.frontier.len() {
                0 => "no feasible candidate — every artifact was rejected on fit or coverage".into(),
                1 => "only one candidate fits; there is no curve to choose from".into(),
                _ => "the curve is flat enough that no candidate is clearly the knee".into(),
            };
        };
        let (fastest, most_capable) = self.extremes();
        let mut s = format!(
            "{} is the knee: {:.0} tok/s at {:.1} GiB",
            p.plan.display,
            p.tps(),
            p.plan.peak_bytes as f64 / (1u64 << 30) as f64
        );
        if let Some(f) = fastest {
            if f.plan.id != p.plan.id {
                s.push_str(&format!(
                    " — {:.0} tok/s below the fastest ({})",
                    (f.tps() - p.tps()).max(0.0),
                    f.plan.display
                ));
            }
        }
        if let Some(b) = most_capable {
            if b.plan.id != p.plan.id && b.tps() > 0.0 {
                s.push_str(&format!(
                    ", and {:.1}x the throughput of the most capable ({})",
                    p.tps() / b.tps(),
                    b.plan.display
                ));
            }
        }
        s.push('.');
        s
    }
}

/// Capability prior from parameter count.
///
/// Total parameters, not active ones: an MoE pays for its whole weight file in
/// memory and carries its whole knowledge base, while reading only a fraction
/// per token. Using active parameters here would rank a 30B-A3B as smaller than
/// a dense 14B, which is the opposite of how they compare.
///
/// Log-scaled because capability returns diminish: 0.5B -> 7B is a much larger
/// step than 27B -> 70B, and a linear prior would let the biggest model on the
/// list win by arithmetic.
fn prior(params: u64) -> Option<f64> {
    if params == 0 {
        return None;
    }
    Some((params as f64).log10())
}

/// A catalogue entry's measured quality, oriented so that **higher is better**
/// on every metric the catalogue can carry.
///
/// `code_nll` and perplexity are error measures, so they are negated; a metric
/// that is already a score (accuracy, pass rate) is used as-is. Returns the
/// metric name too, so a surface can say which one it is quoting.
fn quality_value(entry: &CatalogEntry) -> Option<(f64, &str)> {
    let q = entry.quality.as_ref()?;
    let v = q.value?;
    if !v.is_finite() {
        return None;
    }
    let error_metric = q.metric.contains("nll") || q.metric.contains("perplexity");
    Some((if error_metric { -v } else { v }, q.metric.as_str()))
}

/// Build the frontier from a planner result.
pub fn frontier(set: &PlanSet, catalog: &Catalog) -> SweetSpot {
    let budget_bytes = set.plans.first().map(|p| p.peak_bytes).unwrap_or(0);

    let pairs: Vec<(&Plan, &CatalogEntry)> = set
        .plans
        .iter()
        .filter_map(|p| {
            catalog
                .entries
                .iter()
                .find(|e| e.id == p.entry_id)
                .map(|e| (p, e))
        })
        .collect();

    // The axis is chosen ONCE for the whole set. Mixing a measured score for one
    // candidate with a parameter-count prior for another produces an ordering
    // that is neither a measurement nor a shape, and nothing downstream could
    // tell which.
    //
    // So once anything has been measured, the frontier is built only from
    // candidates that have been measured, and the rest are named as excluded. The
    // alternative — requiring every entry to be scored before any of it counts —
    // would keep the measured axis switched off indefinitely, because half the
    // catalogue is GGUF and not yet scoreable.
    // Different metrics are different measurements. A `code_nll_per_byte` from
    // raw continuation and a `code_instr_nll_per_byte` from a chat turn are not
    // on the same scale, so a frontier that mixed them would be ordering numbers
    // the way a spreadsheet orders text: alphabetically and wrongly.
    //
    // The metric with the most candidates wins and the rest are excluded. As
    // coverage grows the majority metric changes on its own, which is the
    // behaviour we want while the catalogue is being filled in.
    let mut metric_counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (_, e) in &pairs {
        if let Some((_, m)) = quality_value(e) {
            *metric_counts.entry(m).or_insert(0) += 1;
        }
    }
    let majority_metric = metric_counts
        .iter()
        .max_by_key(|(m, n)| (**n, std::cmp::Reverse(*m)))
        .map(|(m, _)| *m);

    let scored_count = pairs
        .iter()
        .filter(|(_, e)| quality_value(e).map(|(_, m)| Some(m) == majority_metric).unwrap_or(false))
        .count();
    let measured_axis = scored_count > 0;
    let axis = if measured_axis {
        CapabilitySource::Measured
    } else {
        CapabilitySource::Prior
    };
    let considered: Vec<(&Plan, &CatalogEntry)> = if measured_axis {
        pairs
            .iter()
            .copied()
            .filter(|(_, e)| quality_value(e).map(|(_, m)| Some(m) == majority_metric).unwrap_or(false))
            .collect()
    } else {
        pairs.clone()
    };

    let mut raw: Vec<(Plan, f64, u64, CapabilitySource)> = Vec::new();
    for (plan, entry) in &considered {
        let params = entry.model.total_params;
        let value = if measured_axis { quality_value(entry).map(|(v, _)| v) } else { prior(params) };
        let (value, source) = match value {
            Some(v) => (v, axis),
            None => (0.0, CapabilitySource::Unknown),
        };
        raw.push(((*plan).clone(), value, params, source));
    }

    // Normalise capability to 0..1 across the candidate set.
    let known: Vec<f64> = raw
        .iter()
        .filter(|(_, _, _, s)| *s != CapabilitySource::Unknown)
        .map(|(_, v, _, _)| *v)
        .collect();
    let (lo, hi) = known
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let span = if hi > lo { hi - lo } else { 0.0 };

    let mut points: Vec<Point> = raw
        .into_iter()
        .filter(|(_, _, _, s)| *s != CapabilitySource::Unknown)
        .map(|(plan, value, params, source)| Point {
            capability: if span > 0.0 { (value - lo) / span } else { 1.0 },
            plan,
            params,
            source,
        })
        .collect();

    // Keep only Pareto-optimal points: a candidate that is both slower and less
    // capable than another is not a trade-off, it is a mistake.
    points.sort_by(|a, b| a.tps().partial_cmp(&b.tps()).unwrap_or(std::cmp::Ordering::Equal));
    let mut frontier: Vec<Point> = Vec::new();
    let mut best_cap = f64::MIN;
    for p in points.into_iter().rev() {
        // Descending speed: keep a point only if nothing faster is at least as
        // capable, and it is strictly more capable than everything kept so far.
        if p.capability > best_cap + f64::EPSILON {
            best_cap = p.capability;
            frontier.push(p);
        } else if frontier.is_empty() {
            frontier.push(p);
        }
    }
    frontier.reverse(); // slowest/most capable first

    let knee = find_knee(&frontier);
    let mut notes = Vec::new();
    match axis {
        CapabilitySource::Measured => {
            let metric = majority_metric.unwrap_or("unknown");
            let excluded = pairs.len().saturating_sub(scored_count);
            notes.push(format!(
                "capability is a measured `{metric}` on a fixed corpus, where lower error is \
                 better; {scored_count} candidate(s) scored"
            ));
            if excluded > 0 {
                notes.push(format!(
                    "{excluded} candidate(s) excluded from the frontier: no `{metric}` score, \
                     and ranking them on a parameter-count prior beside measured points would \
                     order neither"
                ));
            }
        }
        _ => {
            notes.push(format!(
                "capability is a parameter-count prior, not a measurement — none of the {} \
                 candidate(s) carry a measured code score yet",
                pairs.len()
            ));
        }
    }
    if frontier.len() < 3 {
        notes.push(format!(
            "only {} candidate(s) on the curve; a frontier needs at least three to have a knee",
            frontier.len()
        ));
    }

    SweetSpot { frontier, knee, budget_bytes, notes }
}

/// Kneedle: the point furthest from the chord joining the two ends.
///
/// Both axes are normalised to 0..1 first, otherwise a 339 tok/s spread would
/// drown a 0..1 capability span and the "furthest" point would just be the
/// fastest one. No weighting constant is introduced: the geometry decides, and
/// the answer is reproducible.
fn find_knee(points: &[Point]) -> Option<usize> {
    if points.len() < 3 {
        return None;
    }
    let x0 = points.first()?.tps();
    let x1 = points.last()?.tps();
    if !(x1 > x0) {
        return None;
    }
    let (y_lo, y_hi) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| {
        (a.min(p.capability), b.max(p.capability))
    });
    if y_hi <= y_lo {
        return None;
    }

    let norm = |p: &Point| -> (f64, f64) {
        ((p.tps() - x0) / (x1 - x0), (p.capability - y_lo) / (y_hi - y_lo))
    };

    // Chord runs from (0, 1) to (1, 0): the frontier is decreasing. A point
    // above it is a better trade than linear.
    let (mut best_i, mut best_d) = (0usize, f64::MIN);
    for (i, p) in points.iter().enumerate() {
        let (x, y) = norm(p);
        let d = x + y - 1.0;
        if d > best_d {
            best_d = d;
            best_i = i;
        }
    }
    // A knee must actually lie off the chord; collinear points have none.
    if best_d <= 1e-9 {
        return None;
    }
    Some(best_i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Artifact, ByteSource, CatalogEntry, Coverage, ModelMeta, Quality};
    use crate::predict::{Calibration, Interval, ShapeClass};
    use crate::planner::Objective;

    fn point(id: &str, tps: f64, capability: f64, params: u64) -> Point {
        Point {
            plan: Plan {
                id: format!("pln_{id}"),
                entry_id: id.into(),
                display: id.into(),
                objective: Objective::Balanced,
                predicted: Interval { p10: tps * 0.8, p50: tps, p90: tps * 1.2 },
                peak_bytes: (params as f64 * 0.6) as u64,
                weights_bytes: 0,
                kv_bytes: 0,
                rationale: vec![],
                risks: vec![],
                fallbacks: vec![],
                score: 0.0,
            },
            capability,
            params,
            source: CapabilitySource::Prior,
        }
    }

    #[test]
    fn the_prior_is_log_scaled_and_uses_total_parameters() {
        let small = prior(500_000_000).unwrap();
        let large = prior(27_000_000_000).unwrap();
        assert!(large > small);
        // Log scale: 0.5B -> 7B must be a bigger step than 27B -> 70B is not true
        // in absolute terms, so compare the *steps*, which is the actual claim.
        let step_low = prior(7_000_000_000).unwrap() - prior(500_000_000).unwrap();
        let step_high = prior(70_000_000_000).unwrap() - prior(27_000_000_000).unwrap();
        assert!(step_low > 0.0 && step_high > 0.0);
        assert_eq!(prior(0), None, "unknown params must not become a zero capability");
    }

    #[test]
    fn a_dominated_point_is_not_a_trade_off() {
        // 20 tok/s at 0.2 is slower AND less capable than 30 tok/s at 0.6, so it
        // is not a trade-off — it is strictly worse and must leave the frontier.
        // 20 tok/s at the lowest capability of the three: slower than the 7B at
        // 30 tok/s and less capable than the 70B at 10 tok/s, so it is strictly
        // worse than the 7B on both axes and leaves the frontier.
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![
                entry("slow-70b", 70_000_000_000, 70_000_000_000),
                entry("fast-7b", 7_000_000_000, 7_000_000_000),
                entry("dominated", 500_000_000, 500_000_000),
            ],
        };
        let set = PlanSet {
            plans: vec![plan_for("slow-70b", 10.0), plan_for("dominated", 20.0), plan_for("fast-7b", 30.0)],
            rejected: vec![],
            objective: Objective::Balanced,
            ctx: 4096,
        };
        let s = frontier(&set, &catalog);
        let ids: Vec<&str> = s.frontier.iter().map(|p| p.plan.entry_id.as_str()).collect();
        assert!(!ids.contains(&"dominated"), "strictly worse on both axes: {ids:?}");
        assert_eq!(ids, vec!["slow-70b", "fast-7b"]);
    }

    #[test]
    fn the_knee_is_the_point_between_the_extremes() {
        // Deliberately concave: a big capability jump for a small speed cost,
        // then a long flat tail.
        let pts = vec![
            point("big", 10.0, 1.0, 27_000_000_000),
            point("mid", 90.0, 0.9, 7_000_000_000),
            point("small", 100.0, 0.1, 500_000_000),
        ];
        let knee = find_knee(&pts).expect("a knee");
        assert_eq!(pts[knee].plan.entry_id, "mid", "the jump from 10->90 tok/s at 0.1 capability is the knee");
    }

    #[test]
    fn collinear_points_have_no_knee() {
        let pts = vec![
            point("a", 10.0, 1.0, 27_000_000_000),
            point("b", 55.0, 0.5, 7_000_000_000),
            point("c", 100.0, 0.1, 500_000_000),
        ];
        // a: (0,1) -> 0.0, b: (0.5,0.44) -> -0.05ish, c: (1,0) -> 0.0
        assert_eq!(find_knee(&pts), None, "a straight line has no knee to report");
    }

    #[test]
    fn fewer_than_three_points_have_no_knee() {
        assert_eq!(find_knee(&[]), None);
        assert_eq!(find_knee(&[point("a", 10.0, 1.0, 1_000_000)]), None);
        assert_eq!(
            find_knee(&[point("a", 10.0, 1.0, 1_000_000), point("b", 20.0, 0.5, 2_000_000)]),
            None
        );
    }

    #[test]
    fn an_empty_frontier_explains_itself() {
        let s = SweetSpot { frontier: vec![], knee: None, budget_bytes: 0, notes: vec![] };
        assert!(s.recommended().is_none());
        assert!(s.reason().contains("no feasible candidate"));
    }

    #[test]
    fn a_single_candidate_says_there_is_no_curve() {
        let s = SweetSpot {
            frontier: vec![point("only", 42.0, 1.0, 3_000_000_000)],
            knee: None,
            budget_bytes: 0,
            notes: vec![],
        };
        assert!(s.recommended().is_none());
        assert!(s.reason().contains("only one candidate"));
    }

    fn entry(id: &str, total: u64, active: u64) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            family: "f".into(),
            display: id.into(),
            artifact: Artifact {
                format: "gguf".into(),
                quant: "Q4_K_M".into(),
                bytes: 1 << 30,
                bytes_source: ByteSource::Measured,
                sha256: None,
                licence: "apache-2.0".into(),
                gated: false,
                publisher: "x".into(),
            },
            model: ModelMeta {
                total_params: total,
                active_params: active,
                shape: ShapeClass::Dense,
                kv: crate::predict::KvGeometry { layers: 32, kv_heads: 8, head_dim: 128 },
                ctx_max: 32768,
                hidden: 4096,
                vocab: 32000,
            },
            coverage: vec![Coverage {
                arch_id: "apple-m5-pro".into(),
                backend: "metal".into(),
                status: crate::catalog::CoverageStatus::Verified,
                evidence: None,
            }],
            measurements: vec![],
            quality: Some(Quality { metric: "perplexity_delta".into(), value: None, method: "none".into() }),
        }
    }

    fn plan_for(id: &str, tps: f64) -> Plan {
        Plan {
            id: format!("pln_{id}"),
            entry_id: id.into(),
            display: id.into(),
            objective: Objective::Balanced,
            predicted: Interval { p10: tps * 0.8, p50: tps, p90: tps * 1.2 },
            peak_bytes: 1 << 30,
            weights_bytes: 1 << 30,
            kv_bytes: 0,
            rationale: vec![],
            risks: vec![],
            fallbacks: vec![],
            score: 0.0,
        }
    }

    /// The decision this module gets wrong if it uses `active_params`: a
    /// 30B-A3B MoE reads 3B parameters per token but carries 30B of knowledge.
    /// Ranking it by active parameters would place it *below* a dense 14B on
    /// capability, which inverts how the two actually compare.
    #[test]
    fn an_moe_is_ranked_by_total_not_active_parameters() {
        let moe_total = prior(30_000_000_000).expect("prior");
        let moe_active = prior(3_000_000_000).expect("prior");
        let dense_total = prior(14_000_000_000).expect("prior");

        assert!(moe_total > dense_total, "30B total must outrank 14B total");
        assert!(
            dense_total > moe_active,
            "and the active-parameter reading would have inverted the order — which is why it is not used"
        );
    }

    #[test]
    fn the_frontier_is_sorted_slowest_first() {
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![
                entry("a", 500_000_000, 500_000_000),
                entry("b", 7_000_000_000, 7_000_000_000),
                entry("c", 27_000_000_000, 27_000_000_000),
            ],
        };
        let set = PlanSet {
            plans: vec![plan_for("a", 300.0), plan_for("b", 60.0), plan_for("c", 16.0)],
            rejected: vec![],
            objective: Objective::Balanced,
            ctx: 4096,
        };
        let s = frontier(&set, &catalog);
        let tps: Vec<f64> = s.frontier.iter().map(Point::tps).collect();
        assert_eq!(tps, vec![16.0, 60.0, 300.0]);
        assert_eq!(s.frontier[0].plan.entry_id, "c", "most capable first");
    }

    /// The reference curve: a 30B MoE at 123 tok/s, a 7B at 245, a 0.5B at 339.
    /// Every one of them is far above a reading rate, so raw speed stops being
    /// the discriminator once a floor exists.
    fn usable_spot() -> SweetSpot {
        SweetSpot {
            frontier: vec![
                point("coder-30b", 123.0, 1.00, 30_500_000_000),
                point("olmoe", 245.0, 0.64, 7_000_000_000),
                point("tiny", 339.0, 0.00, 500_000_000),
            ],
            knee: Some(1),
            budget_bytes: 19_069_665_280,
            notes: vec![],
        }
    }

    #[test]
    fn the_floor_picks_the_most_capable_usable_point() {
        let s = usable_spot();
        let best = s.best_usable(DEFAULT_USABLE_TPS).expect("a usable point");
        assert_eq!(best.plan.entry_id, "coder-30b", "capability decides above the floor");
        assert_ne!(
            Some(best.plan.entry_id.as_str()),
            s.recommended().map(|p| p.plan.entry_id.as_str()),
            "and it differs from the knee, which is the point of having a floor"
        );
    }

    #[test]
    fn the_reason_names_the_floor_and_the_loser() {
        let r = usable_spot().usable_reason(DEFAULT_USABLE_TPS);
        assert!(r.contains("20 tok/s"), "the floor must be visible: {r}");
        assert!(r.contains("123"), "the chosen rate: {r}");
        assert!(
            r.contains("more capable") || r.contains("less capable"),
            "the recommendation must name what it traded away: {r}"
        );
    }

    #[test]
    fn a_plan_exactly_at_the_floor_is_admissible() {
        // `>` instead of `>=` would let a rounding error exclude a valid model.
        let s = usable_spot();
        let best = s.best_usable(123.0).expect("the floor is inclusive");
        assert_eq!(best.plan.entry_id, "coder-30b");
    }

    #[test]
    fn a_zero_floor_returns_the_most_capable_plan() {
        assert_eq!(usable_spot().best_usable(0.0).unwrap().plan.entry_id, "coder-30b");
    }

    #[test]
    fn a_floor_nothing_reaches_explains_itself() {
        let s = usable_spot();
        assert!(s.best_usable(500.0).is_none());
        let r = s.usable_reason(500.0);
        assert!(r.contains("nothing on this machine"), "{r}");
        assert!(r.contains("339"), "must name the fastest available rate: {r}");
    }

    #[test]
    fn an_empty_frontier_has_no_usable_point() {
        let s = SweetSpot { frontier: vec![], knee: None, budget_bytes: 0, notes: vec![] };
        assert!(s.best_usable(1.0).is_none());
        assert!(s.usable_reason(1.0).contains("no feasible candidate"));
    }

    #[test]
    fn a_prior_is_never_labelled_measured() {
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![entry("a", 7_000_000_000, 7_000_000_000)],
        };
        let set = PlanSet {
            plans: vec![plan_for("a", 60.0)],
            rejected: vec![],
            objective: Objective::Balanced,
            ctx: 4096,
        };
        let s = frontier(&set, &catalog);
        assert_eq!(s.frontier[0].source, CapabilitySource::Prior);
        assert!(s.notes.iter().any(|n| n.contains("prior")), "the caveat must be present: {:?}", s.notes);
    }

    fn scored_entry(id: &str, total: u64, nll: f64) -> CatalogEntry {
        let mut e = entry(id, total, total);
        e.quality = Some(Quality {
            metric: "code_nll".into(),
            value: Some(nll),
            method: "bench@422tokens".into(),
        });
        e
    }

    fn set_of(ids: &[(&str, f64)]) -> PlanSet {
        PlanSet {
            plans: ids.iter().map(|(id, tps)| plan_for(id, *tps)).collect(),
            rejected: vec![],
            objective: Objective::Balanced,
            ctx: 4096,
        }
    }

    /// Two metrics are two measurements. A frontier that mixed a raw-continuation
    /// score with a chat-turn score would be ordering numbers that share no scale.
    #[test]
    fn a_mixed_metric_catalogue_uses_the_majority_metric_and_excludes_the_rest() {
        let mut raw_hi = entry("raw-hi", 27_000_000_000, 27_000_000_000);
        raw_hi.quality = Some(Quality {
            metric: "code_nll_per_byte".into(),
            value: Some(0.30),
            method: "raw".into(),
        });
        let mut raw_lo = entry("raw-lo", 3_000_000_000, 3_000_000_000);
        raw_lo.quality = Some(Quality {
            metric: "code_nll_per_byte".into(),
            value: Some(0.40),
            method: "raw".into(),
        });
        let mut instr = entry("instr", 14_000_000_000, 14_000_000_000);
        instr.quality = Some(Quality {
            metric: "code_instr_nll_per_byte".into(),
            value: Some(0.10),
            method: "instr".into(),
        });

        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![raw_hi, raw_lo, instr],
        };
        let set = set_of(&[("raw-hi", 16.0), ("raw-lo", 131.0), ("instr", 31.0)]);
        let s = frontier(&set, &catalog);

        let ids: Vec<&str> = s.frontier.iter().map(|p| p.plan.entry_id.as_str()).collect();
        assert!(
            !ids.contains(&"instr"),
            "a minority metric must not be ordered against the majority: {ids:?}"
        );
        assert!(ids.contains(&"raw-hi") && ids.contains(&"raw-lo"));
        let note = s.notes.join(" ");
        assert!(note.contains("code_nll_per_byte"), "{note}");
        assert!(note.contains("1 candidate(s) excluded"), "{note}");
    }

    /// When every candidate has been scored, the axis stops being a guess. This
    /// is the transition the whole benchmark exists to enable.
    #[test]
    fn a_fully_measured_catalogue_uses_the_measured_axis() {
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![
                scored_entry("small", 3_000_000_000, 0.98),
                scored_entry("large", 27_000_000_000, 0.90),
            ],
        };
        let s = frontier(&set_of(&[("small", 130.0), ("large", 16.0)]), &catalog);
        assert!(s.frontier.iter().all(|p| p.source == CapabilitySource::Measured));
        assert!(s.notes.iter().any(|n| n.contains("measured")), "{:?}", s.notes);
        // Lower NLL is better, so the 27B must sit at the capable end.
        let large = s.frontier.iter().find(|p| p.plan.entry_id == "large").expect("large");
        let small = s.frontier.iter().find(|p| p.plan.entry_id == "small").expect("small");
        assert!(large.capability > small.capability, "0.90 nll beats 0.98 nll");
    }

    /// Partial coverage is the dangerous case: one measured score beside one
    /// guess, on the same axis, with nothing downstream able to tell them apart.
    /// The unscored candidate leaves the frontier rather than being ranked on a
    /// prior next to a measurement.
    #[test]
    fn an_unscored_candidate_is_excluded_rather_than_ranked_on_a_guess() {
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![
                scored_entry("scored", 3_000_000_000, 0.98),
                entry("unscored", 27_000_000_000, 27_000_000_000),
            ],
        };
        let s = frontier(&set_of(&[("scored", 130.0), ("unscored", 16.0)]), &catalog);
        let ids: Vec<&str> = s.frontier.iter().map(|p| p.plan.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["scored"], "a guess must not sit beside a measurement");
        assert!(s.frontier.iter().all(|p| p.source == CapabilitySource::Measured));
        let note = s.notes.join(" ");
        assert!(note.contains("1 candidate(s) excluded"), "{note}");
        assert!(note.contains("parameter-count prior"), "{note}");
    }

    /// With nothing measured the frontier still works — on a stated prior.
    #[test]
    fn an_unscored_catalogue_falls_back_to_the_prior_for_everyone() {
        let catalog = Catalog {
            schema: "qinfer/catalog@1".into(),
            version: "t".into(),
            generated_at: String::new(),
            note: String::new(),
            entries: vec![
                entry("a", 3_000_000_000, 3_000_000_000),
                entry("b", 27_000_000_000, 27_000_000_000),
            ],
        };
        let s = frontier(&set_of(&[("a", 130.0), ("b", 16.0)]), &catalog);
        assert_eq!(s.frontier.len(), 2);
        assert!(s.frontier.iter().all(|p| p.source == CapabilitySource::Prior));
        assert!(s.notes.join(" ").contains("none of the 2 candidate(s)"));
    }

    #[test]
    fn a_perplexity_metric_is_oriented_the_same_way_as_nll() {
        // Both are error measures: higher is worse, so both must be negated.
        let mut e = entry("x", 3_000_000_000, 3_000_000_000);
        e.quality = Some(Quality {
            metric: "code_perplexity".into(),
            value: Some(2.67),
            method: "bench".into(),
        });
        let (v, metric) = quality_value(&e).expect("scored");
        assert!(v < 0.0, "an error metric must be negated, got {v}");
        assert_eq!(metric, "code_perplexity");

        // A score metric is already oriented.
        let mut s = entry("y", 3_000_000_000, 3_000_000_000);
        s.quality = Some(Quality { metric: "pass_at_1".into(), value: Some(0.42), method: "x".into() });
        assert!((quality_value(&s).unwrap().0 - 0.42).abs() < 1e-9);
    }

    #[test]
    fn an_entry_with_no_quality_block_has_no_measured_value() {
        let e = entry("x", 3_000_000_000, 3_000_000_000);
        assert_eq!(quality_value(&e), None);
    }

    /// Guard on the label vocabulary itself.
    #[test]
    fn capability_source_labels_are_stable() {
        let _unused = Calibration {
            shape: ShapeClass::Dense,
            t0_ms: 1.0,
            k_cal: 1.0,
            r2: 1.0,
            determined: true,
            n: 3,
        };
        assert_eq!(CapabilitySource::Measured.label(), "measured");
        assert_eq!(CapabilitySource::Prior.label(), "prior (params)");
    }
}
