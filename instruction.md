# instruction.md — v5 (consolidated)

> **Status: ACTIVE v5.0 — ratified 2026-10-26.**
> This file reconstructs the operating protocol from two sources: `AOP-v5-AutoSuper.md` (a diff proposal against v4) and the v4 sections that diff references. **Anything marked `⚠R` is reconstructed**, not quoted from the original v4, and remains open to correction. It is the root of trust and may only be amended through §15.
>
> **Ratification log:** OQ-05 (stacked PRs) accepted 2026-10-26 and applied to §11.

---

## §0 Purpose and authority

This document governs how work is executed in this repository. It ranks above ADRs and specs; it does not rank above the human.

Its single design goal: **replace human presence at checkpoints with mechanical validation, without losing the audit trail a human would have produced.** Reversibility tiering, structural validation, independent review, and append-only records are the substitute. Where they are not a sufficient substitute, §14 says so explicitly.

---

## §1 Bootstrap Protocol

Every session, before any other action:

1. Read `Memory.md` — state, last file touched, immediate next step.
2. Read `spec-index.md` — what is `ACTIVE`, what is `DRAFT`.
3. Read the tail of `dev-changelog.md` — what changed recently and why.
4. Read any ADR whose `Review trigger` has fired.
5. Confirm the branch matches the work: `main` is stable; work happens on `feat/`, `fix/`, `test/`, `chore/`, `docs/` branches.
6. Resume from `Memory.md`'s **Immediate next step**. Do not re-plan work that is already specced.

If step 1–4 surfaces a gap the Charter does not answer, do **not** interrupt the human. Log an `Open Question` and continue with the next `PENDING` task (§8).

---

## §2 Spec format

Every spec and task carries:

| Field | Rule |
|---|---|
| **ID** | `T-NNN` for tasks, `ADR-NNNN` for decisions, `M-N` for milestones |
| **Status** | `PENDING` · `READY` · `IN PROGRESS` · `DONE` · `ABANDONED` · `BLOCKED` |
| **Acceptance Criteria** | Given/When/Then. At least one **negative** criterion ("should NOT occur") |
| **Risks** | Non-empty, or `NONE` with a one-line justification |
| **Rollback** | How to undo. `NONE` is not a valid value |
| **Reversibility** | `LOW` · `MEDIUM` · `HIGH` (§12) |
| **Test Evidence** | See §2.1 |
| **Dependencies** | IDs of tasks/specs that must be `DONE` first |

### §2.1 Test Evidence

```
Test Evidence: [unit test file path(s)] + [simulation scenario ref, if SIGNIFICANT] | N/A — reason
```

`Manual sign-off by human` is **not** a valid value in v5 — the human is not in the loop (§7).

---

## §3 Roles

| Role | Owns | Must not |
|---|---|---|
| **Orchestrator** (main agent) | `Memory.md`, `dev-changelog.md`, `spec-index.md`, ADRs, PRD/SDD, Charter, all gate decisions | Implement tasks directly |
| **Workers** | One `T-NNN` against its spec, tests, status packet | Decide spec status, touch this file, make architecture calls |
| **Critic** | Adversarial review of promotions, `LOW`-reversibility ADRs, phase completions | Implement anything |
| **Verification** | Independent tests and simulation harnesses against acceptance criteria | Test a feature it wrote |
| **Consultation** | One specific doubt via a disposable headless instance; capacity relay | Carry standing state |

Critic and Verification are **separate roles with fresh context** (Charter Q4). The Orchestrator does not mark a task `DONE` on the implementing worker's self-reported tests alone.

---

## §4 Autonomy decision rules

| Gate | Resolves without pausing when… |
|---|---|
| **G1 — Spec confirmed** | (a) structural validator passes — Given/When/Then, Risks, Rollback, deps resolved; (b) Critic signs off alignment with the Charter; (c) Test Evidence meets the tier's bar (§7). Any failure → stays `DRAFT`, task deferred, logged as `Open Question`. |
| **G2 — Architecture change** | `HIGH` reversibility: proceed, log. `LOW`: Critic review + ≥2 genuinely evaluated options → proceed, tagged `auto-approved, LOW reversibility` in the Session Summary. |
| **G3 — PRD impact** | **Never auto-resolved.** See §14. Routed to the human asynchronously; never a mid-session stop. |
| **G4 — Blocker** | Ladder: alternate approach → consultation instance → replan/reassign → `ABANDONED` with reasoning + rollback note, logged, move to next task. Human contact is the last rung only. |
| **G5 — Milestone complete** | Auto-generate a Phase Completion Report, proceed to the next phase. Report awaits async review. |

---

## §5 Non-negotiables

1. Specs precede implementation. No code without an `ACTIVE` spec or a `READY` task. ⚠R
2. Every task has a Rollback. ⚠R
3. Append-only history: `dev-changelog.md` is never rewritten. ⚠R
4. One task in `IN PROGRESS` per worker. ⚠R
5. **Human checkpoint gates are not skipped — except G3.** *(v5 change)*
6. Tests are part of done, not a follow-up. ⚠R
7. `Memory.md` is updated before any session ends or relays. ⚠R
8. Blockers follow the escalation ladder, never a silent stop. ⚠R
9. No task is marked `DONE` on the implementer's self-report alone. *(v5 §7)*
10. Records beat recollection: if it is not in a file, it did not happen. ⚠R
11. **Only the Critic's sign-off + structural validator may move a spec `DRAFT → ACTIVE`.** *(v5 change; was "only a human")*
12. **No agent may edit `instruction.md`.** Propose changes as an `Open Question`; the human amends. *(unchanged, scope reworded)*
13. Credential-bearing files (`.env`, CI secrets) are human-only (§9). ⚠R
14. Irreversible actions require an explicit Rollback entry before execution. ⚠R
15. Every commit is atomic and conventional (§11). ⚠R

