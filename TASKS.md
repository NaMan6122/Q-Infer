# Task Backlog — M0 (Advisor)

Atomic units. A task is `DONE` only when its acceptance criteria are covered by tests built **independently** of the worker that wrote the code (AOP v5 §7), and the Orchestrator has seen the Verification packet.

**Reversibility** — `LOW` requires a Critic review plus ≥2 genuinely evaluated options (G2). `MEDIUM`/`HIGH` proceed with logging.

## Summary

| ID | Task | Rev. | Milestone | Status |
|---|---|---|---|---|
| T-001 | `cfp@1` schema as Rust types + validation | LOW | M0 | **DONE** — PR #1, 3 Critic rounds |
| T-002 | Bandwidth probe: thread sweep + pressure guard | MEDIUM | M0 | **DONE** — merged #2, verified 7/7 |
| T-003 | macOS collectors with the documented traps | HIGH | M0 | **DONE** — merged #5, verified 6/6 |
| T-004 | Catalog v1 — 12 entries, arch coverage | HIGH | M0 | **DONE** — merged #11, 18 tests |
| T-005 | Predictor + `k_cal` store | MEDIUM | M0 | **DONE** — merged #6, verified 8/8 |
| T-006 | Planner: constraint solver + rationale | LOW | M0 | **DONE** — PR #12, rank order reproduced |
| T-007 | CLI: `detect`, `plan`, `--json`, exit codes | HIGH | M0 | **DONE** — PR #16, 9 tests |
| T-008 | TUI wired to core (not spike JSON) | HIGH | M0 | **DONE** — PR #13, Plans tab reads `PlanSet` |
| T-009 | Spec structural validator (mechanises G1) | HIGH | M0 | **DONE** — PR #17, 12 tests |
| T-011 | MLX engine adapter (second backend) | MEDIUM | M0 | **DONE** — PR #14, 10 tests |
| T-010 | Second spike: MoE + CUDA | HIGH | M0 | MoE ✅ · CUDA BLOCKED — hardware |

**Review state.** T-001 cleared three Critic rounds (`BLOCK`, `BLOCK`, `PASS-WITH-CHANGES`). T-002/T-003/T-005 cleared independent verification: 23 tests written by a separate agent against the acceptance criteria, fixtures deliberately different from the implementers' (an M4 Max rather than the M5 Pro), **all criteria VERIFIED, no behavioural defect found**.

Verification raised four non-blocking findings; all four were fixed rather than filed:
`reps` echoing the requested count instead of the effective one; `parse_gpu` discarding the whole record on one odd field type; `predict` publishing a positive throughput from a 0 GB/s probe; and a doc comment that derived the 4-bit arithmetic next to the 8-bit constant.

**Merged** (all six PRs are on `main`): #1 T-001 · #2 T-002 · #3 records · #4 ratification · #5 T-003 · #6 T-005 · #7 records.

---

## T-001 — `cfp@1` schema as Rust types + validation

**Acceptance criteria** — revised after Critic review (v0.2). The original roundtrip criterion was unimplementable (no `Serialize`, lossy fields) and is struck.

