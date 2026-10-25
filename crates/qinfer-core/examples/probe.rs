//! Print a bandwidth probe for this machine.
//!
//! `cargo run --release -p qinfer-core --example probe`
//!
//! Exists so the Rust probe can be compared against the Python one it replaces
//! (`spikes/m0/fingerprint.py`) on the same hardware. If the two disagree by
//! more than ~10%, one of them is measuring the wrong thing.

use qinfer_core::probe::{probe, validate_config, ProbeConfig};

fn main() {
    let cfg = ProbeConfig::default();

    if let Err(e) = validate_config(&cfg, None) {
        eprintln!("probe refused: {e}");
        std::process::exit(2);
    }

    println!(
        "buffer {} MiB · {} reps · sweeping {:?} threads\n",
        cfg.buffer_bytes / (1024 * 1024),
        cfg.reps,
        cfg.thread_counts
    );

    let r = probe(&cfg);

    for (threads, gbs) in &r.sweep {
        let marker = if *threads == r.best_threads { "  <= saturated" } else { "" };
        println!("{threads:>3} threads  {gbs:>7.1} GB/s{marker}");
    }

    println!();
    println!("single-thread  {:.1} GB/s", r.single_thread_gbs);
    println!("saturated      {:.1} GB/s @{} threads", r.saturated_gbs, r.best_threads);
    if r.single_thread_gbs > 0.0 {
        println!("gain           {:.2}x", r.saturated_gbs / r.single_thread_gbs);
    }
    println!("spread         {:.1}%", r.spread_pct);

    if r.spread_pct > qinfer_core::MAX_SPREAD_PCT {
        println!(
            "\nwarning: spread above {}% — publish provenance as `unknown`, not `measured`",
            qinfer_core::MAX_SPREAD_PCT
        );
    }
}
