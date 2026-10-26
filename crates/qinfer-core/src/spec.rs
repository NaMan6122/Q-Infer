//! Spec structural linter (T-009).
//!
//! AOP v5 replaced the human at G1 with "a structural validator passes". This
//! *is* that validator. It checks the shape of a spec, not its content: an
//! acceptance criterion that is not falsifiable, a task with no rollback, or an
//! ADR with no alternatives considered are all detectable without judgement.
//!
//! Errors block promotion. Warnings are reported and do not — a linter that
//! fails on style teaches people to bypass it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
}

impl Finding {
    pub fn code(&self) -> &'static str {
        self.rule
    }
}

fn finding(file: &str, line: usize, rule: &'static str, severity: Severity, message: impl Into<String>) -> Finding {
    Finding { file: file.to_string(), line, rule, severity, message: message.into() }
}

// ---------------------------------------------------------------------------
// Task specs
// ---------------------------------------------------------------------------

/// One `## T-NNN — title` block.
#[derive(Debug, Clone)]
pub struct TaskBlock {
    pub id: String,
    pub line: usize,
    pub body: String,
}

pub fn split_tasks(markdown: &str) -> Vec<TaskBlock> {
    let lines: Vec<(usize, &str)> = markdown.lines().enumerate().map(|(i, l)| (i + 1, l)).collect();
    let mut starts: Vec<(usize, String)> = Vec::new();

    for (lineno, text) in &lines {
        if let Some(rest) = text.strip_prefix("## T-") {
            let id = format!("T-{}", rest.split_whitespace().next().unwrap_or(""));
            starts.push((*lineno, id));
        }
    }

    starts
        .iter()
        .enumerate()
        .map(|(i, (lineno, id))| {
            let end = starts.get(i + 1).map(|(l, _)| *l).unwrap_or(lines.len() + 1);
            let body: String = lines
                .iter()
                .filter(|(l, _)| *l >= *lineno && *l < end)
                .map(|(_, t)| *t)
                .collect::<Vec<_>>()
                .join("\n");
            TaskBlock { id: id.clone(), line: *lineno, body }
        })
        .collect()
}

/// Lint every task block in a markdown document.
pub fn lint_tasks(markdown: &str, file: &str) -> Vec<Finding> {
    let mut out = Vec::new();

    for task in split_tasks(markdown) {
        let body = task.body.as_str();
        let lower = body.to_ascii_lowercase();

        // G1: acceptance criteria must exist and be falsifiable.
        if !lower.contains("**acceptance criteria**") {
            out.push(finding(file, task.line, "E-SPEC-001", Severity::Error,
                format!("{}: no acceptance criteria", task.id)));
        } else {
            let bullets: Vec<&str> = body
                .lines()
                .filter(|l| l.trim_start().starts_with('-'))
                .collect();
            let gwt = bullets
                .iter()
                .filter(|b| {
                    let b = b.to_ascii_lowercase();
                    b.contains("given") && b.contains("when") && b.contains("then")
                })
                .count();

            if bullets.is_empty() {
                out.push(finding(file, task.line, "E-SPEC-002", Severity::Error,
                    format!("{}: acceptance criteria has no bullets", task.id)));
            } else if gwt == 0 {
                out.push(finding(file, task.line, "E-SPEC-002", Severity::Error,
                    format!("{}: no criterion in Given/When/Then form", task.id)));
            }

            let negative = bullets.iter().any(|b| {
                let b = b.to_ascii_lowercase();
                b.contains("should not") || b.contains("must not") || b.contains("negative")
            });
            if !negative {
                out.push(finding(file, task.line, "E-SPEC-003", Severity::Warning,
                    format!("{}: no negative criterion (a case that should NOT occur)", task.id)));
            }
        }

        // G1: evidence and rollback are not optional.
        if !has_filled_field(body, "**Test Evidence:**") {
            out.push(finding(file, task.line, "E-SPEC-004", Severity::Error,
                format!("{}: no Test Evidence", task.id)));
        }
        if !has_filled_field(body, "**Rollback:**") {
            out.push(finding(file, task.line, "E-SPEC-005", Severity::Error,
                format!("{}: no Rollback", task.id)));
        }
    }

    // Reversibility lives in the summary table, not the block.
    for task in split_tasks(markdown) {
        if let Some(line) = table_row_for(markdown, &task.id) {
            let row = markdown.lines().nth(line - 1).unwrap_or("");
            let upper = row.to_ascii_uppercase();
            if !(upper.contains("LOW") || upper.contains("MEDIUM") || upper.contains("HIGH")) {
                out.push(finding(file, line, "E-SPEC-006", Severity::Error,
                    format!("{}: reversibility not declared", task.id)));
            }
        }
    }

    out
}

