# AOP v5 Proposal — Removing Human-in-the-Loop Gates

> Companion to `instruction.md` v4. This is a proposal, not a ratified spec — treat it as a DRAFT that needs your G1-equivalent (i.e., your own) approval before it becomes v5.

## 0. What "full autonomy" actually has to change

v4 has two different kinds of human touchpoint, and they don't convert the same way:

1. **Synchronous blocking gates** — G1–G5, branch-confirm, blocker escalation. These pause execution and wait for a reply. These are the ones you're asking to remove.
2. **Ownership rules** — "only a human may do X" (spec DRAFT→ACTIVE, edit `instruction.md`, touch credential-bearing files). These aren't about pacing, they're about *who holds authority*. Removing them is a different, higher-stakes decision than removing a pause.

The proposal below converts (1) wholesale. For (2) I'm recommending you keep three narrow exceptions rather than automating everything — flagged explicitly in §4, with reasoning, so you can override any of them if you disagree.

The throughline: v4 already started this move with the reversibility-gated ADR (only `LOW` reversibility triggers G2). v5 generalizes that idea — **reversibility and mechanical validation replace human presence as the thing that gates action**, everywhere it safely can.

---

## 1. Session-Init Interrogation Phase (new — replaces ad hoc "ask when ambiguous")

Triggered at true project start, or at the start of a phase/scope big enough that the existing Charter (see below) doesn't cover it. **Not** run every session — regular sessions keep the existing §1 Bootstrap Protocol unchanged.

```
1. Research: read the full existing spec stack, codebase, and any prior
   dev-changelog / ADR history. External research if the domain requires it.
2. Brainstorm: the main agent enumerates architecture options, edge cases,
   and every point where product intent is underdetermined — before writing
   a single spec.
3. Compile ONE batched, structured question set from step 2 — not an
   iterative back-and-forth. Precise, closed-form where possible (mirrors
   §13's existing "2–3 concrete choices" style).
4. Human answers once.
5. Answers are locked into a new file, `init-charter.md`, versioned like a
   spec. This is the baseline every later autonomous decision is checked
   against.
6. From this point, zero synchronous human contact for the scope this
   Charter covers.
```

If bootstrap in a later session surfaces a genuine gap the Charter doesn't answer, it does **not** interrupt the human mid-work — it's logged as an `Open Question` and queued for the *next* interrogation phase (§3 covers what happens to the task in the meantime).

---

## 2. Sub-Agent Architecture (new)

| Role | Owns | Does not do |
|---|---|---|
| **Main Agent (Orchestrator)** | `Memory.md`, `dev-changelog.md`, `spec-index.md`, ADRs, `PRD.md`/`TDD.md`, the Charter, all gate-equivalent decisions (§3) | Does not implement tasks directly — delegates atomic `T-NNN` units |
| **Sub-Agent Workers (Executors)** | Implementing one task against its spec + acceptance criteria, running tests, reporting back | Does not decide spec status, does not touch `instruction.md`, does not make architecture calls |
| **Critic Sub-Agent (new role)** | Adversarial review of spec promotions, `LOW`-reversibility ADRs, and phase completions | Does not implement anything — read-only judgment, ideally fresh-context or a different model to avoid the orchestrator rubber-stamping itself |
| **Verification Sub-Agent (new role, see §7)** | Building and running unit tests and real-world simulation harnesses against a task's acceptance criteria, independent of the worker that wrote the code | Does not implement the feature it's testing — same objectivity reason as the Critic |
| **Consultation Agent (on-demand, see §8)** | Answering a specific doubt via a disposable headless `anv` instance, and self-relaying a session at capacity to a successor instance | Does not carry standing state — spun per-doubt or per-relay with only the specific question or handoff context, not full project history |

Workers report back the structured status-packet format you already built for the GHCP orchestrator/executor split — this reuses that, it doesn't invent a new one. The Critic Sub-Agent is the actual substitute for the human at every old gate: it's the "mechanical enforcement" your v3 analysis said was the substantively valuable target, rather than just automating authorship.

---

## 3. Autonomy Decision Rules — replaces §10 Human Checkpoint Gates

