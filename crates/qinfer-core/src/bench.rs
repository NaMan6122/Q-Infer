//! Code-quality measurement (T-019).
//!
//! Two things sit behind this module, and they are the same number.
//!
//! **Quantisation damage.** The catalogue's `quality.value` is `None` for every
//! entry, so `planner::quality_placeholder` guesses from the quant label alone.
//! Scoring one model at two precisions on the same text replaces that guess with
//! a measurement. It also replaces it *usefully*: to detect a 5-point pass@1 drop
//! at 80% power needs roughly 1,090 HumanEval-sized problems per arm, so a
//! leaderboard of 164 items cannot answer the question a laptop can answer in
//! two seconds. Mean NLL over thousands of tokens has the resolution that pass@1
//! lacks.
//!
//! **Capability.** `sweetspot` ranks on `log10(total_params)` because it has
//! nothing better, and the note it prints says so. Mean NLL on code is a
//! measurement of the thing that axis is standing in for, and it is comparable
//! across models because the corpus is fixed.
//!
//! The honesty rule is carried in the type: every score records the **corpus
//! hash** it was measured against, and two scores are only comparable when those
//! hashes match. Changing the corpus invalidates every stored score, which is
//! why the corpus is checked in and frozen.
//!
//! What this is not: instruction-following, agentic ability, or a benchmark
//! score. It is next-token prediction on code, and surfaces must say so.

use crate::collect::Runner;
use crate::engines::Argv;

/// The probe program. One `python3` subprocess, never imported.
pub const PROBE_SCRIPT: &str = include_str!("bench_probe.py");

#[derive(Debug, Clone, PartialEq)]
pub struct CodeScore {
    /// What was scored: an artifact path or a Hub repo id.
    pub model: String,
    /// Identifies the text. Two scores are comparable only if this matches.
    pub corpus_sha: String,
    pub tokens: u32,
    /// Mean negative log-likelihood per token. **Lower is better.**
    pub mean_nll: f64,
    pub perplexity: f64,
    pub load_s: f64,
    pub forward_s: f64,
}

impl CodeScore {
    /// True when two scores were measured on the same text and may be compared.
    pub fn comparable_with(&self, other: &CodeScore) -> bool {
        self.corpus_sha == other.corpus_sha
    }

    /// Relative capability improvement of `self` over `other`, at equal corpus.
    ///
    /// `None` when the corpora differ, because subtracting two numbers measured
    /// on different text is arithmetic without meaning.
    pub fn improvement_over(&self, other: &CodeScore) -> Option<f64> {
        if !self.comparable_with(other) || other.mean_nll <= 0.0 {
            return None;
        }
        Some((other.mean_nll - self.mean_nll) / other.mean_nll * 100.0)
    }

    /// The catalogue's `quality.value`, in the units the planner can consume.
    ///
    /// Perplexity is what a reader expects; NLL is what the model computes. The
    /// pair travels together so neither can be quoted without the other.
    pub fn describe(&self) -> String {
        format!(
            "{:.3} nats/token (perplexity {:.2}) over {} tokens, corpus {}",
            self.mean_nll, self.perplexity, self.tokens, self.corpus_sha
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BenchError {
    /// The probe could not run at all (no python, no MLX).
    NotRun { detail: String },
    /// The probe ran and reported a problem.
    Reported { detail: String },
    /// The probe produced something that is not a measurement.
    Unusable { detail: String },
}

impl BenchError {
    /// Always actionable: "no" without "do this instead" is the error style
    /// PRD §10.5 forbids.
    pub fn fix(&self) -> &'static str {
        match self {
            BenchError::NotRun { .. } => {
                "install MLX for the interpreter that runs the probe: `pip install mlx mlx-lm`"
            }
            BenchError::Reported { .. } => {
                "check the model runs at all: `python3 -m mlx_lm.generate --model <path> --prompt hi`"
            }
            BenchError::Unusable { .. } => "re-run the probe; if it repeats, the model is not scoreable",
        }
    }
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::NotRun { detail } => write!(f, "the code probe did not run: {detail}"),
            BenchError::Reported { detail } => write!(f, "the code probe reported: {detail}"),
            BenchError::Unusable { detail } => write!(f, "not a measurement: {detail}"),
        }
    }
}

pub fn argv(model: &str) -> Argv {
    Argv {
        program: "python3".into(),
        args: vec!["-c".into(), PROBE_SCRIPT.into(), model.to_string()],
    }
}

/// Score one model. A probe that cannot start is an error, never a zero.
pub fn run(runner: &impl Runner, model: &str) -> Result<CodeScore, BenchError> {
    let a = argv(model);
    let args: Vec<&str> = a.args.iter().map(String::as_str).collect();
    let out = runner.run(&a.program, &args).ok_or_else(|| BenchError::NotRun {
        detail: format!("`{}` produced no output", a.render()),
    })?;
    parse(&out, model)
}

