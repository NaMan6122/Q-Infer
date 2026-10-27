//! The measurement pipeline, end to end, on whatever machine runs it.
//!
//! `cargo run --release -p qinfer-core --example measure`
//!
//! Order matters and is the point (PRD §5.1, low to high trust):
//!
//!   1. CPU bandwidth probe   — the sweep that found 1.93x on this box
//!   2. Micro-operations      — what the *device* achieves, not what a CPU copy
//!                              suggests it achieves (T-013)
//!   3. Fingerprint           — identity, plus the probe result
//!   4. Space                 — how much memory this machine will actually grant
//!                              a workload (T-012)
//!   5. Session               — catalogue + calibration + budget, resolved once
//!   6. Plans                 — ranked and explained
//!
//! Run it before and after a reboot: rung 4 should not move (a plan is not a
//! snapshot) while the "free now" figure does.

use qinfer_core::catalog::Catalog;
use qinfer_core::collect::{macos, SystemRunner};
use qinfer_core::microops::{self, OpId, Outcome};
use qinfer_core::planner::{Objective, PlanRequest};
use qinfer_core::probe::{probe, ProbeConfig};
use qinfer_core::session::Session;

const CATALOG: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.27.json");

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

fn main() {
    let runner = SystemRunner;

    // ---- 1. CPU bandwidth -------------------------------------------------
    println!("1. bandwidth probe");
    let bw = probe(&ProbeConfig { reps: 5, ..Default::default() });
    for (threads, gbs) in &bw.sweep {
        println!("     {threads:>3} threads  {gbs:>7.1} GB/s");
    }
    println!(
        "     single {:.1} · saturated {:.1} @{} threads · spread {:.1}%{}",
        bw.single_thread_gbs,
        bw.saturated_gbs,
        bw.best_threads,
        bw.spread_pct,
        if bw.spread_pct > qinfer_core::MAX_SPREAD_PCT {
            "  <- above the bar: this is not evidence"
        } else {
            ""
        }
    );

    // ---- 2. micro-operations ---------------------------------------------
    println!("\n2. micro-operations (device-side, via MLX)");
    let report = microops::run(&runner, &OpId::ALL);
    for op in &report.ops {
        match &op.outcome {
            Outcome::Measured { value, unit, reps, spread_pct, method } => println!(
                "     {:<24} {:>8.2} {:<8} {} reps, spread {:.1}%, {method}",
                op.id.label(),
                value,
                unit,
                reps,
                spread_pct
            ),
            Outcome::Refused { reason, fix } => {
                println!("     {:<24} -- {reason}", op.id.label());
                if let Some(f) = fix {
                    println!("        fix: {f}");
                }
            }
        }
    }

    // ---- 3. fingerprint ---------------------------------------------------
    println!("\n3. fingerprint");
    let fp = match macos::collect(&runner, &bw) {
        Ok(fp) => fp,
        Err(e) => {
            eprintln!("     cannot fingerprint this machine: {e}");
            std::process::exit(2);
        }
    };
    println!(
        "     {} · {} · {} performance levels · arch_id {}",
        fp.cpu.brand,
        fp.perf_summary(),
        fp.cpu.perf_levels.len(),
        fp.cpu.arch_id.as_deref().unwrap_or("unknown")
    );
    println!("     bandwidth provenance: {}", fp.provenance.get("bandwidth").map(String::as_str).unwrap_or("unknown"));

    // ---- 4. space ---------------------------------------------------------
    println!("\n4. memory budget");
    let space = qinfer_core::space::resolve(&fp, report.device.as_ref());
    println!("     {}", space.describe());
    let naive = fp.memory.total_bytes / 2;
    if space.plan_budget() != naive {
        println!(
            "     half-of-RAM would have said {:.1} GiB; the measured ceiling says {:.1} GiB ({}x)",
            gib(naive),
            gib(space.plan_budget()),
            space.plan_budget() as f64 / naive as f64
        );
    }
    if let (Some(t), Some(h)) = (report.prefill_tflops(), report.gpu_bandwidth_gbs()) {
        println!("     device: {h:.1} GB/s stream · {t:.1} TFLOP/s batched matmul");
    }

    // ---- 5. session -------------------------------------------------------
    let catalog = match Catalog::from_json(CATALOG) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\ncatalogue did not parse: {e}");
            std::process::exit(2);
        }
    };
    let session = match Session::with_device(fp, catalog, report.device.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\ncannot open a session: {e}");
            std::process::exit(3);
        }
    };
    println!("\n5. session");
    println!("     calibration: {}", session.calibration_summary());
    println!("     budget: {:.1} GiB", gib(session.available_bytes()));

    // ---- 6. plans ---------------------------------------------------------
    println!("\n6. plans (objective balanced, ctx 4096)\n");
    let set = session.plans(&PlanRequest { objective: Objective::Balanced, ctx: 4096, ..Default::default() });
    if set.is_empty() {
        println!("     no feasible plan");
        if let Some(b) = set.binding_constraint() {
            println!("     binding constraint: {b}");
        }
        return;
    }
    for (i, p) in set.plans.iter().enumerate() {
        println!(
            "  {:>2}. {:<40} {:>5.0} tok/s  p10-p90 {:.0}-{:.0}   {:.1} GiB",
            i + 1,
            p.display,
            p.predicted.p50,
            p.predicted.p10,
            p.predicted.p90,
            gib(p.peak_bytes)
        );
        for r in p.rationale.iter().take(2) {
            println!("        why  {r}");
        }
    }
    println!(
        "\n  excluded: {} not plannable (size estimated) · {} do not fit",
        set.rejected.iter().filter(|r| matches!(r, qinfer_core::Rejection::NotPlannable { .. })).count(),
        set.rejected.iter().filter(|r| matches!(r, qinfer_core::Rejection::DoesNotFit { .. })).count()
    );

    // ---- 7. the sweet spot ------------------------------------------------
    println!("\n7. sweet spot — capability against speed (T-014)");
    let spot = qinfer_core::sweetspot::frontier(&set, session.catalog());
    if spot.is_empty() {
        println!("     {}", spot.reason());
        return;
    }
    for (i, p) in spot.frontier.iter().enumerate() {
        let mark = if Some(i) == spot.knee { "   <- knee" } else { "" };
        println!(
            "     {:<38} {:>6.0} tok/s  cap {:.2}  {:>5.1} GiB  {}{mark}",
            p.plan.display,
            p.tps(),
            p.capability,
            gib(p.plan.peak_bytes),
            p.source.label()
        );
    }
    println!("\n     knee    {}", spot.reason());
    let floor = qinfer_core::sweetspot::DEFAULT_USABLE_TPS;
    println!("     usable  {}", spot.usable_reason(floor));
    for n in &spot.notes {
        println!("     note: {n}");
    }
}