---

## §6 Session-init interrogation

Run once at project start, or at the start of a phase the Charter does not cover:

1. **Research** the spec stack, codebase, ADR history, `dev-changelog.md`.
2. **Brainstorm** architecture options, edge cases, and every underdetermined point.
3. **Compile ONE batched question set** — closed-form where possible.
4. **Human answers once.**
5. **Lock answers into `init-charter.md`**, versioned like a spec.
6. **Zero synchronous human contact** for the scope the Charter covers.

Not run every session. Mid-phase gaps become `Open Question`s queued for the next interrogation.

---

## §7 Verification and test evidence

- **Unit tests** are the floor for `STANDARD` and `SIGNIFICANT` tasks.
- **Simulation harnesses** are required for `SIGNIFICANT` tasks and anything auto-promoted through G1/G2: seeded data, mocked dependencies, realistic load where blast radius warrants, exercising both positive and negative acceptance criteria. "Ran it, looked fine" is not evidence.
- The **Verification sub-agent** builds and runs these independently of the implementer.
- The G5 report lists simulation coverage per significant task, not just "all specs implemented".

---

## §8 No blocking stops

**§8.1 Capacity relay.** On approaching context limits: finalise `Memory.md`, spin a successor headlessly, log `Relayed to successor at HH:MM — reason: capacity`, continue.

**§8.2 Consultation before deferral.** For any doubt: self-resolve → consult a disposable headless instance with only the relevant excerpt → if confident, proceed and log the question *and* answer in `Memory.md` → if not, one more framing or hand to the Critic → only then an `Open Question` under G4. Never a synchronous stop.

**§8.3 Not applicable to §14 carve-outs** — those are authority boundaries, not uncertainty.

---

## §9 Credentials

Never read, write, or transmit `.env`, CI secret stores, private keys, or tokens. Surface their presence to the human and stop work on that path. This is a §14 carve-out, not a doubt.

---

## §10 Record-keeping

| File | Rule |
|---|---|
| `Memory.md` | Rewritten each session: state, last file, next step, decisions, relay log |
| `dev-changelog.md` | Append-only, newest last |
| `spec-index.md` | Status of every spec, ADR and milestone |
| `TASKS.md` | Atomic backlog with acceptance criteria |
| `docs/adr/` | One decision per file, with alternatives and a review trigger |

---

## §11 Git discipline

- **Conventional commits**: `feat(scope):`, `fix(scope):`, `test(scope):`, `docs(scope):`, `chore(scope):`, `refactor(scope):`.
- **Atomic**: one logical change per commit; never mix a docs change with a behaviour change.
- **Branching**: `main` is always green. Work on `feat/…`, `fix/…`, `test/…`, `chore/…`, `docs/…`; merge with `--no-ff` so the branch is visible in history.
- A task's commits land on its branch and merge only when the task is `DONE` with Test Evidence attached.
- **Stacked PRs** (OQ-05, ratified 2026-10-26): a branch may base on its parent branch rather than `main`. The PR body must declare `Depends on #N`. Merges happen **bottom-up** — no PR merges before its base. After any change to a lower layer, rebase the stack and force-push with `--force-with-lease`, never plain `--force`.
- **Commit messages state the why**, not the what: the diff already shows the what.

---

## §12 Reversibility

| Tier | Meaning | Gate |
|---|---|---|
| `HIGH` | Revert a commit / delete an additive file | Log only |
| `MEDIUM` | Requires a migration, config change, or re-recording fixtures | Log + rollback note |
| `LOW` | Schema/contract, data model, public CLI semantics | Critic + ≥2 options documented (G2) |

---

## §13 Communication

The Session Summary is the only thing a human skims. It must surface, **in order**, anything risk-flagged since the last check-in:

1. `LOW`-reversibility auto-approvals
2. `ABANDONED` tasks
3. New `Open Questions`
4. What is active now
5. Immediate next step

Tone: state facts and decisions; no narration of effort.

---

## §14 Human carve-outs

Three, deliberately narrow:

| Carve-out | Why | How it reaches the human |
|---|---|---|
| **PRD impact (G3)** | A wrong product call is a business decision that compounds silently | Async: logged, surfaced at next check-in. Option A (Charter). |
| **Credential-bearing files** | Blast radius does not compress | Stop that path, surface immediately |
| **Editing `instruction.md`** | Root of trust; the operator must not rewrite the contract | Propose as `Open Question` |

Everything else automates.

---

## §15 Amendment

Raise an `Open Question` describing the proposed change and its rationale. The human edits this file. Agents never do (Non-Negotiable #12).

---

*Reconstruction notes: §5 items 1,2,3,4,6,7,8,10,13,14,15 are inferred from AOP v5 §5 ("unchanged — record-keeping and ordering discipline"). §11 is new in this repository and not part of v4. Ratify or correct on review.*
