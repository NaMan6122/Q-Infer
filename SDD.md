# Q-Infer — System Design Document

**Status:** v0.1 (draft, M0-ready)
**Date:** October 2026
**Companions:** `PRD.md` (what/why) · `docs/adr/` (why this way) · `docs/milestones/` (when) · `docs/research/` (evidence)

---

## 0. How to read this

| Document | Answers | Changes when |
|---|---|---|
| `PRD.md` | What are we building and for whom | Product direction shifts |
| **This SDD** | **How is it structured, and what are the contracts** | **Architecture or interfaces change** |
| `docs/adr/*` | Why this option over the alternatives | A decision is revisited |
| `docs/milestones/*` | What ships when, and how we know it's done | Scope or sequencing changes |
| `docs/research/*` | What is true about the hardware | New evidence arrives |

The PRD is the source of truth for intent. This document is the source of truth for **interfaces**. Where they disagree, the PRD wins and this document is wrong.

---

## 1. Design constraints

These are inherited from the PRD and are non-negotiable in review. They constrain every subsequent decision.

| # | Constraint | Consequence |
|---|---|---|
| C1 | The core never links an ML runtime (ADR-0001) | Engines are subprocesses; all engine knowledge is data or adapters |
| C2 | Every prediction is falsifiable (ADR-0003) | Every plan carries predicted values; every receipt carries the error |
| C3 | Installation is reversible (ADR-0005) | Journaled, isolated, content-addressed, resumable |
| C4 | No implicit privilege | Privileged steps are enumerated, printed, never executed |
| C5 | Offline is a first-class mode | Catalog, plans and bundles all work with zero network |
| C6 | State is inspectable | Versioned JSON/YAML on disk; no database |
| C7 | Capability is per-artifact, not per-version (ADR-0004) | Catalog records arch coverage, not version ranges |
| C8 | We do not certify what we have not run (ADR-0007) | Tier A is hardware-gated |

---

## 2. Decomposition

```
                        ┌─────────────────────────────┐
                        │  CLI · TUI · qinfer.yaml     │
                        └──────────────┬──────────────┘
                                       │
   ┌───────────────────────────────────▼───────────────────────────────────┐
   │ CORE                                                                  │
   │                                                                       │
   │  fingerprint ──▶ catalog ──▶ predictor ──▶ planner                    │
   │       │             │            │             │                      │
   │       │             │            │             ▼                      │
   │       │             │            │        provisioner ──▶ verifier    │
   │       │             │            │             │             │        │
   │       │             │            └─────────────┴─────────────┘        │
   │       │             │                          ▼                      │
   │       │             │                     supervisor                  │
   │       ▼             ▼                          ▼                      │
   │  state dir     signed catalog            engine processes             │
   └───────────────────────────────────────────────────────────────────────┘
```

| Component | Responsibility | Owns | Fails how |
|---|---|---|---|
| **fingerprint** | Produce `cfp@1` from OS probes + microbenchmarks | `fingerprint.json` | Partial probe → field is `null` + reason; never a default |
| **catalog** | Signed, versioned set of models × artifacts × engine versions | `catalog/` | Stale/unverifiable → plans warn and refuse to guess |
| **predictor** | `t_token = t₀ + bytes/(k_cal·BW)` | calibration store | Insufficient data → widen interval or refuse |
| **planner** | Ranked plans under hard constraints + objective | `plans/` | No feasible plan → exit 3 with arithmetic |
| **provisioner** | Fetch, verify, install, isolate, journal, rollback | `envs/` | Any step fails → previous plan intact |
| **verifier** | Smoke → correctness → perf → stability → receipt | `receipts/` | Verdict FAIL; never a silent pass |
| **supervisor** | Process lifecycle, health, restart, metrics | runtime state | Bounded backoff, then degraded (not crash-loop) |
| **proxy** | Stable OpenAI-compatible surface over a swappable backend | — | 503 with the reason, never a silent hang |

**Dependency rule:** the arrow direction is enforced. `fingerprint` knows nothing; `planner` never touches the OS; `provisioner` never predicts. Violations are caught by a module-boundary test.

---

## 3. Data contracts

All contracts are versioned with a `schema` field. **Additive changes only within a major version.** No in-place migration: a reader that sees a newer major refuses rather than guessing.

### 3.1 `qinfer/cfp@1` — Capability Fingerprint

