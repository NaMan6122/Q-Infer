//! Command-line surface (T-007).
//!
//! Every command has two renderings: human, and a single versioned JSON
//! envelope (`qinfer/cli@1`). The envelope is the contract — scripts and CI
//! consume it, and nothing in it may change without a major bump.
//!
//! Exit codes are stable and documented (PRD §10.5). `plan` distinguishes
//! "this machine cannot run it" (2, unsupported) from "it does not fit" (3),
//! because those need different actions from the user.

use qinfer_core::catalog::Catalog;
use qinfer_core::planner::{Objective, PlanRequest, PlanSet, Rejection};
use qinfer_core::{Session, Workspace};

pub const SCHEMA: &str = "qinfer/cli@1";

/// The catalogue ships with the binary; a network fetch belongs to M1.
const CATALOG_JSON: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.26.json");

pub mod exit {
    pub const OK: i32 = 0;
    /// The machine is detected but cannot run what was asked for.
    pub const UNSUPPORTED: i32 = 2;
    /// Nothing fits in the available memory.
    pub const NO_FIT: i32 = 3;
    /// Provisioning or environment failure.
    pub const FAILED: i32 = 6;
    /// The command line itself was wrong.
    pub const USAGE: i32 = 64;
    /// A spec failed its structural gate (T-009).
    pub const SPEC_INVALID: i32 = 5;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Detect,
    Plan,
    SpecLint,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub command: Command,
    pub json: bool,
    pub offline: bool,
    pub family: Option<String>,
    pub ctx: u32,
    pub objective: Objective,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    UnknownCommand(String),
    UnknownFlag(String),
    MissingValue(String),
    BadValue { flag: String, value: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnknownCommand(c) => write!(f, "unknown command {c:?}"),
            ParseError::UnknownFlag(f_) => write!(f, "unknown flag {f_:?}"),
            ParseError::MissingValue(flag) => write!(f, "{flag} needs a value"),
            ParseError::BadValue { flag, value } => write!(f, "{flag} does not accept {value:?}"),
        }
    }
}

pub fn parse(args: &[String]) -> Result<Parsed, ParseError> {
    let mut it = args.iter();
    let command = match it.next().map(String::as_str) {
        Some("detect") => Command::Detect,
        Some("plan") => Command::Plan,
        Some("spec-lint") => Command::SpecLint,
        Some("help") | Some("--help") | Some("-h") => Command::Help,
        Some("version") | Some("--version") | Some("-V") => Command::Version,
        Some(other) => return Err(ParseError::UnknownCommand(other.to_string())),
        None => return Err(ParseError::UnknownCommand(String::new())),
    };

    let mut parsed = Parsed {
        command,
        json: false,
        offline: false,
        family: None,
        ctx: 4096,
        objective: Objective::Balanced,
    };

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => parsed.json = true,
            "--offline" => parsed.offline = true,
            "--model" => {
                let v = it.next().ok_or_else(|| ParseError::MissingValue("--model".into()))?;
                parsed.family = Some(v.clone());
            }
            "--ctx" => {
                let v = it.next().ok_or_else(|| ParseError::MissingValue("--ctx".into()))?;
                parsed.ctx = v
                    .parse()
                    .map_err(|_| ParseError::BadValue { flag: "--ctx".into(), value: v.clone() })?;
            }
            "--objective" => {
                let v = it.next().ok_or_else(|| ParseError::MissingValue("--objective".into()))?;
                parsed.objective = Objective::parse(v)
                    .ok_or_else(|| ParseError::BadValue { flag: "--objective".into(), value: v.clone() })?;
            }
            other => return Err(ParseError::UnknownFlag(other.to_string())),
        }
    }

    Ok(parsed)
}

/// The one shape every command emits under `--json`.
pub fn envelope(command: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA,
        "command": command,
        "ok": true,
        "data": data,
        "error": serde_json::Value::Null,
    })
}

pub fn error_envelope(command: &str, code: i32, message: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA,
        "command": command,
        "ok": false,
        "data": serde_json::Value::Null,
        "error": { "exit": code, "message": message },
    })
}

/// Which exit code a failed plan deserves. "Cannot run here" and "does not
/// fit" are different problems and must not collapse into one.
pub fn exit_for(set: &PlanSet) -> i32 {
    if !set.plans.is_empty() {
        return exit::OK;
    }
    let unsupported = set.rejected.iter().all(|r| matches!(r, Rejection::NoCoverage { .. }));
    let any_fit = set.rejected.iter().any(|r| matches!(r, Rejection::DoesNotFit { .. }));
    if any_fit {
        exit::NO_FIT
    } else if unsupported && !set.rejected.is_empty() {
        exit::UNSUPPORTED
    } else {
        exit::NO_FIT
    }
}

