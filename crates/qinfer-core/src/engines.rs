//! Engine adapters (T-011, ADR-0008).
//!
//! ADR-0001: the core never links a runtime. An engine is a *subprocess* that
//! this module knows how to invoke, probe and parse. Nothing here imports
//! Python, MLX or llama.cpp.
//!
//! Why a second adapter earns its place: GGUF/llama.cpp is universal, but the
//! product's premise is to get closer to the silicon. MLX is Apple's native
//! path and is usually faster on Metal — *usually*, and the difference is
//! 5–30%, not multiples, because decode is bandwidth-bound either way. So the
//! planner must be able to try both and let the measurement decide.
//!
//! `k_cal` is keyed per backend precisely because these two do not extract the
//! same fraction of peak bandwidth. A coefficient measured under llama.cpp is
//! not valid under MLX.

use crate::collect::Runner;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Engine {
    LlamaCpp,
    Mlx,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::LlamaCpp => "llama.cpp",
            Engine::Mlx => "mlx",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Argv {
    pub program: String,
    pub args: Vec<String>,
}

impl Argv {
    pub fn render(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.args {
            s.push(' ');
            if a.contains(' ') {
                s.push('"');
                s.push_str(a);
                s.push('"');
            } else {
                s.push_str(a);
            }
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct BenchRequest {
    /// Path or repo id the engine understands.
    pub model: String,
    pub prompt_tokens: u32,
    pub gen_tokens: u32,
    pub reps: u32,
}

impl Default for BenchRequest {
    fn default() -> Self {
        BenchRequest { model: String::new(), prompt_tokens: 512, gen_tokens: 128, reps: 5 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchResult {
    pub decode_tps: f64,
    pub prefill_tps: Option<f64>,
    pub engine: Engine,
}

pub trait Adapter {
    fn engine(&self) -> Engine;
    /// Artifact formats this engine can execute, as used by `catalog.artifact.format`.
    fn formats(&self) -> &'static [&'static str];
    /// The command that proves the engine is installed.
    fn probe(&self) -> Argv;
    fn bench(&self, req: &BenchRequest) -> Argv;
    /// Parse the engine's own output. Returns `None` rather than a made-up
    /// number when the output does not look like a measurement.
    fn parse(&self, stdout: &str) -> Option<BenchResult>;
}

// ---------------------------------------------------------------------------
// llama.cpp — GGUF, the universal path
// ---------------------------------------------------------------------------

pub struct LlamaCpp;

impl Adapter for LlamaCpp {
    fn engine(&self) -> Engine {
        Engine::LlamaCpp
    }

    fn formats(&self) -> &'static [&'static str] {
        &["gguf"]
    }

    fn probe(&self) -> Argv {
        Argv { program: "llama-bench".into(), args: vec!["--version".into()] }
    }

    fn bench(&self, req: &BenchRequest) -> Argv {
        Argv {
            program: "llama-bench".into(),
            args: vec![
                "-m".into(), req.model.clone(),
                "-p".into(), req.prompt_tokens.to_string(),
                "-n".into(), req.gen_tokens.to_string(),
                "-r".into(), req.reps.to_string(),
                "-ngl".into(), "99".into(),
                "-o".into(), "json".into(),
            ],
        }
    }

    fn parse(&self, stdout: &str) -> Option<BenchResult> {
        #[derive(Deserialize)]
        struct Row {
            n_gen: u32,
            avg_ts: f64,
        }

        let rows: Vec<Row> = serde_json::from_str(stdout.trim()).ok()?;
        let decode = rows.iter().find(|r| r.n_gen > 0)?.avg_ts;
        let prefill = rows.iter().find(|r| r.n_gen == 0).map(|r| r.avg_ts);
        if !decode.is_finite() || decode <= 0.0 {
            return None;
        }
        Some(BenchResult { decode_tps: decode, prefill_tps: prefill, engine: Engine::LlamaCpp })
    }
}

// ---------------------------------------------------------------------------
// MLX — Apple's native path
// ---------------------------------------------------------------------------

pub struct Mlx;

impl Adapter for Mlx {
    fn engine(&self) -> Engine {
        Engine::Mlx
    }

    fn formats(&self) -> &'static [&'static str] {
        &["mlx"]
    }

    fn probe(&self) -> Argv {
        // Import both: mlx-lm can be installed without a working Metal device.
        Argv {
            program: "python3".into(),
            args: vec![
                "-c".into(),
                "import mlx.core, mlx_lm; print('ok')".into(),
            ],
        }
    }

    fn bench(&self, req: &BenchRequest) -> Argv {
        // `mlx_lm.generate` reports its own tokens-per-second lines, which are
        // the measurement we parse. Prompt length is padded to make runs
        // comparable across models.
        let prompt = "The quick brown fox jumps over the lazy dog. ".repeat(1 + req.prompt_tokens as usize / 9);
        Argv {
            program: "python3".into(),
            args: vec![
                "-m".into(), "mlx_lm.generate".into(),
                "--model".into(), req.model.clone(),
                "--prompt".into(), prompt,
                "--max-tokens".into(), req.gen_tokens.to_string(),
                "--temp".into(), "0".into(),
            ],
        }
    }

    fn parse(&self, stdout: &str) -> Option<BenchResult> {
        // Two known shapes:
        //   "Prompt: 512 tokens, 1234.567 tokens-per-sec"
        //   "Generation: 128 tokens, 45.678 tokens-per-sec"
        let mut decode = None;
        let mut prefill = None;

        for line in stdout.lines() {
            let line = line.trim();
            let Some((label, rest)) = line.split_once(':') else { continue };
            if !rest.contains("tokens-per-sec") {
                continue;
            }
            let Some(tps) = rest
                .split(',')
                .find_map(|part| part.trim().trim_end_matches("tokens-per-sec").trim().parse::<f64>().ok())
            else {
                continue;
            };
            match label.trim() {
                "Generation" => decode = Some(tps),
                "Prompt" => prefill = Some(tps),
                _ => {}
            }
        }

        let decode = decode?;
        if !decode.is_finite() || decode <= 0.0 {
            return None;
        }
        Some(BenchResult { decode_tps: decode, prefill_tps: prefill, engine: Engine::Mlx })
    }
}

// ---------------------------------------------------------------------------
// Availability
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Availability {
    pub engine: Engine,
    pub available: bool,
    pub detail: String,
}

/// Probe every engine through the injected runner, so this is testable without
/// either runtime installed.
pub fn detect(runner: &impl Runner) -> Vec<Availability> {
    let adapters: [&dyn Adapter; 2] = [&LlamaCpp, &Mlx];
    adapters
        .iter()
        .map(|a| {
            let argv = a.probe();
            let args: Vec<&str> = argv.args.iter().map(String::as_str).collect();
            match runner.run(&argv.program, &args) {
                Some(out) => Availability {
                    engine: a.engine(),
                    available: true,
                    detail: out.lines().next().unwrap_or("installed").to_string(),
                },
                None => Availability {
                    engine: a.engine(),
                    available: false,
                    detail: format!("`{}` not found or failed", argv.render()),
                },
            }
        })
        .collect()
}

/// The adapter that can run this artifact format, if any.
pub fn adapter_for_format(format: &str) -> Option<&'static dyn Adapter> {
    match format {
        "gguf" => Some(&LlamaCpp),
        "mlx" => Some(&Mlx),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::Runner;
    use std::collections::BTreeMap;

    const LLAMACPP_OUT: &str = r#"[
      {"n_prompt":512,"n_gen":0,"avg_ts":2798.3,"stddev_ts":12.1},
      {"n_prompt":0,"n_gen":128,"avg_ts":108.75,"stddev_ts":2.6}
    ]"#;

    const MLX_OUT: &str = "\
Prompt: 512 tokens, 1234.567 tokens-per-sec
Generation: 128 tokens, 45.678 tokens-per-sec
Peak memory: 3.456 GB
";

    #[test]
    fn llamacpp_parses_its_json() {
        let r = LlamaCpp.parse(LLAMACPP_OUT).expect("parse");
        assert_eq!(r.decode_tps, 108.75);
        assert_eq!(r.prefill_tps, Some(2798.3));
        assert_eq!(r.engine, Engine::LlamaCpp);
    }

    #[test]
    fn llamacpp_refuses_output_it_does_not_understand() {
        assert!(LlamaCpp.parse("").is_none());
        assert!(LlamaCpp.parse("not json").is_none());
        assert!(LlamaCpp.parse(r#"[{"n_prompt":512,"n_gen":0,"avg_ts":10.0}]"#).is_none(), "no decode row");
    }

    #[test]
    fn llamacpp_refuses_a_non_positive_measurement() {
        assert!(LlamaCpp.parse(r#"[{"n_prompt":0,"n_gen":128,"avg_ts":0.0}]"#).is_none());
        assert!(LlamaCpp.parse(r#"[{"n_prompt":0,"n_gen":128,"avg_ts":-5.0}]"#).is_none());
    }

    #[test]
    fn mlx_parses_its_tokens_per_sec_lines() {
        let r = Mlx.parse(MLX_OUT).expect("parse");
        assert_eq!(r.decode_tps, 45.678);
        assert_eq!(r.prefill_tps, Some(1234.567));
        assert_eq!(r.engine, Engine::Mlx);
    }

    #[test]
    fn mlx_refuses_output_without_a_generation_line() {
        assert!(Mlx.parse("").is_none());
        assert!(Mlx.parse("Prompt: 512 tokens, 1234.5 tokens-per-sec").is_none());
        assert!(Mlx.parse("Traceback (most recent call last):\nImportError: no module named mlx_lm").is_none());
    }

    #[test]
    fn each_adapter_claims_only_the_format_it_can_run() {
        assert!(LlamaCpp.formats().contains(&"gguf"));
        assert!(!LlamaCpp.formats().contains(&"mlx"));
        assert!(Mlx.formats().contains(&"mlx"));
        assert!(!Mlx.formats().contains(&"gguf"));

        assert!(adapter_for_format("gguf").is_some());
        assert!(adapter_for_format("mlx").is_some());
        assert!(adapter_for_format("safetensors").is_none());
        assert!(adapter_for_format("qnn-bundle").is_none(), "bundle runtimes are a separate class (ADR-0008)");
    }

    #[test]
    fn bench_commands_carry_the_parameters_that_make_runs_comparable() {
        let req = BenchRequest { model: "/models/x.gguf".into(), prompt_tokens: 512, gen_tokens: 128, reps: 5 };
        let argv = LlamaCpp.bench(&req);
        assert!(argv.args.contains(&"/models/x.gguf".to_string()));
        assert!(argv.args.windows(2).any(|w| w == ["-p", "512"]));
        assert!(argv.args.windows(2).any(|w| w == ["-n", "128"]));
        assert!(argv.args.windows(2).any(|w| w == ["-r", "5"]));
        assert!(argv.args.windows(2).any(|w| w == ["-o", "json"]));

        let argv = Mlx.bench(&req);
        assert_eq!(argv.program, "python3");
        assert!(argv.args.windows(2).any(|w| w == ["--model", "/models/x.gguf"]));
        assert!(argv.args.windows(2).any(|w| w == ["--max-tokens", "128"]));
    }

    struct Fake(BTreeMap<String, String>);

    impl Runner for Fake {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            self.0.get(&format!("{} {}", program, args.join(" "))).cloned()
        }
    }

    #[test]
    fn detection_reports_absence_rather_than_assuming() {
        let none = Fake(BTreeMap::new());
        let found = detect(&none);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|a| !a.available));
        assert!(found.iter().any(|a| a.detail.contains("llama-bench")));
    }

    #[test]
    fn detection_reports_presence() {
        let mut m = BTreeMap::new();
        m.insert(
            "llama-bench --version".to_string(),
            "version: 0.7.0 (build 11669)".to_string(),
        );
        let found = detect(&Fake(m));
        let lc = found.iter().find(|a| a.engine == Engine::LlamaCpp).unwrap();
        assert!(lc.available);
        assert!(lc.detail.contains("0.7.0"));
        let mlx = found.iter().find(|a| a.engine == Engine::Mlx).unwrap();
        assert!(!mlx.available, "mlx must not be assumed present");
    }

    #[test]
    fn argv_rendering_is_readable_and_quoted() {
        let argv = Argv { program: "x".into(), args: vec!["a".into(), "b c".into()] };
        assert_eq!(argv.render(), "x a \"b c\"");
    }
}
