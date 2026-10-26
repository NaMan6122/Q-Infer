//! Print what the planner would choose for a machine like this one.
//!
//! `cargo run --release -p qinfer-core --example plan`
//!
//! This is the product in one screen: a ranked, explained set of plans, with
//! the constraint arithmetic shown and the fallbacks named.

use qinfer_core::catalog::Catalog;
use qinfer_core::planner::{Constraints, Objective, PlanRequest, Planner, Rejection};
use qinfer_core::predict::{fit, Observation, ShapeClass};

const BW_GBS: f64 = 227.99; // measured saturated bandwidth, spikes/m0
const CATALOG: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.26.json");

fn main() {
    let catalog = Catalog::from_json(CATALOG).expect("catalogue");
    if let Err(v) = catalog.validate() {
        for x in v {
            eprintln!("catalogue invalid: {}", x);
        }
        std::process::exit(2);
    }

    // Calibrated on the three dense spike measurements — nothing else.
    let calibration = fit(
        &[
            Observation { bytes_per_token: 491_400_032, measured_decode_tps: 337.63, shape: ShapeClass::Dense },
            Observation { bytes_per_token: 2_104_932_768, measured_decode_tps: 108.75, shape: ShapeClass::Dense },
            Observation { bytes_per_token: 3_616_088_480, measured_decode_tps: 68.4, shape: ShapeClass::Dense },
        ],
        BW_GBS,
    );

    // 24 GiB unified, minus OS/runtime headroom.
    let available = 12 * (1u64 << 30);
    let constraints = Constraints::new("apple-m5-pro", available);
    let request = PlanRequest { ctx: 4096, ..Default::default() };

    let planner = Planner::new(&catalog, &calibration, BW_GBS);
    let set = planner.plan(&request, &constraints);

    println!(
        "Q-Infer · {} · {:.1} GiB available · ctx {} · objective {}\n",
        constraints.arch_id,
        available as f64 / (1u64 << 30) as f64,
        request.ctx,
        "balanced"
    );
    println!(
        "calibration: t₀ {:.2} ms · k_cal {:.3} · R² {:.4} · n={}\n",
        calibration.t0_ms, calibration.k_cal, calibration.r2, calibration.n
    );

    if set.is_empty() {
        println!("no feasible plan.");
        if let Some(b) = set.binding_constraint() {
            println!("binding constraint: {b}");
        }
        return;
    }

    for (i, p) in set.plans.iter().enumerate() {
        println!(
            "{:>2}. {:<38} {:>5.0} tok/s  p10–p90 {:.0}–{:.0}   {:.1} GiB",
            i + 1,
            p.display,
            p.predicted.p50,
            p.predicted.p10,
            p.predicted.p90,
            p.peak_bytes as f64 / (1u64 << 30) as f64
        );
        for r in &p.rationale {
            println!("      why   {r}");
        }
        for r in &p.risks {
            println!("      risk  {r}");
        }
        if !p.fallbacks.is_empty() {
            println!("      fallback  {}", p.fallbacks.join(", "));
        }
        println!();
    }

    let fits: Vec<&Rejection> = set
        .rejected
        .iter()
        .filter(|r| matches!(r, Rejection::DoesNotFit { .. }))
        .collect();
    let estimated: Vec<&Rejection> = set
        .rejected
        .iter()
        .filter(|r| matches!(r, Rejection::NotPlannable { .. }))
        .collect();

    if !set.rejected.is_empty() {
        println!(
            "excluded: {} not plannable (size estimated) · {} do not fit",
            estimated.len(),
            fits.len()
        );
        for r in fits.iter().take(3) {
            println!("  {r}");
        }
    }

    // Objective sensitivity: the answer should change, or the flag is decoration.
    let quality = planner.plan(
        &PlanRequest { objective: Objective::MaxQuality, ..request.clone() },
        &constraints,
    );
    if let (Some(a), Some(b)) = (set.plans.first(), quality.plans.first()) {
        if a.entry_id != b.entry_id {
            println!(
                "\nobjective max_quality would choose: {} ({:.0} tok/s)",
                b.display, b.predicted.p50
            );
        }
    }
}
