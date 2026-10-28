# Roadmap

The systematic plan. Specs define *what* (`PRD.md`), design defines *how* (`SDD.md`), the backlog defines *now* (`TASKS.md`) — this file defines *order and dependency*.

**Last updated:** 2026-10-26 · **Phase:** M0 complete, M1 not started.

---

## 1. Where we are

| | |
|---|---|
| Milestone | **M0 (Advisor) task-complete** — 10/10 tasks, 8/12 acceptance tests demonstrated |
| Code | 5,900 lines of Rust across `qinfer-core` and `qinfer-tui` |
| Tests | **159 green** — 135 unit, 23 independent verification, 1 doctest |
| History | 19 PRs, every one reviewed; three Critic rounds on the validation layer |
| Machine | `qinfer detect --refresh` → trusted fingerprint; `qinfer plan` → ranked, explained plans |

**What works end to end:** probe the machine, calibrate from measured artifacts, rank plans against real memory, refuse what does not fit with arithmetic, and explain every choice.

**What does not work yet:** it cannot install anything. Q-Infer is still advisory.

---

## 2. Milestone map

| | Goal | Exit gate | Status |
|---|---|---|---|
| **M0** Advisor | Tell the truth about a machine | 12 acceptance tests | 8 demonstrated, 3 partial |
| **M1** Apply | Make a plan real, reversibly | 9 acceptance tests | Not started |
| **M2** Tune & Serve | Improve it, keep it running | 8 acceptance tests | Not started |
| **M3** Fleet & Air-gap | Reproduce it everywhere | 7 acceptance tests | Not started |

---

## 3. M0 closeout — three items, all small

| # | Item | Why it is not yet green | Effort |
|---|---|---|---|
| C-1 | **AT-0.8** unsupported platform | Exit-2 path exists but has only run in a unit test | S |
| C-2 | **AT-0.9** no network I/O | Asserted structurally (no network deps linked), never traced | S |
| C-3 | **Fingerprint roundtrip** | AC struck in Critic round 1; now implementable because the types derive `Serialize` | S |

Do these before M1. They are cheap, and a milestone that closes on a technicality teaches the wrong habit.

---

## 4. M1 — Apply

**Goal:** `qinfer up` installs a verified, pinned, reversible configuration and hands back a receipt.

**The risk profile changes here.** M0 could be wrong harmlessly. M1 touches the user's machine, and "rollback works" is not a claim to make from unit tests.

### Task breakdown

#### T-012 — Artifact resolver and pinned fetch

| | |
|---|---|
| Reversibility | LOW (touches the network and the filesystem) |
| Guarantee | No byte is executed before its SHA-256 matches the catalogue |

**Acceptance criteria**
- *Given* a catalogue entry with a `sha256`, *when* fetched, *then* the digest is verified before the file is used.
- *Given* a tampered artifact, *when* fetched, *then* install refuses and names expected vs actual.
- *Given* an entry whose size is `estimated`, *when* resolving, *then* it is refused — a guess is not an artifact.
- *Given* no network, *when* fetching, *then* it fails fast with exit 6 and names the artifact.
- *Given* any fetch, *then* no `curl | sh` and no implicit elevation.

#### T-013 — Transactional provisioner

| | |
|---|---|
| Reversibility | LOW (the component where half-built is worst) |
| Guarantee | A partially-applied plan never leaves the previous plan unusable |

**Acceptance criteria**
- *Given* a working install, *when* any provisioning step fails, *then* the previous plan still runs.
- *Given* an install in progress, *when* the process is `kill -9`'d, *then* the next invocation resumes or rolls back.
- *Given* a user without sudo, *when* `qinfer up` runs, *then* no elevation is attempted and privileged steps are printed.
- *Given* fault injection at every step, *then* rollback succeeds 100% of the time.

#### T-014 — Verification protocol and receipt

| | |
|---|---|
| Reversibility | MEDIUM |
| Guarantee | `PASS` requires a passing correctness check, not just a fast one |

