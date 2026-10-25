# M1 — Apply — transactional provisioning, verification, receipts.

**Objective:** Take a plan and make it real, reversibly, with proof.

**Duration:** 6–8 weeks · **Depends on:** M0 · **Unlocks:** M2

## 1. Scope

The provisioner, the verifier, and the receipt. Targets Apple Silicon (Tier A), NVIDIA x86 (Tier A once hardware exists), and llama.cpp on Linux CPU (Tier B). Containers and native installs are both first-class.

The invariant that defines this milestone: **a partially-applied plan never leaves the previously working plan unusable.**

## 2. Non-goals

- Autotuning beyond the planner's initial guess.
- Supervised serving as a service (a foreground process only).
- Fleet features.
- Any platform outside the three above.

## 3. Deliverables

| # | Deliverable | Notes |
|---|---|---|
| D1.1 | Provisioner with journaling, isolation, resume, rollback | ADR-0005 |
| D1.2 | Content-addressed artifact fetch + signature verification | Refuses unpinned |
| D1.3 | `--print-sudo` and a hard no-implicit-elevation guarantee | Audit test |
| D1.4 | Verifier: smoke → correctness → perf → stability | PRD §5.5 |
| D1.5 | `qinfer/receipt@1` with signing | ADR-0006 |
| D1.6 | `qinfer bench`, `qinfer ls`, `qinfer status`, `qinfer rm` | |
| D1.7 | Exit-code taxonomy wired end to end | PRD §10.5 |

## 4. Acceptance tests

Every item is a test that must exist and pass before the milestone closes. Format is Given/When/Then where the test is behavioural.

| ID | Given | When | Then |
|---|---|---|---|
| AT-1.1 | A working install | Any provisioning step fails | Previous plan is still runnable; `qinfer status` reports healthy |
| AT-1.2 | An install in progress | The process is `kill -9`'d | Next invocation detects the journal and offers resume or rollback |
| AT-1.3 | A tampered artifact | Install runs | Refuses before execution, names the expected and actual hash |
| AT-1.4 | A user with no sudo rights | `qinfer up` runs | No elevation is attempted; privileged steps are printed instead |
| AT-1.5 | A deliberately wrong-output config (spec-decode trap) | Verification runs | Correctness check fails; receipt verdict is FAIL |
| AT-1.6 | A completed install | Receipt is validated | Conforms to `receipt@1`, signature verifies, prediction error recorded |
| AT-1.7 | 20 consecutive installs on Tier A | Measured | ≥80 % reach serving on first attempt |
| AT-1.8 | Fault injection at every provisioning step | Test suite runs | 100 % rollback success |
| AT-1.9 | An install is interrupted by network loss | Retry runs | Resumes without re-downloading completed artifacts |

**Fault injection is mandatory in CI:** simulated network failure, disk-full, checksum mismatch, permission denied, and process kill at each of the N steps.

## 5. Exit criteria

- AT-1.1 … AT-1.9 green.
- ≥80 % first-attempt success across ≥30 real installs on Tier-A hardware.
- Median time-to-serving < 15 min for artifacts ≤ 20 GB.
- Zero P0 security findings in the provisioning path review.

## 6. Risks specific to this milestone

| Risk | Mitigation |
|---|---|
| Isolation breaks vendor installers | Documented exceptions with manual steps; never silently escalate |
| Verification is slow enough that users skip it | Default protocol ≤ 12 min; `--quick` for a 90 s smoke |
| Receipt schema churn | Version it from day one; never migrate in place |

## 7. Demo script

Install on a Mac, show the receipt, then deliberately corrupt the artifact hash and show the refusal. Then `kill -9` mid-install and show the recovery. Total: 4 minutes, and it is the most convincing demo we have.
