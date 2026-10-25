# Memory

**Protocol:** AOP v5 (`AOP-v5-AutoSuper.md`) · **Session:** 2026-10-26 · **Phase:** M0 (Advisor)

## State

| Field | Value |
|---|---|
| Phase | M0 — Advisor. **Not started**; charter pending ratification. |
| Last file touched | `init-charter.md` (created, DRAFT) |
| Immediate next step | On charter ratification → promote T-001…T-007 to `READY` and dispatch workers |
| Open questions | Q-AOP-1 … Q-AOP-6 (see `init-charter.md` §6) — **blocking phase start only** |
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
