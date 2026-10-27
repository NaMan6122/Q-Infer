//! Score a model's code quality on the fixed corpus.
//!
//! `cargo run --release -p qinfer-core --example bench -- <model> [<model> ...]`
//!
//! Mean negative log-likelihood per token on checked-in code. Lower is better,
//! and two scores are only comparable when their corpus hashes match — which is
//! why the comparison below refuses to print one otherwise.
//!
//! `mlx` artifacts only, for now: `mlx_lm` scores them directly. Scoring a GGUF
//! needs `llama-perplexity`, which is a second adapter and not yet written — so
//! the GGUF half of the catalogue still has no measured quality.

use qinfer_core::bench::{run, CodeScore, MODE_RAW};
use qinfer_core::collect::SystemRunner;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--mode instruction` scores the assistant span of a coding request
    // instead of raw continuation. The two corpora hash differently, so the
    // comparison block below will refuse to mix them.
    let mut mode = MODE_RAW.to_string();
    if let Some(i) = args.iter().position(|a| a == "--mode") {
        if i + 1 < args.len() {
            mode = args[i + 1].clone();
            args.drain(i..i + 2);
        }
    }
    let models: Vec<String> = args;
    if models.is_empty() {
        eprintln!("usage: bench [--mode raw|instruction] <model-path-or-hub-repo> [<model> ...]");
        std::process::exit(2);
    }

    let runner = SystemRunner;
    let mut scores: Vec<CodeScore> = Vec::new();

    for m in &models {
        match run(&runner, m, &mode) {
            Ok(s) => {
                println!("  {:<44} {}", m, s.describe());
                scores.push(s);
            }
            Err(e) => {
                println!("  {:<44} -- {e}", m);
                println!("  {:<44}    fix: {}", "", e.fix());
            }
        }
    }

    if scores.len() < 2 {
        if let Some(only) = scores.first() {
            println!("\n  one score is not a comparison; run it against another precision of the same model");
            println!("  to see what quantisation costs on code: {}", only.corpus_sha);
        }
        return;
    }

    println!("\n  comparisons (same corpus only):");
    let base = &scores[0];
    let mut printed = false;
    for s in scores.iter().skip(1) {
        match s.improvement_over(base) {
            Some(d) if d > 0.0 => {
                println!("  {} is {d:.1}% better on code than {}", s.model, base.model);
                printed = true;
            }
            Some(d) => {
                println!("  {} is {:.1}% worse on code than {}", s.model, -d, base.model);
                printed = true;
            }
            None => {
                println!(
                    "  {} and {} were scored on different corpora — not comparable",
                    s.model, base.model
                );
                printed = true;
            }
        }
    }
    if !printed {
        println!("  nothing comparable");
    }
}
