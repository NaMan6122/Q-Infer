# Init Charter — M0 (Advisor)

**Status:** ACTIVE v0.1 · **Ratified:** 2026-10-26 · **Governs:** M0 phase
**Authorised by:** the one-time interrogation required by `instruction.md` §6.

This is the baseline every autonomous decision in M0 is checked against. If a decision is not covered here, it becomes an `Open Question` (§8) — it does not become a pause.

---

## 1. Scope

M0 delivers the **Advisor**: fingerprint, catalog, predictor, planner, CLI, and the read-only dashboard. No installs, no tuning, no serving. Safe to run on any machine the user owns.

Exit is governed by `docs/milestones/m0-advisor.md` — 12 acceptance tests.

---

## 2. Interrogation answers

| # | Question | Answer | Source |
|---|---|---|---|
| Q1 | `instruction.md` v4 is missing | **Reconstruct a consolidated v5** from the AOP diff plus referenced v4 sections; commit as DRAFT for one-time approval; mark inferred content `⚠R` | Human, explicit |
| Q2 | G3 (PRD impact) handling | **Option A** — human-gated but asynchronous; never a mid-session stop | Recommended default, no objection |
| Q3 | Three carve-outs | **Confirmed** — PRD impact, credential files, `instruction.md` edits | Recommended default, no objection |
| Q4 | Critic & Verification staffing | **Separate roles, fresh context, same model tier** | Recommended default, no objection |
| Q5 | M0 scope | **Core M0 as specced** — T-001…T-009; no signing infrastructure this phase | Recommended default, no objection |
| Q6 | Autonomy level | **Full autonomy to M0 exit**; async reporting via `Memory.md` + Phase Completion Report | Recommended default, no objection |

> Q2–Q6 were presented with recommendations and adopted by default. Any can be overridden at the next check-in without unwinding work, because each maps to a reversible process choice, not a code decision.

---

## 3. Locked product decisions

These are inherited from `PRD.md` §16 and are **not** re-litigated inside M0:

| ID | Decision |
|---|---|
| Q1·P | Single static Rust binary; Python only where an engine requires it |
| Q3·P | Never host or redistribute model weights |
| Q4·P | Telemetry off by default; opt-in, content-free, inspectable |
| Q5·P | Integrate with orchestrators, never compete |
| Q6·P | Apache-2.0 core; paid fleet/air-gap tier |
| Q8·P | We measure quant quality ourselves on a fixed eval suite |
| Q9·P | `balanced` is the default objective |
| Q10·P | Local/private artifacts supported from day one |

Engineering decisions are locked in `docs/adr/0001…0009`.

---

## 4. Standing constraints for this phase

1. `k_cal` is **never clamped** to 1.0 (ADR-0003, measured 1.175).
2. Bandwidth is published **both** single-threaded and saturated; a probe that cannot satisfy the invariants reports `unknown`.
3. Capability is recorded **per artifact**, never per version (ADR-0004).
4. The dashboard is **read-only** and opens no socket (ADR-0009).
5. No implicit privilege escalation, ever (ADR-0005).
6. Only Apple Silicon is Tier A until hardware we own has run a platform (ADR-0007).

---

## 5. Out of scope for M0

Installing or serving anything; autotuning; fleet bundles; Windows-ARM installs; catalog signing; multi-node. These are M1–M3 and are `PENDING` in `spec-index.md`.

---

## 6. Autonomy grant

Within §1–§5 above, the Orchestrator may:

- decompose and dispatch work to Workers,
- auto-promote through G1/G2 where the rules allow,
- choose between implementation options with ≥2 documented alternatives for `LOW` reversibility,
- mark tasks `DONE` on an independent Verification packet,
- proceed from M0 into M1 without a checkpoint (G5), reporting asynchronously.

The Orchestrator may **not**: alter PRD intent (G3), touch credentials, or edit `instruction.md`.

---

## 7. Amendment

Raise an `Open Question` with the proposed change and rationale. The human amends this file. Until amended, this Charter stands.

---

## 8. Open questions carried into M0

| # | Question | Impact if unanswered |
|---|---|---|
| OQ-01 | Does the `sm_121` FP4 rewrite risk invalidate GB10 as an M1 target? | M1 platform scope |
| OQ-02 | Native on-device bandwidth probe, or CPU probe + `k_cal`? (SDD S1) | Accuracy ceiling on non-Apple platforms |
| OQ-03 | Catalog signing — key custody and rotation | Blocks M3 fleet work, not M0 |
| OQ-04 | Second spike (MoE + CUDA) needs hardware we do not own (T-010) | Predictor generality claim |
