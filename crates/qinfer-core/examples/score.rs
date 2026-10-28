//! Score a model on the fixed code corpus and record the result in the
//! catalogue.
//!
//! `cargo run --release -p qinfer-core --example score -- <catalogue.json> <entry-id> <model>`
//!
//! This is the write end of T-019. Until a score lands in `catalogue.quality`,
//! `sweetspot` has nothing to rank on but `log10(total_params)` and says so in a
//! note; the moment one does, the frontier switches to a measured axis and names
//! the candidates it had to leave out.
//!
//! The score is written into the catalogue rather than a sidecar file so it is
//! reviewed, diffed and pinned with the artifact it describes (ADR-0004).

use qinfer_core::bench::{self, MODE_RAW};
use qinfer_core::catalog::Catalog;
use qinfer_core::collect::SystemRunner;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 || args.len() > 4 {
        eprintln!(
            "usage: score <catalogue.json> <entry-id> <model> [raw|instruction]"
        );
        std::process::exit(2);
    }
    let (path, entry_id, model) = (&args[0], &args[1], &args[2]);
    let mode = args.get(3).map(String::as_str).unwrap_or(MODE_RAW);

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let mut catalog = match Catalog::from_json(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{path} is not a valid catalogue: {e}");
            std::process::exit(2);
        }
    };
    if let Err(v) = catalog.validate() {
        eprintln!(
            "{path} is invalid before we start: {}",
            v.first().map(|x| x.to_string()).unwrap_or_default()
        );
        std::process::exit(2);
    }

    // Check the entry exists *before* paying for a model load: a typo in the id
    // should cost a second, not a two-minute scoring run.
    if !catalog.entries.iter().any(|e| e.id == *entry_id) {
        eprintln!("no entry `{entry_id}` in {path}");
        std::process::exit(3);
    }

    println!("scoring {model} for entry `{entry_id}` ({mode}) ...");
    let score = match bench::run(&SystemRunner, model, mode) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("fix: {}", e.fix());
            std::process::exit(4);
        }
    };

    // Record the per-byte figure, not the per-token one. The token count varies
    // by tokeniser (422 for Qwen2.5 on this corpus, 464 for Qwen3), so a stored
    // per-token score would make two entries look comparable when they are not.
    let method = format!(
        "bench@{}tokens/{}bytes corpus {}",
        score.tokens, score.corpus_bytes, score.corpus_sha
    );
    // The metric name carries the mode, so a raw score and an instruction
    // score can never be mistaken for one another in the catalogue.
    let metric = if score.mode == bench::MODE_INSTRUCTION {
        "code_instr_nll_per_byte"
    } else {
        "code_nll_per_byte"
    };
    if !catalog.record_quality(entry_id, metric, score.nll_per_byte, &method) {
        eprintln!("could not record the score for `{entry_id}`");
        std::process::exit(5);
    }

    match catalog.to_json_pretty() {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json + "\n") {
                eprintln!("cannot write {path}: {e}");
                std::process::exit(6);
            }
        }
        Err(e) => {
            eprintln!("cannot serialise the catalogue: {e}");
            std::process::exit(6);
        }
    }

    println!("  {}", score.describe());
    println!("  recorded in {path} as `{metric}` ({method})");
}