**Acceptance criteria**
- *Given* a plausible-but-wrong config (speculative decoding emitting degenerate output), *when* verified, *then* the verdict is `FAIL`.
- *Given* a completed install, *when* verified, *then* the receipt carries measured TTFT/TPOT, prediction error, cache state and engine build.
- *Given* a receipt, *when* replayed, *then* the environment can be reconstructed.
- *Given* a thermal throttle during the stability run, *then* sustained figures are reported separately from burst.

#### T-015 — CLI lifecycle

| | |
|---|---|
| Reversibility | HIGH |
| Guarantee | Every installed plan is inspectable and removable without reading docs |

**Acceptance criteria**
- *Given* an installed plan, *when* `qinfer status` runs, *then* it reports health and the receipt it came from.
- *Given* two installed plans, *when* `qinfer rm` removes one, *then* the other is untouched.
- *Given* no install, *when* `qinfer status` runs, *then* it says so without erroring.

#### T-016 — Supervisor and endpoint

| | |
|---|---|
| Reversibility | MEDIUM |
| Guarantee | The thing that was verified is the thing that runs |

**Acceptance criteria**
- *Given* a serving plan, *when* the engine dies, *then* it restarts within 5s.
- *Given* a 24h soak, *then* no unbounded memory growth.
- *Given* an OpenAI client, *when* requests are sent, *then* conformance holds (chat, completions, streaming, models).
- *Given* a `kill -9` of the supervisor, *then* it does not leave an orphaned engine holding the port.

---

## 5. M2 — Tune & Serve (outline)

Closed-loop autotuning with a time budget, the TUI wizard, and drift detection against receipts. Detail belongs in `docs/milestones/m2-tune-and-serve.md`; it should not be decomposed until M1's receipts exist, because tuning is *conditioned on* a verified baseline.

---

## 6. M3 — Fleet & Air-gap (outline)

Bundle export/import for disconnected hosts, fleet drift, `qinfer.yaml` in CI. The air-gap test is binary: it runs in a network namespace with no route, or it did not happen.

---

## 7. Sequencing

```
M0 closeout            M1 Apply                          M2               M3
─────────────          ────────────────────────────────  ───────────────  ────────────
C-1 AT-0.8  ─┐
C-2 AT-0.9  ─┼─▶ T-012 fetch ─▶ T-013 provision ─▶ T-014 verify ─▶ T-015 CLI ─▶ T-016 serve
C-3 roundtrip┘        │              │                 │
                      └──────────────┴─────────────────┘
                            all three are LOW reversibility
                            → full Critic review before merge
```

T-013 is the critical path. T-016 can start once T-014 produces a receipt to launch from.

---

## 8. What is blocked, and on whom

| Blocker | Blocks | Needs |
|---|---|---|
| **No CUDA hardware** | T-010's second half; GB10 certification | One borrowed GPU or a cloud hour |
| **27B needs 16.7 GiB, box has ~5.4 free** | The end-to-end M1 demo on a real user model | ~11 GB of free memory |
| **Windows-ARM installs** | M4 scope | A device |
| **Snapdragon NPU path** | The bundle adapter (ADR-0008) untested in production | One phone over `adb` |

None blocks M1's start.

---

## 9. Standing risks

| Risk | Where it bites | Mitigation |
|---|---|---|
| Upstream churn (vLLM weekly, llama.cpp daily) | T-012/T-013 | Catalog mediates; contract tests per engine |
| Provisioning is irreversible in practice | T-013 | Fault injection is a gate, not a nicety |
| Predictor validated on one platform | T-014 | Receipts record prediction error; CUDA gap stated |
| Review cost scales with surface | all | Proportional to reversibility (AOP §12) |
| Context/session boundaries | all | `Memory.md` bootstrap; M1 starts fresh |

---

## 10. Definition of done, per milestone

A milestone is done when **every acceptance test in its milestone spec has been demonstrated on hardware we own**, the spec linter passes, and `Memory.md` records the board honestly — including what is partial.

Not before.