- *Given* the committed reference fingerprint (`spikes/m0/fingerprint.json`), *when* deserialised, *then* it is **rejected** with `E-DET-002` and `E-DET-003`. *(The Critic's "most important missing test": the one real artefact in the repo is not trustworthy.)*
- *Given* `saturated_gbs <= single_thread_gbs` on a multi-threaded sweep, *when* validated, *then* it is rejected — equality is the "sweep never ran" case.
- *Given* a headline taken from `triad` and a `copy` single-thread figure, *when* validated, *then* no ordering violation fires. *(Cross-method comparison is a guaranteed false positive.)*
- *Given* any sample with `spread_pct > 15` or `NaN`, *when* provenance claims `measured`, *then* it is rejected — `copy` and `triad` are both checked.
- *Given* a missing or unrecognised `provenance.bandwidth`, *when* validated, *then* it is rejected. *(Fails closed; a missing key previously passed.)*
- *Given* swap headroom below 10%, *when* provenance claims `measured`, *then* it is rejected. *(Headroom, not swap-used-vs-RAM: the reference host sat at 48.1% of RAM while swap was 96.3% full.)*
- *Given* `single_thread_gbs` absent, *when* deserialised, *then* it fails — the field is required, not optional.
- *Given* any consumer, *when* it needs trustworthy facts, *then* it must accept `&Validated<Fingerprint>`; an unvalidated fingerprint cannot reach the planner.

**Test Evidence:** `crates/qinfer-core/src/cfp.rs` — **35 tests**, incl. the committed-artefact regression, headline corroboration, the sealed `Validated` type, and fail-closed `headline_method`. `cargo test -p qinfer-core` on this branch → **38 passed + 1 `compile_fail` doctest**; on the integrated tip → **73 passed**.
**Rollback:** additive module; delete `cfp.rs`, no migration.

---

## T-002 — Bandwidth probe

**Acceptance criteria**
- *Given* a probe run, *when* it completes, *then* it reports single-thread **and** saturated GB/s plus the thread count at saturation.
- *Given* swap in use exceeds 50 % of RAM, *when* publishing, *then* provenance is `unknown` and the value is flagged.
- *Given* the M5 Pro reference host, *when* run, *then* saturated/single ≤ 2.5× and ≥ 1.3× (sanity band from the spike).
- *Given* a buffer smaller than 4× LLC, *when* configured, *then* the probe refuses to run.

**Test Evidence:** `tests/bandwidth.rs` + simulation harness `sim/bandwidth_pressured.yaml` (mocked /proc, forced swap)
**Rollback:** config flag to fall back to the Python probe.

---

## T-003 — macOS collectors

**Acceptance criteria**
- *Given* an Apple Silicon host, *when* collected, *then* `arch_id`, ISA flags and Metal family are present.
- *Given* `hw.l3cachesize` returns empty, *when* parsed, *then* the field is `null`, not `0` and not a panic.
- *Given* `hw.optional.arm.FEAT_I8MM` casing differs from `FEAT_DotProd`, *when* read, *then* both are detected.
- *Given* `platform.processor()` would return empty, *when* arch is determined, *then* it is not used.

**Test Evidence:** `tests/macos_collectors.rs` + recorded fixture from `spikes/m0/fingerprint.json`
**Rollback:** collectors are behind a trait; drop the impl.

---

## T-004 — Catalog v1

**Acceptance criteria**
- *Given* a catalog, *when* validated, *then* every entry has hash, licence, arch coverage and KV geometry.
- *Given* an artifact with no coverage for this `arch_id`, *when* planned, *then* it is never offered.
- *Given* two publishers' "Q4_K_M", *when* keyed, *then* they are distinct entries.
- *Given* the catalog is stale, *when* planning offline, *then* a staleness warning is emitted.

**Test Evidence:** `tests/catalog.rs` + `sim/catalog_missing_arch.yaml`
**Rollback:** catalog is data; revert to previous version pin.

---

## T-005 — Predictor + `k_cal` store

**Acceptance criteria**
- *Given* the spike fixtures, *when* predicting, *then* MAPE ≤ 10 % against measured rows.
- *Given* fewer than 3 observations, *when* fitting, *then* the fit is marked underdetermined and not used to claim accuracy.
- *Given* `k_cal > 1`, *when* stored, *then* it is accepted, not clamped.
- *Given* a receipt, *when* recorded, *then* prediction error is stored and the local `k_cal` updates.

**Test Evidence:** `tests/predictor.rs` + golden fixture from `spikes/m0/results.json`
**Rollback:** predictions are advisory; disable via feature flag.

---

## T-006 — Planner

**Acceptance criteria**
- *Given* a model request, *when* planning, *then* ≥3 ranked plans, each with non-empty rationale, risks and ≥2 fallbacks.
- *Given* a model that cannot fit, *when* planning, *then* exit 3 with the fit arithmetic shown.
- *Given* the spike models, *when* ranked, *then* predicted order matches measured order (Kendall τ ≥ 0.7).
- *Given* no feasible plan, *when* planning, *then* the reason names the binding constraint.

**Test Evidence:** `tests/planner.rs` + `sim/planner_rank_order.yaml`
**Rollback:** planner is pure (inputs → plans); delete and re-implement.

---

## T-007 — CLI surface

**Acceptance criteria**
- *Given* any command, *when* `--json` is passed, *then* a single versioned envelope is emitted.
- *Given* `--offline`, *when* run, *then* no network syscalls occur (traced).
- *Given* an unsupported platform, *when* `plan` runs, *then* exit 2 with a Tier-D reason.
- *Given* a stale catalog offline, *when* planning, *then* warning + success.

**Test Evidence:** `tests/cli.rs` + `sim/offline_network_trace.yaml`
**Rollback:** flags are additive.

---

## T-008 — TUI wired to core

**Acceptance criteria**
- *Given* a generated fingerprint, *when* the dashboard opens, *then* it renders from core types, not raw spike JSON.
- *Given* `--snapshot 64x20` and `140x40`, *when* rendered, *then* no panic and no clipped values.
- *Given* the dashboard, *when* it starts, *then* it opens no listening socket.
- *Given* no data, *when* opened, *then* an actionable empty state with the exact command to run.

**Test Evidence:** `tests/snapshot.rs` golden frames + `sim/tui_no_listener.yaml`
**Rollback:** UI-only; revert commit.

---

## T-009 — Spec structural validator

Mechanises G1(a) so the Critic reviews judgement, not formatting.

**Acceptance criteria**
- *Given* a task block, *when* linted, *then* it must carry acceptance criteria, Test Evidence and Rollback, or be rejected.
- *Given* a criterion written as prose ("it should work"), *when* linted, *then* it is rejected as not falsifiable.
- *Given* a summary-table row with no `LOW`/`MEDIUM`/`HIGH`, *when* linted, *then* reversibility is reported missing.
- *Given* an ADR with no `Alternatives considered`, *when* linted, *then* it is rejected — G2 requires ≥2 evaluated options.
- *Given* a **warning** (e.g. no negative criterion), *when* linted, *then* the gate still passes. A linter that fails on style gets bypassed.
- *Given* **this repository**, *when* linted, *then* zero errors — a linter that fails on its own specs is worthless.

**Test Evidence:** `crates/qinfer-core/src/spec.rs` — 12 tests, including the repo linting itself via `include_str!`.
**Rollback:** tooling only; delete the module.

---

## T-010 — Second spike: MoE + CUDA

**Status:** PARTIAL — MoE done (OLMoE, §10). CUDA blocked: no host available.

**Acceptance criteria**
- *Given* a 30B-class MoE (Qwen3-30B-A3B, 128 experts), *when* benchmarked, *then* its decode is within ±25 % of the two-parameter prediction.
- *Given* three dense models on a CUDA host, *when* benchmarked, *then* MAPE is within ±25 %, proving the Metal result is not a platform artefact.
- *Given* the same model on Metal and CUDA, *when* compared, *then* the difference is attributable to bytes or `k_cal`, not to an unexplained residual.
- *Given* a machine that spills, *when* benchmarked, *then* the prediction is **invalidated**, not reported with a 10× error.

**Test Evidence:** a `REPORT.md` addendum with measured-vs-predicted per model. MoE half satisfied (§10).
**Rollback:** evidence only.

---

## T-011 — MLX engine adapter

**Acceptance criteria**
- *Given* a `gguf` artifact, *when* an adapter is requested, *then* llama.cpp is selected and MLX is not.
- *Given* an `mlx` artifact, *when* an adapter is requested, *then* MLX is selected.
- *Given* engine output that is not a measurement (traceback, zero, missing decode row), *when* parsed, *then* `None` is returned rather than a fabricated number.
- *Given* neither runtime installed, *when* detection runs, *then* both report absent — presence is never assumed.
- *Given* the same model on both engines, *when* measured, *then* the decode difference is explained by bytes-per-token, not by the framework name.

**Test Evidence:** `crates/qinfer-core/src/engines.rs` — 10 tests; `spikes/m0/REPORT.md` §11.
**Rollback:** revert the commit; adapters are additive.
