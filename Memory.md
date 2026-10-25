# Memory

**Protocol:** AOP v5 (`AOP-v5-AutoSuper.md`) · **Session:** 2026-10-26 · **Phase:** M0 (Advisor)

## State

| Field | Value |
|---|---|
| Phase | M0 — Advisor. **IN PROGRESS**; T-001 reworked after `BLOCK`, T-002 in review. |
| Last file touched | `crates/qinfer-core/src/cfp.rs` (commit `73563e5`) |
| Immediate next step | Re-submit T-001 to the Critic for a second verdict, then T-003 |
| Open questions | OQ-01…OQ-04 (`init-charter.md` §8) — non-blocking; `instruction.md` v5 pending one-time human ratification |
| Verification bar | `SIGNIFICANT` tasks require unit tests **and** a simulation harness (§7) |
| Relay | none |

## What exists

| Artefact | Status | Notes |
|---|---|---|
| `PRD.md` v0.3 | ACTIVE | 59 requirements, 14 open questions resolved provisionally |
| `SDD.md` v0.1 | ACTIVE | Data contracts, engine adapter, error taxonomy |
| `docs/adr/0001…0009` | ACTIVE | Review triggers recorded per ADR |
| `docs/milestones/m0…m3` | ACTIVE | 36 acceptance tests |
| `docs/research/01…03` | ACTIVE | Apple / GB10 / Snapdragon dossiers |
| `spikes/m0/` | DONE | Predictor validated, MAPE 1.45 % |
| `crates/` | PARTIAL | `qinfer-core` types + `qinfer-tui` dashboard, builds clean |

## Decisions taken this session (append-only)

| # | Decision | Reversibility | Evidence |
|---|---|---|---|
| D-01 | Predict with `t₀ + bytes/(k_cal·BW)`, `k_cal` unbounded | HIGH | ADR-0003, spike R² 0.9996 |
| D-02 | `k_cal` renamed from η; it is not an efficiency | HIGH | spike, measured 1.175 |
| D-03 | Fingerprint publishes single-thread **and** saturated bandwidth | HIGH | spike F1, 1.74× gap |
| D-04 | Only Apple Silicon is Tier A today | MEDIUM | dossier 02 §1 refused GB10 certification |
| D-05 | TUI is a read-only dashboard, not a wizard | MEDIUM | ADR-0009 |
| D-06 | Dashboard verified via `--snapshot` in CI, not a human at a TTY | HIGH | ADR-0009, FR-58 |
| D-07 | `cfp@1` validation returns *all* violations, not the first | HIGH | commit `6a8b92e`, 9 tests |
| D-08 | Protocol consolidated into `instruction.md` v5; inferred v4 clauses marked `⚠R` | MEDIUM | awaiting one-time ratification |
| D-09 | PRs are **stacked**: each branch bases on its parent, not `main` | HIGH | OQ-05 (needs §11 amendment) |
| D-10 | Rust probe reproduces the Python finding (126.9 → 259.0 GB/s, 2.04×) | HIGH | `e087840`, example output |
| D-11 | Validation is enforced by a `Validated<T>` newtype, not a voluntary call | LOW | Critic finding 1; `73563e5` |
| D-12 | Pressure is swap **headroom** (<10% free), not swap-used-vs-RAM | MEDIUM | Critic counterexample 1; `73563e5` |
| D-13 | Ordering invariants compare like methods only (`headline_method`) | MEDIUM | Critic finding 2; `73563e5` |

## Open questions (queued, non-blocking)

| # | Question | Where it came from |
|---|---|---|
| OQ-01 | Does `sm_121` FP4 rewrite risk invalidate the GB10 Tier-A target for M1? | dossier 02 §3 |
| OQ-02 | Do we probe bandwidth natively on-device, or keep CPU probe + `k_cal`? | SDD S1 |
| OQ-03 | Catalog signing infrastructure — key custody, rotation | ADR-0004 |

## Relayed sessions

_none_

## Session log

| Time | Event |
|---|---|
| 2026-10-26 | PRD v0.1 → v0.2 (evidence pass) → v0.3 (dashboard) |
| 2026-10-26 | Three platform dossiers completed |
| 2026-10-26 | M0 spike completed; 4 claims corrected |
| 2026-10-26 | SDD, 9 ADRs, 4 milestone specs written |
| 2026-10-26 | Rust workspace + Ratatui dashboard built and validated |
| 2026-10-26 | AOP v5 adopted; bootstrap + interrogation phase opened |
| 2026-10-26 | 13 atomic baseline commits on `main`; protocol bootstrap merged |
| 2026-10-26 | Interrogation complete → `init-charter.md` ACTIVE |
| 2026-10-26 | T-001 `cfp@1` validation implemented (`6a8b92e`), 11 tests green, Critic dispatched |
| 2026-10-26 | PR **#1** opened: `feat/m0-advisor` → `main` |
| 2026-10-26 | T-002 bandwidth probe implemented (`e087840`), 19 tests green |
| 2026-10-26 | PR **#2** opened stacked on #1: `feat/m0-probe` → `feat/m0-advisor` |
| 2026-10-26 | Critic returned **BLOCK** on T-001 — 8 required findings, all valid |
| 2026-10-26 | T-001 reworked (`73563e5`, `b7bb509`, `0532c29`); 27 tests on branch, 35 on stack |
| 2026-10-26 | Stack rebased and force-pushed; PR #1 now +704/−25 |

## Open Question OQ-05 (new)

`instruction.md` §11 specifies conventional commits and branch-per-change but is silent on **stacked PRs**. Proposal: codify that a branch may base on its parent feature branch rather than `main`, that PR bodies declare `Depends on #N`, and that merges happen bottom-up. Cannot self-apply — §14 reserves `instruction.md` edits to the human.
