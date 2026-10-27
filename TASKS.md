# Task Backlog — M0 (Advisor)

> **M1 tasks (T-012 … T-016) are specified in [`ROADMAP.md`](ROADMAP.md) §4** rather than here. They enter this backlog when M1 starts, so that a task section never exists without its acceptance criteria having been thought through.

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
| T-012 | Memory budget from the device, not half of RAM | MEDIUM | M0 | **DONE** — 10 unit + 11 contract tests |
| T-013 | Micro-operation suite (GPU stream, GEMV, GEMM, ANE) | MEDIUM | M0 | **DONE** — 8 unit tests, live on M5 Pro |
| T-014 | Sweet-spot frontier: capability vs speed, knee marked | MEDIUM | M0 | **DONE** — 11 tests, live on M5 Pro |
| T-015 | Usability floor on the frontier | LOW | M0 | **DONE** — 6 tests, changes the answer |
| T-016 | Setup ladder: staged run with injected measurements | MEDIUM | M0 | **DONE** — 8 tests |
| T-017 | Dashboard: Frontier tab and a measured memory budget | MEDIUM | M0 | **DONE** — renders at 46–100 cols |
| T-018 | Dashboard: Setup tab carries the ladder | MEDIUM | M0 | **DONE** — 8 snapshot tests, live on M5 Pro |
| T-019 | Code-quality measurement: mean NLL on a fixed corpus | MEDIUM | M0 | **DONE** — 10 tests, 8.0% measured |
| T-020 | Rank the frontier on measured quality | MEDIUM | M0 | **DONE** — 22 tests, catalogue round-trips |

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

---

## T-012 — Memory budget from the device, not half of RAM

Before this task, `session` answered "how much memory may this workload use?" with **half of RAM**, stated as an assumption and then consumed as a fact. On the reference 24 GiB M5 Pro that is 12.0 GiB, which rejected `qwen3.8-27b-4bit-mlx` at 14.98 GiB — a model sitting in this machine's own cache. An assumption that refuses the correct answer is indistinguishable from a hardware limit.

**Acceptance criteria**
- *Given* a GPU device report carrying `max_recommended_working_set_size`, *when* a session resolves its budget, *then* the plan budget is that figure and not half of physical RAM.
- *Given* a device report whose working set exceeds physical RAM, *when* resolved, *then* the figure is refused and resolution falls through to the reclaimable figure rather than being clamped.
- *Given* neither a device report nor a `vm_stat` figure, *when* resolved, *then* the source is `AssumedFraction` and provenance is `unknown`.
- *Given* a machine whose configured swap is over 90% used, *when* resolved even with a valid working set, *then* provenance is `unknown` — a paging machine is not a measurement.
- *Given* headroom is unknown, *when* `fits_now` is asked, *then* it returns `None` and **must not** return `false`.
- *Given* the same device report under different current load, *when* resolved twice, *then* `plan_budget()` is identical — a plan is not a snapshot.

**Test Evidence:** `crates/qinfer-core/src/space.rs` — 10 unit tests; `crates/qinfer-core/tests/space_contract.rs` — 11 contract tests written against the criteria on a 32 GiB fixture deliberately different from the module's 24 GiB one. Live: `cargo run --release -p qinfer-core --example measure` resolves 17.8 GiB against a 12.0 GiB assumption (1.48x) and admits the 27B.
**Rollback:** additive module; deleting the `space` field from `Session` restores the stated-fraction behaviour.

---

## T-013 — Micro-operation suite

The predictor needs numbers no spec sheet provides: the bandwidth the **device** achieves, the throughput of the compute-bound phase, and whether the neural-engine path exists at all. `k_cal` was carrying all three as one free parameter; these ops remove it.

**Acceptance criteria**
- *Given* a machine with MLX installed, *when* the probe runs, *then* it reports GPU stream bandwidth, quantised GEMV bandwidth at a weight size above last-level cache, and batched bf16 matmul throughput, each with method, repetition count and spread.
- *Given* the reference M5 Pro, *when* the probe runs, *then* the GPU stream figure agrees with the saturated CPU probe within 10%.
- *Given* an op the probe could not perform, *when* parsed, *then* it is `Refused` with a reason and a fix, and **must not** be reported as `0.0`.
- *Given* a weight matrix under 128 MiB, *when* the GEMV op runs, *then* it declines rather than reporting a cache-resident figure that over-states bandwidth fivefold.
- *Given* non-JSON output from the probe, *when* parsed, *then* every requested op is refused and the device block is absent.