| Field | Type | Unit | Required | Notes |
|---|---|---|---|---|
| `schema` | string | — | yes | `qinfer/cfp@1` |
| `fingerprint_id` | string | — | yes | Content hash; stable for identical machines |
| `identity.os` | enum | — | yes | `macos` \| `linux` \| `windows` |
| `identity.arch` | enum | — | yes | `arm64` \| `x86_64` |
| `identity.virt` | enum | — | yes | `bare` \| `vm` \| `container` \| `wsl` |
| `cpu.isa` | string[] | — | yes | Normalised lowercase: `dotprod`, `i8mm`, `sve2`, `sme2`, `amx_bf16` |
| `cpu.physical_cores` | int | count | yes | |
| `accelerators[].arch_id` | string | — | yes | `sm_121`, `gfx1151`, `X1E80100`, `apple-m5-pro` |
| `accelerators[].compute_cap` | [int,int] | — | no | NVIDIA only |
| `accelerators[].memory.total_bytes` | int | bytes | yes | |
| `accelerators[].memory.unified` | bool | — | yes | Gates the fit model |
| `accelerators[].bandwidth.single_thread_gbs` | float | GB/s | yes | |
| `accelerators[].bandwidth.saturated_gbs` | float | GB/s | yes | The number the predictor uses |
| `accelerators[].bandwidth.spread_pct` | float | % | yes | >15 % ⇒ low confidence |
| `accelerators[].bandwidth.method` | string | — | yes | Reproducibility |
| `runtime_gaps[]` | object[] | — | yes | `{component, issue, consequence, evidence}` |
| `provenance.<field>` | enum | — | yes | `exact` \| `measured` \| `inferred` \| `unknown` |

**Invariant:** `saturated_gbs ≥ single_thread_gbs`. A fingerprint violating this is corrupt and must be rejected — it means the thread sweep did not run.

**Invariant:** if `bandwidth.spread_pct > 15` or swap usage exceeds 50 % of RAM, `provenance.bandwidth` must be `unknown`, not `measured`.

### 3.2 `qinfer/plan@1`

| Field | Type | Required | Notes |
|---|---|---|---|
| `plan_id` | string | yes | Stable hash of inputs + catalog version |
| `objective` | enum | yes | `max_speed` \| `balanced` \| `max_context` \| `min_latency` \| `min_power` \| `fit` |
| `inputs` | object | yes | `{model, ctx, concurrency, quality_floor}` |
| `selection` | object | yes | `{engine, engine_version, distribution, artifact, artifact_hash, device}` |
| `predicted` | object | yes | `{ttft_ms, decode_tps, peak_bytes}` each `{p50, p10, p90}` |
| `flags` | object | yes | Engine-specific, validated by the adapter |
| `rationale` | string[] | yes | **Non-empty; planner refuses to emit without it** |
| `risks` | string[] | yes | Derived from `runtime_gaps` |
| `fallbacks` | plan_id[] | yes | ≥2 for any primary plan |
| `catalog_version` | string | yes | Pinning |

### 3.3 `qinfer/receipt@1`

| Field | Type | Required | Notes |
|---|---|---|---|
| `receipt_id` | string | yes | |
| `plan_id`, `fingerprint_id` | string | yes | The join keys |
| `installed[]` | object[] | yes | `{component, version, digest}` |
| `measured` | object | yes | `{ttft_ms{p50,p95}, decode_tps{p50,p95}, prefill_tps, peak_bytes}` |
| `measured.sustained` | object | no | Separate from burst; `{decode_tps, throttled}` |
| `prediction_error` | object | yes | `{decode_pct, ttft_pct}` |
| `environment` | object | yes | Cache state, engine build hash, power mode, swap at measurement |
| `verdict` | enum | yes | `PASS` \| `FAIL` \| `INCONCLUSIVE` |
| `signature` | string | yes | Detached signature over the canonical JSON |

**Invariant:** `verdict: PASS` requires a passing correctness check. A fast-but-wrong run is `FAIL`, always.

### 3.4 `qinfer/catalog-entry@1`

Keyed on `(model_family, artifact_hash)` — never on `(model, quant)`, because two publishers' "Q4_K_M" are different artifacts.

Required: `artifact.format`, `artifact.quant`, `artifact.bytes_on_disk`, `artifact.hash`, `artifact.license`, `model.arch_params{layers, kv_heads, head_dim, hidden, vocab, ctx_max}`, `model.params{total, active}`, `quality.delta_vs_bf16{metric, value, method}`, and **`coverage[]`** — the architectures the artifact can actually execute on (cubin targets, bundle chipset, declared-by-publisher vs verified-by-us).

---

## 4. Interfaces

### 4.1 The engine adapter

Two classes, because bundle runtimes have a different parameter model (ADR-0008).