pub fn run(args: &[String]) -> i32 {
    let parsed = match parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("qinfer: {e}");
            eprintln!("try `qinfer help`");
            return exit::USAGE;
        }
    };

    match parsed.command {
        Command::Help => {
            print_help();
            exit::OK
        }
        Command::Version => {
            if parsed.json {
                println!("{}", envelope("version", serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })));
            } else {
                println!("qinfer {}", env!("CARGO_PKG_VERSION"));
            }
            exit::OK
        }
        Command::Detect => cmd_detect(&parsed),
        Command::Plan => cmd_plan(&parsed),
        Command::SpecLint => cmd_spec_lint(&parsed),
    }
}

/// Mechanises G1 across the repository's own specs (T-009).
fn cmd_spec_lint(parsed: &Parsed) -> i32 {
    use qinfer_core::spec::{lint_adr, lint_milestone, lint_tasks, LintSummary, Severity};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut findings = Vec::new();

    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).ok();

    if let Some(md) = read(root.join("TASKS.md")) {
        findings.extend(lint_tasks(&md, "TASKS.md"));
    }
    for entry in std::fs::read_dir(root.join("docs/adr")).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(md) = read(path.clone()) {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("adr").to_string();
                if name != "README.md" {
                    findings.extend(lint_adr(&md, &name));
                }
            }
        }
    }
    for entry in std::fs::read_dir(root.join("docs/milestones")).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("m").to_string();
            if name == "README.md" {
                continue;
            }
            if let Some(md) = read(path.clone()) {
                findings.extend(lint_milestone(&md, &name));
            }
        }
    }

    let summary = LintSummary::from(findings);
    let code = if summary.is_clean() { exit::OK } else { exit::SPEC_INVALID };

    if parsed.json {
        let data = serde_json::json!({
            "errors": summary.errors,
            "warnings": summary.warnings,
            "findings": summary.findings.iter().map(|f| serde_json::json!({
                "file": f.file, "line": f.line, "rule": f.code(),
                "severity": match f.severity { Severity::Error => "error", Severity::Warning => "warning" },
                "message": f.message,
            })).collect::<Vec<_>>(),
        });
        if summary.is_clean() {
            println!("{}", envelope("spec-lint", data));
        } else {
            println!("{}", serde_json::json!({
                "schema": SCHEMA, "command": "spec-lint", "ok": false,
                "data": data, "error": { "exit": code, "message": format!("{} spec error(s)", summary.errors) }
            }));
        }
        return code;
    }

    for f in &summary.findings {
        let sev = match f.severity { Severity::Error => "error  ", Severity::Warning => "warning" };
        println!("{sev} {} {}:{} {}", f.code(), f.file, f.line, f.message);
    }
    println!(
        "\n{} error(s), {} warning(s) across TASKS.md, docs/adr, docs/milestones",
        summary.errors, summary.warnings
    );
    code
}

fn load_workspace() -> Workspace {
    Workspace::discover()
}

fn cmd_detect(parsed: &Parsed) -> i32 {
    let ws = load_workspace();
    let Some(fp) = ws.fingerprint.as_ref() else {
        let msg = ws.error.clone().unwrap_or_else(|| "no fingerprint found".into());
        if parsed.json {
            println!("{}", error_envelope("detect", exit::UNSUPPORTED, &msg));
        } else {
            eprintln!("qinfer: {msg}");
            eprintln!("run: python3 spikes/m0/fingerprint.py");
        }
        return exit::UNSUPPORTED;
    };

    if parsed.json {
        let data = serde_json::json!({
            "arch_id": fp.cpu.arch_id.clone()
                .unwrap_or_else(|| qinfer_core::collect::slugify(&fp.cpu.brand)),
            "brand": fp.cpu.brand,
            "os": fp.identity.os,
            "arch": fp.identity.arch,
            "bandwidth_gbs": {
                "single_thread": fp.single_thread_gbs(),
                "saturated": fp.saturated_gbs(),
            },
            "memory": {
                "total_bytes": fp.memory.total_bytes,
                "available_bytes": fp.memory.available_bytes,
                "pressure": format!("{:?}", fp.pressure()).split(' ').next().unwrap_or("Unknown"),
            },
            "trusted": ws.is_trusted(),
            "violations": ws.violations.iter().map(|v| serde_json::json!({
                "code": v.code(), "message": v.to_string()
            })).collect::<Vec<_>>(),
        });
        println!("{}", envelope("detect", data));
    } else {
        println!("machine    {} ({})", fp.cpu.brand, fp.identity.arch);
        println!(
            "bandwidth  {:.1} GB/s saturated · {:.1} GB/s single-thread",
            fp.saturated_gbs(),
            fp.single_thread_gbs()
        );
        println!(
            "memory     {:.1} GiB total · {} available",
            fp.total_gib(),
            fp.memory
                .available_bytes
                .map(|b| format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64))
                .unwrap_or_else(|| "not reported".into())
        );
        println!(
            "trust      {}",
            if ws.is_trusted() {
                "valid".to_string()
            } else {
                format!("{} violation(s) — not usable for planning", ws.violations.len())
            }
        );
        for v in ws.violations.iter().take(3) {
            println!("           {} {}", v.code(), v);
        }
    }
    exit::OK
}