/// Parse the probe's JSON. Pure, so every failure path is testable without MLX.
pub fn parse(json: &str, model: &str) -> Result<CodeScore, BenchError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| BenchError::Unusable {
        detail: format!("probe output is not JSON ({e})"),
    })?;

    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(BenchError::Reported { detail: err.to_string() });
    }

    let corpus_sha = v
        .get("corpus_sha")
        .and_then(|x| x.as_str())
        .ok_or_else(|| BenchError::Unusable {
            detail: "no corpus hash — the score could never be compared to anything".into(),
        })?
        .to_string();

    let num = |key: &str| v.get(key).and_then(|x| x.as_f64());

    let mean_nll = num("mean_nll").ok_or_else(|| BenchError::Unusable {
        detail: "no mean_nll in the payload".into(),
    })?;
    // A zero or negative NLL is not a very good model; it is a broken probe.
    if !mean_nll.is_finite() || mean_nll <= 0.0 {
        return Err(BenchError::Unusable { detail: format!("mean_nll is {mean_nll}") });
    }

    let tokens = v.get("tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    if tokens < 32 {
        return Err(BenchError::Unusable {
            detail: format!("only {tokens} tokens scored — too few to mean anything"),
        });
    }

    Ok(CodeScore {
        model: model.to_string(),
        corpus_sha,
        tokens,
        mean_nll,
        perplexity: num("perplexity").unwrap_or_else(|| mean_nll.exp()),
        load_s: num("load_s").unwrap_or(0.0),
        forward_s: num("forward_s").unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from the reference machine (Qwen2.5-3B-Instruct-4bit).
    const LIVE: &str = r#"{"corpus_sha":"599e9c50dff219e0","tokens":422,"mean_nll":0.982607,"perplexity":2.6714,"load_s":1.201,"forward_s":0.159,"dtype":"Device(gpu, 0)"}"#;

    #[test]
    fn a_live_payload_parses() {
        let s = parse(LIVE, "mlx-community/Qwen2.5-3B-Instruct-4bit").expect("score");
        assert_eq!(s.tokens, 422);
        assert!((s.mean_nll - 0.982607).abs() < 1e-9);
        assert!((s.perplexity - 2.6714).abs() < 1e-4);
        assert_eq!(s.corpus_sha, "599e9c50dff219e0");
        assert!(s.describe().contains("corpus 599e9c50dff219e0"));
    }

    #[test]
    fn a_reported_error_is_carried_with_its_fix() {
        let err = parse(r#"{"corpus_sha":"x","error":"ModuleNotFoundError: mlx_lm"}"#, "m")
            .unwrap_err();
        assert!(matches!(err, BenchError::Reported { .. }));
        assert!(err.to_string().contains("mlx_lm"));
        // A `Reported` failure means the probe *ran*, so the fix is to check the
        // model, not to install MLX — which is what `NotRun` prescribes.
        assert!(
            err.fix().contains("check the model runs"),
            "wrong fix for a reported error: {}",
            err.fix()
        );
    }

    #[test]
    fn each_failure_class_prescribes_its_own_fix() {
        let not_run = BenchError::NotRun { detail: "no python3".into() };
        let reported = BenchError::Reported { detail: "bad model".into() };
        let unusable = BenchError::Unusable { detail: "nonsense".into() };
        assert!(not_run.fix().contains("pip install"));
        assert_ne!(not_run.fix(), reported.fix());
        assert_ne!(reported.fix(), unusable.fix());
        for e in [not_run, reported, unusable] {
            assert!(!e.fix().trim().is_empty(), "every failure needs an action");
        }
    }

    #[test]
    fn a_score_without_a_corpus_hash_is_refused() {
        // Without the hash there is no way to know what the number was measured
        // against, so it could be compared to anything and mean nothing.
        let err = parse(r#"{"tokens":422,"mean_nll":0.98}"#, "m").unwrap_err();
        assert!(matches!(err, BenchError::Unusable { .. }));
        assert!(err.to_string().contains("corpus hash"));
    }

    #[test]
    fn a_zero_or_negative_nll_is_not_a_measurement() {
        for bad in ["0.0", "-1.0"] {
            let j = format!(r#"{{"corpus_sha":"x","tokens":422,"mean_nll":{bad}}}"#);
            assert!(parse(&j, "m").is_err(), "mean_nll {bad} must be refused");
        }
    }

    #[test]
    fn too_few_tokens_is_refused() {
        let j = r#"{"corpus_sha":"x","tokens":8,"mean_nll":0.9}"#;
        assert!(parse(j, "m").unwrap_err().to_string().contains("too few"));
    }

    #[test]
    fn malformed_output_is_unusable_not_a_panic() {
        assert!(matches!(parse("Traceback...", "m"), Err(BenchError::Unusable { .. })));
        assert!(matches!(parse("", "m"), Err(BenchError::Unusable { .. })));
    }

    #[test]
    fn scores_are_only_comparable_on_the_same_corpus() {
        let a = parse(LIVE, "a").unwrap();
        let mut b = parse(LIVE, "b").unwrap();
        assert!(a.comparable_with(&b));
        assert!(a.improvement_over(&b).is_some());

        // Different corpus: the numbers exist but the comparison does not.
        b.corpus_sha = "different".into();
        assert!(!a.comparable_with(&b));
        assert_eq!(a.improvement_over(&b), None, "arithmetic on different text has no meaning");
    }

    #[test]
    fn improvement_is_signed_and_oriented_correctly() {
        let good = parse(LIVE, "good").unwrap();
        let mut worse = parse(LIVE, "worse").unwrap();
        worse.mean_nll = 1.2;
        // `good` is better, so it must show a positive improvement over `worse`.
        let d = good.improvement_over(&worse).expect("comparable");
        assert!(d > 0.0, "lower nll must read as improvement, got {d}");
        assert!(worse.improvement_over(&good).unwrap() < 0.0);
    }

    #[test]
    fn argv_passes_the_model_through() {
        let a = argv("mlx-community/x");
        assert_eq!(a.program, "python3");
        assert_eq!(a.args.last().map(String::as_str), Some("mlx-community/x"));
    }
}