```rust
/// Capabilities this engine has *on this machine* — not on the vendor's site.
fn capabilities(&self, fp: &Fingerprint) -> CapabilitySet;

/// Candidate plans. Must return an empty vec (with a reason) rather than
/// a plan it cannot honour.
fn plan(&self, req: &PlanRequest, cat: &Catalog) -> Result<Vec<PlanCandidate>>;

fn provision(&self, plan: &Plan, ctx: &mut ProvisionCtx) -> Result<Installed>;
fn launch(&self, installed: &Installed) -> Result<Child>;
fn measure(&self, child: &Child, proto: &Protocol) -> Result<Measurement>;
fn serve(&self, installed: &Installed, sb: &Supervisor) -> Result<Endpoint>;
```

| Class | Examples | Context / precision | Planner behaviour |
|---|---|---|---|
| **Dynamic** | llama.cpp, MLX, vLLM, SGLang, Ollama | Supplied at launch | Full search over ctx/quant/KV/batch |
| **Bundle** | QAIRT/QNN, TensorRT-LLM engines | **Baked at compile time** | Selects an existing bundle; ctx/precision are *outputs* |

For bundle engines, `plan()` may only return candidates whose artifact exists for this exact `arch_id`/chipset. Offering a bundle that does not exist is a planner bug, not a user error.

### 4.2 CLI contract

- Every command supports `--json`, emitting a single versioned envelope: `{schema, command, ok, data, error}`.
- Exit codes are stable (PRD §10.5). `8` (drift) is a *success* for CI purposes but a non-zero shell code by design.
- `--offline` must be honoured by every command; a command that cannot work offline fails fast with code 6 and names the artifact it needs.

### 4.3 Supervisor ↔ engine

Health is determined by three probes, in order: process liveness → HTTP readiness → one-token generation. A runtime that answers `/health` but cannot generate is **not** healthy; this catches the common case of a model that loaded but cannot allocate KV.

### 4.4 TUI ↔ core

The dashboard renders `cfp@1`, `plan@1`, `receipt@1` and the spike results. It owns no state beyond view selection and reads nothing the CLI cannot also emit (ADR-0009).

| Rule | Consequence | Enforced by |
|---|---|---|
| Same process, no socket | No daemon to secure, no port to conflict | Architecture test: the TUI binary opens no listener |
| Read-only by default | No mutation without explicit confirmation | Keymap test: no destructive action on a bare key |
| Every panel is `--json`-reachable | Deleting the TUI loses no fact | `--json` equivalence test per panel |
| `--snapshot WxH [--tab]` | CI renders frames via `TestBackend` | Golden-frame test at 64×20, 96×24, 140×40 |
| Layout breakpoints | ≥76 cols two-column; <76 stacked; <84 the bench table sheds columns | Snapshot tests at each breakpoint |

Column widths and labels are presentation; the *values* come from typed structs in `qinfer-core`, so the dashboard cannot drift from the data model.

---

## 5. State and lifecycle

```
~/.qinfer/
  fingerprint.json            # + .prev for drift diffing
  calibration.json            # k_cal per (backend, probe, arch, quant, shape)
  catalog/                    # signed, versioned
  plans/<plan_id>.json
  envs/<plan_id>/             # isolated install prefix
  receipts/<receipt_id>.json
  bundles/
  journal/<plan_id>.jsonl     # provisioning journal, fsync'd per step
  logs/
```

**Plan lifecycle:**

```
draft ──plan──▶ planned ──apply──▶ applying ──ok──▶ applied
                                     │
                                     └─fail──▶ rolled_back
applied ──verify──▶ verified ──serve──▶ serving
                                          │
                    degraded ◀────────────┘  (health probe fails)
                       │
                       └──rm──▶ retired
```

Transitions are recorded in the journal. A crash leaves the last durable state; `qinfer status` reconciles the journal against reality and reports the divergence rather than hiding it.

---

## 6. Error taxonomy

Errors are a product surface. Each carries: **what happened** (fact), **why** (cause), **what to do** (action), and a stable doc anchor.

| Code | Class | Example cause | User-facing action |
|---|---|---|---|
| `E-DET-001` | Detection | Probe returned empty on a supported OS | `--diagnose` dump attached |
| `E-DET-002` | Detection | Bandwidth unstable (>15 % spread) | Re-run when idle; do not trust prediction |
| `E-ARCH-001` | Architecture | No kernels for `arch_id` | Choose a different artifact or engine |
| `E-FIT-001` | Fit | KV cache exceeds available memory | Reduce ctx, quantise KV, smaller model |
| `E-CAT-001` | Catalog | Stale/unverifiable entry | Update catalog or pin an older one |
| `E-PRV-001` | Provisioning | Checksum mismatch | Re-fetch; do not proceed |
| `E-PRV-002` | Provisioning | Network loss mid-install | Resume (idempotent) |
| `E-PRV-003` | Provisioning | Step requires elevation | `--print-sudo` and run manually |
| `E-VRF-001` | Verification | Correctness mismatch | Config is wrong, not the hardware |
| `E-VRF-002` | Verification | Performance below floor | Try the recorded fallback plan |
| `E-LIC-001` | Licence | Gated model / vendor SDK absent | Accept licence or supply SDK |

