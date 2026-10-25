# M2 — Tune & Serve — closed-loop optimisation and a supervised endpoint.

**Objective:** Improve on the plan automatically, and keep the result running.

**Duration:** 5–6 weeks · **Depends on:** M1 · **Unlocks:** M3

## 1. Scope

The autotune loop (search over the knobs that matter, measure, keep the winner), the serving supervisor, and the OpenAI-compatible proxy. This is where the tool stops being an installer and becomes an operating layer.

Tuning must be **closed-loop on the user's machine** — the search space and its effect sizes are documented in PRD §5.2 and the ADR-0003 notes, but the winner is always measured, never assumed.

## 2. Non-goals

- Multi-node serving.
- Kubernetes integration.
- Model fine-tuning.
- Any knob search that cannot be measured on the box.

## 3. Deliverables

| # | Deliverable | Notes |
|---|---|---|
| D2.1 | Autotune over a ranked search space with a time budget | Stops when marginal gain < noise |
| D2.2 | `qinfer tune` writing a new plan + receipt | Reproducible: same inputs → same search |
| D2.3 | Supervisor: restart, backoff, health, readiness | launchd/systemd |
| D2.4 | OpenAI-compatible proxy with backend switching | FR-42 |
| D2.5 | Metrics endpoint (tok/s, TTFT, queue, memory) | FR-43 |
| D2.6 | Drift check against the receipt | FR-47 precursor |
| D2.7 | TUI wizard | FR-51 |

## 4. Acceptance tests

Every item is a test that must exist and pass before the milestone closes. Format is Given/When/Then where the test is behavioural.

| ID | Given | When | Then |
|---|---|---|---|
| AT-2.1 | A Tier-A install | `qinfer tune` runs within budget | MAPE on the tuned config ≤ 25 % |
| AT-2.2 | A tuning run | It completes | The chosen config is measured, and the runner-up is recorded |
| AT-2.3 | A serving plan | The engine process is killed | Supervisor restarts within 5 s; endpoint recovers |
| AT-2.4 | A serving plan | 24 h soak | No leak, no unbounded memory growth, health stable |
| AT-2.5 | A client using the OpenAI API | Requests are sent | Conformance tests pass (chat, completions, streaming, models) |
| AT-2.6 | A model that does not fit | `qinfer up` is attempted | Refuses before download, names the component that overflows |
| AT-2.7 | Two installed plans | `qinfer use` switches | Client sees no change beyond the model name |
| AT-2.8 | A known-good receipt | The machine is thermally throttled deliberately | Drift check flags sustained degradation |

**Noise discipline is a test, not a hope:** the harness must discard the first run, control cache state, and refuse to report a difference below 10 %.

## 5. Exit criteria

- AT-2.1 … AT-2.8 green.
- Tuned configs beat the untuned plan on ≥70 % of attempts.
- 24 h soak clean on Apple and NVIDIA x86.
- A user can go from `qinfer up` to a working OpenAI client in one command.

## 6. Risks specific to this milestone

| Risk | Mitigation |
|---|---|
| Autotune overfits noise | Enforce the 10 % floor; prefer fewer, higher-effect knobs |
| Tuning takes longer than the user tolerates | Hard time budget, early stop, resumable |
| Supervisor diverges from the verified config | Supervisor always launches from the receipt, never from user edits |

## 7. Demo script

`qinfer up`, then `qinfer tune`, then show tok/s improving with the receipt recording both. Kill the server; show it come back. Point an existing client at it without changing code.