| Gate | v4 (pauses for human) | v5 proposal (resolves without pausing) |
|---|---|---|
| **G1 — Spec Confirmed** | Human approves DRAFT→ACTIVE | Auto-promotes when **(a)** a structural validator passes — acceptance criteria in Given/When/Then form, `Risks` non-empty or justified `NONE`, `Rollback` filled, dependency chain resolved — **(b)** the Critic Sub-Agent signs off that the spec aligns with the Charter, and **(c)** Test Evidence meets the tier's bar (§7). Fails any of these → spec stays `DRAFT`, task deferred (not blocked indefinitely), logged as `Open Question` for the next interrogation phase. |
| **G2 — Architecture Change** | Human approves `TDD.md` changes / `LOW` reversibility ADRs | `HIGH` reversibility: proceeds exactly as v4 already allows (logged, no pause). `LOW` reversibility: requires Critic Sub-Agent review + ≥2 genuinely evaluated options documented — then proceeds automatically, tagged in the Session Summary as "auto-approved, LOW reversibility" so it's visible on the next async check-in. |
| **G3 — PRD Impact** | Human approves any product-intent deviation | **The one case I'd push back on removing.** See §4. |
| **G4 — Blocker Resolution** | Human clears the blocker | Escalation ladder (full detail in §8): worker retries with an alternate approach → consults a disposable headless agent via `anv` → if still unresolved, main agent replans or reassigns → if still stuck, `ABANDONED` with full reasoning + rollback note, logged as `Open Question`, and the session moves on to the next `PENDING` task. Human contact is the last rung, reached only if consultation doesn't resolve it — never a synchronous stop. |
| **G5 — Milestone Complete** | Human signs off on phase completion | Auto-generates a Phase Completion Report (same shape as the existing Session Summary) and proceeds straight into the next phase. Report sits available for async review whenever the human next looks — it doesn't block. |

---

## 4. What I'd recommend keeping human-gated

Three narrow exceptions, each with a specific reason — not a hedge, a deliberate seam:

