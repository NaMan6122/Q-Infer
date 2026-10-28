# Spec Index

Single source of truth for what is `ACTIVE`, what is `DRAFT`, and what depends on what.
A spec may only move `DRAFT → ACTIVE` through G1 (structural validator + Critic sign-off + Test Evidence at the tier's bar).

## Normative specs

| Spec | Path | Status | Version | Depends on | Owner |
|---|---|---|---|---|---|
| Product requirements | `PRD.md` | **ACTIVE** | v0.3 | — | Orchestrator |
| System design | `SDD.md` | **ACTIVE** | v0.1 | PRD | Orchestrator |
| Charter | `init-charter.md` | **DRAFT** | v0.1 | PRD, AOP v5 | Human (ratification) |
| Instruction protocol | `AOP-v5-AutoSuper.md` | **ACTIVE** | v5 (proposal, adopted) | — | Human |
| Task backlog | `TASKS.md` | **ACTIVE** | v0.1 | Charter | Orchestrator |
| Roadmap (order & dependency) | `ROADMAP.md` | **ACTIVE** | v1 | PRD, TASKS, milestones | Orchestrator |

## Decisions

| ADR | Title | Status |
|---|---|---|
| 0001 | Core orchestrates engines as subprocesses | ACTIVE |
| 0002 | Fingerprint is the only source of hardware truth | ACTIVE |
| 0003 | Two-parameter roofline, unbounded `k_cal` | ACTIVE |
| 0004 | Catalog mediates churn; per-artifact arch coverage | ACTIVE |
| 0005 | Transactional, isolated, privilege-explicit provisioning | ACTIVE |
| 0006 | Receipts are the primary artifact; plain-file state | ACTIVE |
| 0007 | Tiered certification gated on owned hardware | ACTIVE |
| 0008 | Bundle runtimes get a separate adapter class | ACTIVE |
| 0009 | TUI is a first-class view (Ratatui) | ACTIVE |

## Evidence (non-normative)

| Artefact | Path | Status | Confidence |
|---|---|---|---|
| Apple dossier | `docs/research/01-apple-silicon.md` | ACTIVE | High on host, medium on Ultra |
| GB10 dossier | `docs/research/02-nvidia-gb10.md` | ACTIVE | High specs, medium perf |
| Snapdragon dossier | `docs/research/03-qualcomm-snapdragon.md` | ACTIVE | Low–medium perf |
| M0 spike | `spikes/m0/REPORT.md` | DONE | High, narrow scope |
| Test rig inventory | `docs/rigs.md` | ACTIVE | What each owned machine can and cannot prove (ADR-0007) |

## Milestones

| Milestone | Status | Exit gate |
|---|---|---|
| M0 Advisor | **IN PROGRESS** — 4 of 10 tasks merged (`cfp`, probe, collectors, predictor) | 12 acceptance tests green |
| M1 Apply | PENDING | 9 acceptance tests green |
| M2 Tune & Serve | PENDING | 8 acceptance tests green |
| M3 Fleet & Air-gap | PENDING | 7 acceptance tests green |

## Dependency chain

```
AOP v5 ──▶ Charter ──▶ TASKS ──▶ M0 ──▶ M1 ──▶ M2 ──▶ M3
                │                  │
PRD v0.3 ───────┘                  └── SDD v0.1 ──▶ ADRs
```
