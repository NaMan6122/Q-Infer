# Task Backlog — M0 (Advisor)

Atomic units. A task is `DONE` only when its acceptance criteria are covered by tests built **independently** of the worker that wrote the code (AOP v5 §7), and the Orchestrator has seen the Verification packet.

**Reversibility** — `LOW` requires a Critic review plus ≥2 genuinely evaluated options (G2). `MEDIUM`/`HIGH` proceed with logging.

## Summary

| ID | Task | Rev. | Milestone | Status |
|---|---|---|---|---|
| T-001 | `cfp@1` schema as Rust types + validation | LOW | M0 | READY |
| T-002 | Bandwidth probe: thread sweep + pressure guard | MEDIUM | M0 | READY |
| T-003 | macOS collectors with the documented traps | HIGH | M0 | READY |
| T-004 | Catalog v1 — 12 entries, arch coverage | HIGH | M0 | READY |
| T-005 | Predictor + `k_cal` store | MEDIUM | M0 | READY |
| T-006 | Planner: constraint solver + rationale | LOW | M0 | READY |
| T-007 | CLI: `detect`, `plan`, `--json`, exit codes | HIGH | M0 | READY |
| T-008 | TUI wired to core (not spike JSON) | HIGH | M0 | READY |
| T-009 | Spec structural validator (mechanises G1) | HIGH | M0 | READY |
| T-010 | Second spike: MoE + CUDA | HIGH | M0 | BLOCKED — hardware |

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

**Test Evidence:** `crates/qinfer-core/src/cfp.rs` — 20 unit tests incl. the committed-artefact regression; `cargo test -p qinfer-core` → 27 passed.
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

Mechanises G1(a): Given/When/Then present, Risks non-empty or justified `NONE`, Rollback filled, dependency chain resolved.
**Test Evidence:** `tests/spec_lint.rs` against this repository's own specs.
**Rollback:** tooling only.

---

## T-010 — Second spike: MoE + CUDA

**Status:** BLOCKED — requires ≥24 GB free unified memory (MoE) and any CUDA host.
**Why it matters:** `t₀` dominance is the entire MoE story and is untested; the CUDA result could show the Metal finding is an artifact.
**Exit:** a report with MAPE for a 30B-A3B model and for three dense models on CUDA.