**Test Evidence:** `crates/qinfer-core/src/microops.rs` — 8 tests, including the live M5 Pro payload verbatim. Measured on the reference machine: GPU stream 257.9 GB/s (20.7% spread), GEMV q4 256.4 GB/s (1.7%), GEMM bf16 23.1 TFLOP/s (5.5%), ANE unreachable. `k_cal` fell from 1.175 to 1.031 once the probe saturated — the original coefficient was an artefact of an under-saturated CPU probe, which is PRD Q11 answered by measurement rather than argument.
**Rollback:** additive module; the planner falls back to the stored `k_cal` when no micro-op report is supplied.

---

## T-014 — Sweet-spot frontier

`Planner` ranks by throughput, and on a bandwidth-bound machine the fastest artifact is always the smallest one: on the reference M5 Pro the top-ranked plan is a 0.5B at 339 tok/s. A quality *floor* does not fix it either, because `quality_placeholder` keys on quantisation rather than model size, so a 0.5B Q4 and a 27B Q4 both score 0.93. The missing axis is capability, which with speed makes the question a curve rather than a ranking.

**Acceptance criteria**
- *Given* a plan set and a catalogue, *when* the frontier is computed, *then* a candidate that is both slower and less capable than another is absent from it.
- *Given* three or more frontier points, *when* the knee is found, *then* it is the point furthest above the chord joining the two ends, and **must not** depend on a weighting constant the product could not defend.
- *Given* an MoE entry, *when* its capability prior is computed, *then* it uses total parameters, and the active-parameter reading must not be the one used.
- *Given* a catalogue whose `quality.value` is null everywhere, *when* a frontier is produced, *then* every point is labelled `prior (params)` and the skew is reported in `notes`.
- *Given* fewer than three frontier points, *when* `recommended()` is called, *then* it returns `None` and `reason()` explains why rather than inventing a winner.

**Test Evidence:** `crates/qinfer-core/src/sweetspot.rs` — 17 tests. Live on the M5 Pro the frontier is Qwen3 Coder 30B-A3B (123 tok/s, cap 1.00), OLMoE (245, 0.64), 0.5B (339, 0.00); the three 3B variants are correctly dominated.
**Rollback:** additive module; `Planner` output is unchanged and remains the throughput-ranked answer.

---

## T-015 — Usability floor on the frontier

The knee answers a question about a curve; the user asks a different one. On the reference M5 Pro they diverge: the knee is a 7B at 245 tok/s, the answer is a 30B-A3B at 123, and both are far above the rate at which anyone reads. Past the floor extra speed is worth nothing, so capability should decide.

**Acceptance criteria**
- *Given* a frontier and a floor, *when* the recommendation is taken, *then* it is the most capable point whose predicted decode is at or above the floor.
- *Given* a point predicted at exactly the floor, *when* the recommendation is taken, *then* it is admissible — the comparison **must not** lose a model to rounding.
- *Given* a floor nothing reaches, *when* the recommendation is taken, *then* it returns nothing and the reason names the fastest available rate, rather than silently falling back to the fastest plan.
- *Given* a recommendation above the floor, *when* its reason is rendered, *then* it names the floor and the alternative it traded away, so the choice is auditable.
- *Given* the knee and the floor-constrained answer differ, *when* both are shown, *then* both are labelled distinctly — a surface must not present one as the other.

**Test Evidence:** `crates/qinfer-core/src/sweetspot.rs` — 6 additional tests. Live: the knee is OLMoE at 245 tok/s; the 20 tok/s answer is Qwen3 Coder 30B-A3B at 123 tok/s, and the output says why.
**Rollback:** `DEFAULT_USABLE_TPS` is a constant; setting it to 0.0 restores pure knee behaviour.

---

## T-016 — Setup ladder

A wizard asks questions; a ladder measures. The order is imposed by the data dependencies, not by the PRD's diagram: the fingerprint cannot be collected before the bandwidth probe, because it contains the probe's result.

**Acceptance criteria**
- *Given* a run, *when* it completes, *then* the stages occur in dependency order and the run ends with `Done`.
- *Given* a stage, *when* it starts, *then* `Begin` is emitted before the work so a slow rung is visible while it is slow, not only after it finishes.
- *Given* the fingerprint collection fails, *when* the ladder runs, *then* it stops at `Identify`, emits `Failed` naming the stage, and **must not** emit a choice.
- *Given* a device report, *when* the budget rung runs, *then* the budget comes from the device's working set rather than half of RAM.
- *Given* a floor nothing reaches, *when* the ladder finishes, *then* the frontier is still emitted so the surface can explain why nothing qualifies.
- *Given* the same catalogue and measurements, *when* the ladder is run twice, *then* it produces the same events.

**Test Evidence:** `crates/qinfer-core/src/setup.rs` — 8 tests against a fake `Measurements`, so the whole ladder runs on a machine with no GPU, no MLX and no macOS.
**Rollback:** additive module; the CLI and dashboard do not call it yet.