/// A field is "filled" when the same line carries content after the label.
fn has_filled_field(body: &str, label: &str) -> bool {
    body.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with(label) && t[label.len()..].trim().len() > 3
    })
}

fn table_row_for(markdown: &str, id: &str) -> Option<usize> {
    markdown
        .lines()
        .enumerate()
        .find(|(_, l)| l.trim_start().starts_with(&format!("| {id} |")))
        .map(|(i, _)| i + 1)
}

// ---------------------------------------------------------------------------
// ADRs
// ---------------------------------------------------------------------------

pub fn lint_adr(markdown: &str, file: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let lower = markdown.to_ascii_lowercase();

    for (label, rule, msg) in [
        ("## context", "E-SPEC-010", "no Context section"),
        ("## decision", "E-SPEC-011", "no Decision section"),
        ("## consequences", "E-SPEC-012", "no Consequences section"),
    ] {
        if !lower.contains(label) {
            out.push(finding(file, 1, rule, Severity::Error, msg.to_string()));
        }
    }

    // G2: a LOW-reversibility decision is only legitimate with options weighed.
    if !lower.contains("## alternatives considered") {
        out.push(finding(file, 1, "E-SPEC-013", Severity::Error,
            "no Alternatives considered section".to_string()));
    } else {
        let after = lower.split("## alternatives considered").nth(1).unwrap_or("");
        let content = after.split("\n## ").next().unwrap_or("").trim();
        if content.len() < 40 {
            out.push(finding(file, 1, "E-SPEC-013", Severity::Error,
                "Alternatives section is effectively empty — G2 requires >=2 evaluated options".to_string()));
        }
    }

    if !lower.contains("- **status:**") {
        out.push(finding(file, 1, "E-SPEC-014", Severity::Error, "no Status field".to_string()));
    }

    out
}

// ---------------------------------------------------------------------------
// Milestones
// ---------------------------------------------------------------------------

pub fn lint_milestone(markdown: &str, file: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let lower = markdown.to_ascii_lowercase();

    if !lower.contains("## 4. acceptance tests") {
        out.push(finding(file, 1, "E-SPEC-020", Severity::Error,
            "no acceptance tests section".to_string()));
        return out;
    }

    // The section must be followed by a table of AT-N rows.
    let after = markdown.split("## 4. Acceptance tests").nth(1).unwrap_or("");
    let tests = after.lines().filter(|l| l.trim_start().starts_with("| AT-")).count();
    if tests == 0 {
        out.push(finding(file, 1, "E-SPEC-021", Severity::Error,
            "acceptance-test section has no AT-N rows".to_string()));
    }

    if !lower.contains("## 5. exit criteria") {
        out.push(finding(file, 1, "E-SPEC-022", Severity::Error,
            "no exit criteria".to_string()));
    }

    out
}

pub struct LintSummary {
    pub errors: usize,
    pub warnings: usize,
    pub findings: Vec<Finding>,
}

impl LintSummary {
    pub fn is_clean(&self) -> bool {
        self.errors == 0
    }