- **G3, PRD Impact.** Everything else in this system is recoverable through `Rollback` fields and append-only history. A wrong PRD-level call is a business decision, not an engineering one, and it compounds silently — by the time it surfaces, a lot of `ACTIVE` work may be built on it. If you want this gateless too, the honest way to do it is to expand the Charter in session-init to cover foreseeable PRD forks up front, and treat any genuinely novel PRD-level fork mid-session as `ABANDON`-and-log rather than pause. That's Option B — real, but riskier, and I'd only take it once the Charter step in §1 has proven itself over a few phases.
- **Credential-bearing files (§9).** The existing "pause and surface" rule for `.env`/CI config costs almost nothing to keep and the blast radius of getting it wrong doesn't compress the way spec or architecture risk does.
- **Editing `instruction.md` itself (Non-Negotiable #12).** This file is the root of trust for the whole autonomous system. Letting the agent that operates *under* the contract also rewrite the contract removes the one fixed reference point everything else in v5 depends on. The agent can still propose changes as an `Open Question`, same as v4.

Everything else in v4's gate list, I don't see a reason not to automate.

---

## 5. Non-Negotiables — proposed diff from v4's 15

**Change:** #5 ("Human checkpoint gates are not skipped") → narrows to G3 only, per §4.
**Change:** #11 ("Only a human may move a spec from DRAFT to ACTIVE") → becomes "Only the Critic Sub-Agent's sign-off + structural validator may move a spec from DRAFT to ACTIVE" (§3, G1).
**Unchanged:** #1, 2, 3, 4, 6, 7, 8, 9, 10, 13, 14, 15 — none of these were human-pause mechanics, they were record-keeping and ordering discipline, and none of them need a human to enforce.
**Unchanged, reworded scope only:** #12 stays exactly as-is per §4.

---

## 6. §13 Communication Style — one addendum

With no one watching each step, the Session Summary becomes the *only* thing a human skims on a check-in, not a formality. Proposed addition: Session Summary must surface, in order, anything risk-flagged since the last check-in — `LOW`-reversibility auto-approvals, `ABANDONED` tasks, and new `Open Questions` — before the routine "what's active" line. Buried-but-technically-logged isn't good enough once nobody's reading the Task Log in real time.

---

## 7. Verification & Test Evidence — replaces "Manual sign-off by human"

v4's `Test Evidence` field allows three values: a test file path, manual human sign-off, or `N/A` with a reason (§2.1). Removing the human removes the middle option for anything above `TRIVIAL` — nobody's available to sign off, so the bar on the other two has to go up, not stay where it was.

**Unit Tests.** Unchanged in kind from v4's "Tests are part of done" (§9) — every `STANDARD`/`SIGNIFICANT` task's acceptance criteria must be covered by automated tests before `DONE`. What changes is this is now the floor, not one of three interchangeable options.

**Real-World Simulation Tests.** New requirement for `SIGNIFICANT` tasks (§0.1) and anything auto-promoted through G1/G2 — unit tests alone don't catch what manual sign-off used to catch: does the behavior hold up end-to-end, under realistic conditions? For each such task, the Verification Sub-Agent builds a scoped simulation environment — seeded data, mocked external dependencies, realistic concurrency/load where the spec's blast radius calls for it — and exercises it against the spec's actual Acceptance Criteria, including the negative "should NOT occur" criteria the spec format already reserves a line for (v4 §4.2). A simulation pass needs the same discipline as a unit test: defined inputs, an expected outcome, a recorded pass/fail — not a narrative description of "ran it, looked fine."

**Why a separate Verification Sub-Agent.** The same reason the Critic reviews specs instead of the Orchestrator self-certifying them applies here: the worker that wrote the code is a bad judge of whether its own tests are honest. Unit tests and simulation harnesses for `SIGNIFICANT` work are built and run independently, fresh-context or different model, same as the Critic. It reports results back as part of the task's status packet; the main agent does not mark a task `DONE` on the implementing worker's self-reported test results alone.

**Proposed Test Evidence format:**
```
Test Evidence: [Unit test file path(s)] + [Simulation scenario ref, if SIGNIFICANT] | N/A — reason
```
`Manual sign-off by human on YYYY-MM-DD` is dropped as a valid value — it's simply not available without a human in the loop. If a human checks in and wants to verify something by hand anyway, that's fine, it's just no longer something the protocol can rely on as its evidence.

**Tie-in to G1/G5.** The Critic's sign-off (§3, G1) now also checks Test Evidence meets the tier's bar before promoting DRAFT→ACTIVE or marking a task DONE — clean spec structure with thin test evidence doesn't pass. The Phase Completion Report (G5) should list simulation coverage per significant task, not just "all specs implemented."

---

## 8. No Blocking Stops — Headless Consultation Before Human Contact

The behavior you flagged — *"I've reached this session's safe capacity; I'm stopping here... I'll open directly into T-182 next"* — is a blocker in substance even though it isn't one of G1–G5. It doesn't ask a question, but it still ends with the human as the one who has to notice and open the next session. v5 should close that gap the same way it closed the others: the agent relays itself instead of stopping.

### 8.1 Self-relay at capacity

When the agent detects it's nearing safe context capacity mid-task:

```
1. Do NOT stop and wait. Finalize the current Session Summary in Memory.md
   exactly as §1 already requires — state, last file touched, immediate
   next step.
2. Spin a successor instance directly, headlessly:
   anv --no-tui --yolo --model <model> --provider <provider>
     "Bootstrap per instruction.md §1, then continue from Memory.md's
      Immediate Next Step" 2>&1
3. The successor runs the standard Bootstrap Protocol (§1) cold and picks
   up exactly where the prior instance left off — T-182, or whatever's next.
4. Log the relay as a one-line note in Memory.md: "Relayed to successor at
   HH:MM — reason: capacity" so the chain of sessions stays traceable for
   whenever a human does look.
```

No human action sits between one session ending and the next beginning.

### 8.2 Consultation before any Open Question is logged

The same mechanism generalizes to *doubts*, not just capacity. Anywhere this protocol would otherwise have the agent stop and ask — an ambiguous instruction, a judgment call it isn't confident in — it consults before it defers to a human, rather than instead of:

```
1. Self-resolve: try an alternate approach with available context.
2. Consult: spin a disposable headless agent via `anv`, with the specific
   doubt plus the relevant spec/task excerpt as its entire prompt — it
   only needs enough context to answer one question, not the full project
   history.
3. If the answer resolves the doubt with reasonable confidence: proceed,
   and log both the question and the answer in Memory.md — consulted, not
   silently trusted, so it's auditable later.
4. If not resolved, or the consultation agent is itself uncertain: one
   more attempt with a different framing, or hand it to the Critic
   Sub-Agent if the doubt is spec/architecture-shaped.
5. Only if that also fails does it become an `Open Question` under G4 —
   logged, async, queued for the next interrogation phase or whenever the
   human next checks in. Still never a synchronous stop.
```

### 8.3 What this doesn't override

The three carve-outs in §4 — PRD-impact deviations, credential-bearing files, `instruction.md` edits — aren't doubts a peer instance can resolve; they're authority boundaries, not uncertainty. Consultation doesn't apply to them — they still route toward you, just async rather than mid-session, same as before. Routing everything else through a consultation agent first is what keeps that short list an intentional exception instead of the system quietly leaking pauses everywhere else.

---

## 9. Residual risk, stated plainly

The trade is "review before" → "review whenever, if ever." Reversibility tiering, structural validation, and the Critic Sub-Agent are the substitute for a person at each individual gate — and they're a reasonable substitute for reversible, in-spec work. What they're *not* a substitute for is a bad session-init: since G3 is the one seam left, and everything downstream inherits the Charter, the quality of that first interrogation phase now matters more than it did in v4, not less. If the brainstorming in §1 is shallow, v5 will execute a long way on a bad premise before anything forces a second look.

The consultation and self-relay mechanism in §8 adds a second version of that same risk: a disposable headless instance giving a confident-but-wrong answer, or a chain of relayed sessions each inheriting the last one's small drift, can travel several tasks before the Critic or a human catches it. The logging requirement in §8.2 doesn't fix that — it just keeps the thread traceable back to where it went wrong.

---

*Open items for you to rule on, not blocking this document: (1) accept Option A or B for G3 in §4; (2) confirm the three carve-outs in §4 or override any of them; (3) decide whether the Critic Sub-Agent should be a different model from the Orchestrator, or just a fresh-context instance of the same one; (4) same question for the Verification Sub-Agent in §7 — separate role, or folded into the Critic's remit; (5) which model/provider a self-relay successor should spin up with — same tier as the current session, or the lighter `anv --yolo` config already used for commits; (6) whether relay chain depth needs a soft cap or just stays visible via the Memory.md log.*