fn cmd_plan(parsed: &Parsed) -> i32 {
    let ws = load_workspace();
    let Some(fp) = ws.fingerprint.clone() else {
        let msg = ws.error.clone().unwrap_or_else(|| "no fingerprint found".into());
        if parsed.json {
            println!("{}", error_envelope("plan", exit::UNSUPPORTED, &msg));
        } else {
            eprintln!("qinfer: {msg}");
        }
        return exit::UNSUPPORTED;
    };

    let catalog = match Catalog::from_json(CATALOG_JSON) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("catalogue parse failed: {e}");
            if parsed.json {
                println!("{}", error_envelope("plan", exit::FAILED, &msg));
            } else {
                eprintln!("qinfer: {msg}");
            }
            return exit::FAILED;
        }
    };

    // AT-0.7: a stale catalogue must warn, offline or not. Age is measured
    // against the generation date rather than assumed to be current.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let catalog_age_days = catalog.age_days(now);
    let catalog_stale = catalog.is_stale(now, 90);

    let session = match Session::new(fp, catalog) {
        Ok(s) => s,
        Err(e) => {
            if parsed.json {
                println!("{}", error_envelope("plan", exit::FAILED, &e.to_string()));
            } else {
                eprintln!("qinfer: {e}");
            }
            return exit::FAILED;
        }
    };

    let req = PlanRequest {
        family: parsed.family.clone(),
        ctx: parsed.ctx,
        concurrency: 1,
        objective: parsed.objective,
    };
    let set = session.plans(&req);
    let code = exit_for(&set);

    if parsed.json {
        let plans: Vec<serde_json::Value> = set
            .plans
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "entry_id": p.entry_id,
                    "display": p.display,
                    "predicted": {
                        "p10": p.predicted.p10, "p50": p.predicted.p50, "p90": p.predicted.p90
                    },
                    "peak_bytes": p.peak_bytes,
                    "rationale": p.rationale,
                    "risks": p.risks,
                    "fallbacks": p.fallbacks,
                })
            })
            .collect();

        let rejected: Vec<serde_json::Value> = set
            .rejected
            .iter()
            .map(|r| serde_json::json!({ "code": r.code(), "entry_id": r.entry_id(), "reason": r.to_string() }))
            .collect();

        let data = serde_json::json!({
            "arch_id": session.arch_id(),
            "ctx": set.ctx,
            "objective": format!("{:?}", set.objective).to_lowercase(),
            "calibration": session.calibration_summary(),
            "availability_assumed": session.availability_is_assumed(),
            "plans": plans,
            "rejected": rejected,
            "offline": parsed.offline,
            "catalog_age_days": catalog_age_days,
            "catalog_stale": catalog_stale,
        });
        println!("{}", envelope("plan", data));
        return code;
    }

    println!(
        "{} · ctx {} · {} available{}",
        session.arch_id(),
        set.ctx,
        format_gib(session.available_bytes()),
        if session.availability_is_assumed() { " (assumed: half of RAM)" } else { "" }
    );
    println!("calibration: {}", session.calibration_summary());
    if catalog_stale {
        println!(
            "⚠ catalogue is {} days old — plans may reference artifacts that have moved. Run `qinfer catalog update` when online.",
            catalog_age_days.unwrap_or(-1)
        );
    }
    println!();

    if set.plans.is_empty() {
        println!("no feasible plan.");
        if let Some(b) = set.binding_constraint() {
            println!("binding constraint: {b}");
        }
        return code;
    }

    for (i, p) in set.plans.iter().enumerate() {
        println!(
            "{:>2}. {:<38} {:>5.0} tok/s   {:.1} GiB",
            i + 1,
            p.display,
            p.predicted.p50,
            p.peak_bytes as f64 / (1u64 << 30) as f64
        );
        if i == 0 {
            for r in p.rationale.iter().take(3) {
                println!("      {r}");
            }
        }
    }

    let fit = set.rejected.iter().filter(|r| matches!(r, Rejection::DoesNotFit { .. })).count();
    if fit > 0 {
        println!("\n{} artifact(s) excluded: do not fit in available memory", fit);
    }
    code
}