    pub fn from(findings: Vec<Finding>) -> Self {
        let errors = findings.iter().filter(|f| f.severity == Severity::Error).count();
        let warnings = findings.iter().filter(|f| f.severity == Severity::Warning).count();
        LintSummary { errors, warnings, findings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASKS: &str = include_str!("../../../TASKS.md");
    const ADR: &str = include_str!("../../../docs/adr/0003-prediction-uses-a-two-parameter-roofline.md");
    const MILESTONE: &str = include_str!("../../../docs/milestones/m0-advisor.md");

    #[test]
    fn the_repository_tasks_pass_their_own_linter() {
        let s = LintSummary::from(lint_tasks(TASKS, "TASKS.md"));
        assert!(s.is_clean(), "errors: {:#?}", s.findings.iter().filter(|f| f.severity == Severity::Error).collect::<Vec<_>>());
    }

    #[test]
    fn the_repository_adrs_pass_their_own_linter() {
        let s = LintSummary::from(lint_adr(ADR, "ADR-0003"));
        assert!(s.is_clean(), "{:#?}", s.findings);
    }

    #[test]
    fn the_repository_milestones_pass_their_own_linter() {
        let s = LintSummary::from(lint_milestone(MILESTONE, "M0"));
        assert!(s.is_clean(), "{:#?}", s.findings);
    }

    #[test]
    fn every_task_in_the_backlog_is_found() {
        let tasks = split_tasks(TASKS);
        assert!(tasks.len() >= 11, "expected the full backlog, found {}", tasks.len());
        assert!(tasks.iter().any(|t| t.id == "T-001"));
        assert!(tasks.iter().any(|t| t.id == "T-011"));
    }

    #[test]
    fn a_task_without_rollback_is_rejected() {
        let md = "| T-900 | x | HIGH | M0 | READY |\n\n## T-900 — x\n\n**Acceptance criteria**\n- *Given* a, *when* b, *then* c\n\n**Test Evidence:** tests/x.rs\n";
        let s = LintSummary::from(lint_tasks(md, "x.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-005"), "{:#?}", s.findings);
    }

    #[test]
    fn a_task_without_test_evidence_is_rejected() {
        let md = "| T-901 | x | HIGH | M0 | READY |\n\n## T-901 — x\n\n**Acceptance criteria**\n- *Given* a, *when* b, *then* c\n\n**Rollback:** revert\n";
        let s = LintSummary::from(lint_tasks(md, "x.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-004"), "{:#?}", s.findings);
    }

    #[test]
    fn a_criterion_that_is_not_falsifiable_is_rejected() {
        let md = "| T-902 | x | HIGH | M0 | READY |\n\n## T-902 — x\n\n**Acceptance criteria**\n- it should work nicely\n\n**Test Evidence:** tests/x.rs\n**Rollback:** revert\n";
        let s = LintSummary::from(lint_tasks(md, "x.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-002"), "{:#?}", s.findings);
    }

    #[test]
    fn a_task_without_reversibility_in_the_table_is_rejected() {
        let md = "| T-903 | x | ??? | M0 | READY |\n\n## T-903 — x\n\n**Acceptance criteria**\n- *Given* a, *when* b, *then* c\n\n**Test Evidence:** tests/x.rs\n**Rollback:** revert\n";
        let s = LintSummary::from(lint_tasks(md, "x.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-006"), "{:#?}", s.findings);
    }

    #[test]
    fn an_adr_without_alternatives_is_rejected() {
        let md = "# ADR\n\n- **Status:** Accepted\n\n## Context\n\nx\n\n## Decision\n\ny\n\n## Consequences\n\nz\n";
        let s = LintSummary::from(lint_adr(md, "adr.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-013"), "{:#?}", s.findings);
    }

    #[test]
    fn an_adr_with_an_empty_alternatives_section_is_rejected() {
        let md = "# ADR\n\n- **Status:** Accepted\n\n## Context\n\nx\n\n## Decision\n\ny\n\n## Consequences\n\nz\n\n## Alternatives considered\n\nnone\n";
        let s = LintSummary::from(lint_adr(md, "adr.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-013"), "{:#?}", s.findings);
    }

    #[test]
    fn a_milestone_without_acceptance_tests_is_rejected() {
        let md = "# M9\n\n## 4. Acceptance tests\n\nnothing here\n\n## 5. Exit criteria\n\nx\n";
        let s = LintSummary::from(lint_milestone(md, "m9.md"));
        assert!(s.findings.iter().any(|f| f.code() == "E-SPEC-021"), "{:#?}", s.findings);
    }

    #[test]
    fn warnings_do_not_block() {
        let s = LintSummary::from(vec![
            Finding { file: "x".into(), line: 1, rule: "E-SPEC-003", severity: Severity::Warning, message: "m".into() },
        ]);
        assert!(s.is_clean(), "a warning alone must not fail the gate");
        assert_eq!(s.warnings, 1);
    }
}
