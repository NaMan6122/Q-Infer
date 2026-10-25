# ADR-0002 — The Capability Fingerprint is the only source of hardware truth, and measured beats declared

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §5.1, §20, spike REPORT §3
- **Review trigger:** If a Tier-A plan needs a hardware fact the CFP does not carry

## Context

Every downstream component — predictor, planner, provisioner, verifier — needs to know what the machine is. If each one shells out to `nvidia-smi` or `sysctl` independently, they will disagree, and the disagreements will be invisible until a plan fails.

Specification sheets are also actively misleading in the directions that matter: Apple Ultra parts advertise 819 GB/s and deliver ~317–357 GB/s in decode; a Snapdragon NPU advertises 45 TOPS and loses to its own CPU on an 8B model.

## Decision

One component collects a versioned Capability Fingerprint (`qinfer/cfp@1`) as a single JSON document. No other component may invoke a hardware probe. Every field carries `source`, `confidence`, and — where the field is a measurement — the method and spread.

Where a sensor is unavailable, we substitute a short controlled experiment rather than reading a spec sheet. Where neither is possible, the field is `null` with an explicit reason, never a plausible default.

## Consequences

**Positive**

- One place to fix a detection bug; one place to test traps (PRD §20).
- Plans become reproducible: same fingerprint, same inputs.
- Confidently-wrong spec numbers are structurally excluded.

**Negative / costs**

- Fingerprinting is now on the critical path; it must be fast (<20 s full, <2 s fast).
- Measurement requires care (buffer > LLC, thread sweep, refuse when paging), which is real engineering.
- A wrong measurement is worse than a missing one, so probe discipline is a correctness requirement, not a nicety.

## Alternatives considered

- **Per-component probing.** Rejected: guarantees drift.
- **Trust the OS/spec values.** Rejected: demonstrably wrong by 2.3× on Apple Ultra and by 2.4× on a Qualcomm datasheet (dossier 03).

## Notes

The M0 spike is the reference implementation of the measurement discipline the product needs.