**Rule:** no error message may say only "failed". If we do not know the cause, the message says what we observed and asks for the diagnostic bundle.

---

## 7. Test strategy

| Layer | What it proves | Where it runs |
|---|---|---|
| **Unit** | Parsing, arithmetic, schema validation | Every commit |
| **Property** | Invariants (fit arithmetic never overflows; planner never emits without rationale; plan never references an artifact lacking arch coverage) | Every commit |
| **Contract** | Each engine adapter against each pinned engine version | Nightly + on version bump |
| **Fault injection** | Rollback at every provisioning step | Every commit (simulated) |
| **Hardware-in-loop** | Detect → plan → apply → verify → receipt | Nightly on Tier A only |
| **Golden receipt** | Re-running a recorded plan reproduces the same verdict | Nightly |
| **Air-gap** | Bundle apply in a network namespace with no route | Weekly |
| **Regression** | **Rank order** of plans matches measured order | Per release |

**The rank-order test is the load-bearing one.** Point accuracy can drift; if the planner stops ordering plans correctly, the product has no reason to exist (PRD §5.2.4).

**Determinism policy:** any test that measures performance must declare cache state, discard the first run, and assert a 10 % noise floor before claiming a difference.

---

## 8. Traceability

| PRD requirements | ADR | Milestone | Primary tests |
|---|---|---|---|
| FR-1 … FR-9 (detect) | 0002 | M0 | AT-0.1, AT-0.2 |
| FR-10 … FR-16 (catalog) | 0004 | M0 | AT-0.3, property tests |
| FR-17 … FR-25 (plan) | 0003 | M0 | AT-0.3 … AT-0.7, rank order |
| FR-26 … FR-33 (apply) | 0005 | M1 | AT-1.1 … AT-1.4, AT-1.8 |
| FR-34 … FR-40 (verify/record) | 0006 | M1, M2 | AT-1.5 … AT-1.7, AT-2.1 |
| FR-41 … FR-45 (serve) | 0001 | M2 | AT-2.3 … AT-2.7 |
| FR-46 … FR-49 (fleet) | 0007 | M3 | AT-3.1 … AT-3.7 |
| FR-50 … FR-54 (surfaces) | — | M0–M2 | CLI contract tests |

---

## 9. Versioning and release

| Artifact | Scheme | Compatibility rule |
|---|---|---|
| Core binary | SemVer | Minor may add commands/flags; major may change schemas |
| Catalog | CalVer (`2026.10.1`) | Users may pin; plans record the pin |
| CFP / Plan / Receipt | `schema` URI with major | Readers refuse unknown major |
| Engine adapters | Track engine major | Adapter declares the range it supports |

**Supply chain:** releases are signed, with an SBOM per release. The catalog is signed separately so it can update without a core release (ADR-0004). Receipts are signed so a third party can verify a claimed benchmark.

---

## 10. Open design questions

| # | Question | Blocks | Current thinking |
|---|---|---|---|
| S1 | Do we probe bandwidth natively on-device (Metal/CUDA) or keep the CPU probe + `k_cal`? | M1 | CPU probe is enough if `k_cal` is allowed >1; native probe is better but 3× the code |
| S2 | How does the catalog represent "verified by us" vs "declared by publisher"? | M0 | Two fields, never merged; UI must distinguish |
| S3 | What is the minimum viable catalog curation process? | M0 | Manual review + one benchmark per entry; automate later |
| S4 | Do we ship a `qinfer doctor` that repairs drift, or only reports it? | M2 | Report first; repair is a footgun |
| S5 | How do we test Windows-ARM without hardware? | M4 | GitHub ARM64 runners for detect; install paths stay Tier C |

---

## 11. Deliberately not designed yet

Kubernetes operator, multi-node coordination, a hosted control plane, multi-tenant isolation, model hosting, and any form of scheduling. These are excluded by the PRD's non-goals and by ADR-0007 (integrate, do not orchestrate). Designing them now would be speculative work that the first three milestones will invalidate.

---

*End of SDD v0.1. The next engineering artefact is the M0 skeleton: the `cfp@1` schema as code, the bandwidth probe with its thread sweep, and the planner's constraint solver — each with the acceptance tests from `docs/milestones/m0-advisor.md` wired in from the first commit.*