fn format_gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

fn print_help() {
    println!("qinfer — bare-metal inference provisioning\n");
    println!("USAGE:");
    println!("  qinfer                 open the dashboard");
    println!("  qinfer detect [--json] [--offline]");
    println!("  qinfer plan   [--json] [--model <family>] [--ctx N] [--objective <o>] [--offline]");
    println!("  qinfer spec-lint [--json]      structural gate over the repo's own specs");
    println!("  qinfer help | version\n");
    println!("OBJECTIVES: max_speed · balanced (default) · max_quality · max_context · fit\n");
    println!("EXIT CODES:");
    println!("  0 ok · 2 unsupported · 3 does not fit · 6 failed · 64 usage");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_each_command() {
        assert_eq!(parse(&args(&["detect"])).unwrap().command, Command::Detect);
        assert_eq!(parse(&args(&["plan"])).unwrap().command, Command::Plan);
        assert_eq!(parse(&args(&["--help"])).unwrap().command, Command::Help);
        assert_eq!(parse(&args(&["-V"])).unwrap().command, Command::Version);
    }

    #[test]
    fn parses_flags_with_defaults() {
        let p = parse(&args(&["plan"])).unwrap();
        assert!(!p.json);
        assert!(!p.offline);
        assert_eq!(p.ctx, 4096);
        assert_eq!(p.objective, Objective::Balanced);
        assert_eq!(p.family, None);
    }

    #[test]
    fn parses_flags_with_values() {
        let p = parse(&args(&["plan", "--json", "--ctx", "32768", "--objective", "max_speed", "--model", "qwen2.5"]))
            .unwrap();
        assert!(p.json);
        assert_eq!(p.ctx, 32768);
        assert_eq!(p.objective, Objective::MaxSpeed);
        assert_eq!(p.family.as_deref(), Some("qwen2.5"));
    }

    #[test]
    fn rejects_a_flag_without_its_value() {
        assert_eq!(parse(&args(&["plan", "--ctx"])).unwrap_err(), ParseError::MissingValue("--ctx".into()));
    }

    #[test]
    fn rejects_a_bad_value_rather_than_defaulting() {
        assert!(matches!(parse(&args(&["plan", "--ctx", "lots"])).unwrap_err(), ParseError::BadValue { .. }));
        assert!(matches!(
            parse(&args(&["plan", "--objective", "turbo"])).unwrap_err(),
            ParseError::BadValue { .. }
        ));
    }

    #[test]
    fn rejects_an_unknown_command_and_flag() {
        assert!(matches!(parse(&args(&["frobnicate"])).unwrap_err(), ParseError::UnknownCommand(_)));
        assert!(matches!(parse(&args(&["plan", "--wat"])).unwrap_err(), ParseError::UnknownFlag(_)));
    }

    #[test]
    fn the_envelope_has_a_stable_shape() {
        let e = envelope("plan", serde_json::json!({"plans": []}));
        assert_eq!(e["schema"], SCHEMA);
        assert_eq!(e["command"], "plan");
        assert_eq!(e["ok"], true);
        assert!(e["error"].is_null());
        assert!(e["data"].is_object());
    }

    #[test]
    fn the_error_envelope_carries_the_exit_code() {
        let e = error_envelope("plan", exit::NO_FIT, "nothing fits");
        assert_eq!(e["ok"], false);
        assert_eq!(e["error"]["exit"], exit::NO_FIT);
        assert_eq!(e["error"]["message"], "nothing fits");
        assert!(e["data"].is_null());
    }

    #[test]
    fn unsupported_and_does_not_fit_are_different_exit_codes() {
        use qinfer_core::planner::PlanSet;

        let unsupported = PlanSet {
            plans: vec![],
            rejected: vec![Rejection::NoCoverage { entry_id: "x".into(), arch_id: "sm_121".into() }],
            objective: Objective::Balanced,
            ctx: 4096,
        };
        assert_eq!(exit_for(&unsupported), exit::UNSUPPORTED);

        let too_big = PlanSet {
            plans: vec![],
            rejected: vec![Rejection::DoesNotFit {
                entry_id: "x".into(),
                peak_bytes: 10,
                available_bytes: 1,
                weights_bytes: 9,
                kv_bytes: 1,
            }],
            objective: Objective::Balanced,
            ctx: 4096,
        };
        assert_eq!(exit_for(&too_big), exit::NO_FIT);
    }
}
