# Development Changelog

Append-only. Newest last. Commit refs are recorded where they exist.

## 2026-10-26

### v0.3 — the dashboard (ADR-0009)
- Ratatui dashboard in the same static binary; read-only view over `cfp@1` and results.
- `--snapshot WxH [--tab]` renders offscreen for CI and review.
- Layout breakpoints: two-column ≥76 cols, stacked below; bench sheds non-essential columns <84.
- PRD FR-55…FR-59 added (59 total). SDD §4.4 (TUI↔core contract) added. M0 gains D0.9 + AT-0.11/AT-0.12.

### v0.2 — evidence pass
Three platform dossiers and an M0 spike forced six corrections:
- `η` → `k_cal`; no longer bounded by 1.0 (measured 1.175).
- Bandwidth probe must saturate (131 → 228 GB/s, 1.74×).
- `t₀` is per model class (1.22 ms dense Metal vs ~8.4 ms MoE Ultra).
- NPU efficiency 0.15–0.35; CPU beats NPU above ~4B on Snapdragon.
- `sm_121` story moved from packaging to `tcgen05`/TMEM architecture.
- Q1–Q10 given provisional decisions; Q11–Q14 opened.

Spike result: two-parameter model **1.45 % MAPE** vs 15.6 % naive, R² 0.9996.

### v0.1 — initial PRD
- Product framing, six failure walls, personas, 54 requirements, platform tiers.
- SDD, 9 ADRs, 4 milestone specs (36 acceptance tests).

### Repo
- Rust workspace: `qinfer-core` (types + invariants), `qinfer-tui` (dashboard).
- 11 atomic baseline commits; protocol bootstrap on `chore/protocol-bootstrap`.

### M0 implementation
- **T-001** `cfp@1` validation — 5 invariants, 9 tests. PR **#1**.
- **T-002** bandwidth probe — thread sweep, 4x LLC floor, 8 tests. PR **#2**, stacked on #1.
- Rust probe reproduces the Python finding: 126.9 -> 259.0 GB/s (2.04x) at 15 threads.
- AOP v5 adopted; `instruction.md` v5 reconstructed (DRAFT), `init-charter.md` ACTIVE.