---

## T-017 — Dashboard: Frontier tab and a measured budget

The ranked plan list cannot answer "what should I run": ordering by throughput always crowns the smallest artifact. The dashboard needed the second axis, and it was still budgeting with a half-of-RAM fallback, contradicting T-012.

**Acceptance criteria**
- *Given* a machine with plans, *when* the Frontier tab renders, *then* it shows the curve, marks the floor-constrained recommendation, and states that capability is a prior.
- *Given* an 84-column terminal, *when* the tab renders, *then* the provenance column is shed; at 64 the capability column goes too; below that the name truncates — and no row **must** wrap onto a second line.
- *Given* a terminal between 46 and 100 columns, *when* any tab renders, *then* no line exceeds the terminal width and the frame stays closed.
- *Given* a fingerprint with no `available_bytes`, *when* the dashboard budgets, *then* it uses `space::resolve` and labels the source rather than assuming half of RAM.
- *Given* a fingerprint that cannot be planned against, *when* the Frontier tab opens, *then* it prints the reason rather than an empty panel.

**Test Evidence:** `crates/qinfer-tui/tests/snapshot.rs`; also rendered and inspected at 100x22, 80x20, 64x18 and 46x16.
**Rollback:** revert the commit; the tab is additive and the Overview tab is unchanged.

---

## T-018 — Dashboard: the Setup tab carries the ladder

The last piece that made T-016 merely a library. Pressing Enter is the entire "few clicks" promise, and until it worked the dashboard was still showing whoever last ran `fingerprint.py` rather than the machine in front of the user.

**Acceptance criteria**
- *Given* the Setup tab with nothing measured this session, *when* it renders, *then* it shows the five rungs pending and states plainly that nothing here is a measurement, rather than showing months-old numbers with confidence.
- *Given* Enter on the Setup tab, *when* pressed, *then* a ladder run starts on a worker thread and each rung fills in as its event arrives.
- *Given* a run already in flight, *when* Enter is pressed again, *then* no second run starts — two ladders would compete for the bandwidth they are trying to measure.
- *Given* a completed run, *when* the other tabs re-derive, *then* they use the fingerprint that run collected and **must not** fall back to the file on disk.
- *Given* any terminal from 20 to 100 columns, *when* the header renders, *then* the machine name and the tab bar are both still present; the tab bar sheds names for abbreviations for digits before it loses a tab.

**Test Evidence:** `crates/qinfer-tui/tests/snapshot.rs` — 8 tests including `the_header_survives_a_short_terminal`; `crates/qinfer-core/src/setup.rs` — 8 tests; live end-to-end via `cargo run -p qinfer-core --example ladder`, which resolves 17.8 GiB and chooses Qwen3 Coder 30B-A3B at 123 tok/s on the reference machine.
**Rollback:** revert the commit; the tab is additive and Enter is the only binding added.

---

## T-019 — Code-quality measurement

`planner::quality_placeholder` has stood in for a measurement since the seed catalogue: Q4 -> 0.93, Q8 -> 0.99, chosen to look monotonic. They were not wrong and they were not evidence, and no surface could say which. The same function is why `Balanced` cannot distinguish a 0.5B from a 27B.

**Acceptance criteria**
- *Given* a model and the fixed corpus, *when* it is scored, *then* the result carries the corpus hash it was measured against and the token count.
- *Given* two scores from different corpora, *when* they are compared, *then* the comparison returns nothing rather than subtracting two numbers measured on different text.
- *Given* a mean NLL of zero or below, *when* parsed, *then* it is refused — that is a broken probe, not a perfect model.
- *Given* a payload with fewer than 32 tokens scored, *when* parsed, *then* it is refused as too little signal to mean anything.
- *Given* any failure, *when* it is reported, *then* it carries a fix specific to its class; a probe that never ran **must not** prescribe "check the model", and a model that failed to load must not prescribe "install MLX".
- *Given* the same model at two precisions, *when* both are scored, *then* the difference is reported as a signed percentage and the direction is not inverted.

**Test Evidence:** `crates/qinfer-core/src/bench.rs` — 10 tests. Live on the reference machine, same corpus (`599e9c50dff219e0`), 422 tokens:

| artifact | nats/token | perplexity |
|---|---|---|
| Qwen2.5-3B-Instruct-4bit | 0.983 | 2.67 |
| Qwen2.5-3B-Instruct-8bit | 0.904 | 2.47 |

8-bit is **8.0% better on code** than 4-bit — close to the ~6.5% gap `quality_placeholder` was estimating, and now measured rather than asserted.
**Rollback:** additive module; `quality_placeholder` is untouched and the planner behaves as before until a surface feeds it a score.
