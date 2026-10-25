# ADR-0007 — Platform certification is tiered and gated on hardware we have actually run

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §8.1, dossier 02 §1, dossier 03 §1
- **Review trigger:** If a Tier-A platform cannot be kept in CI for two consecutive releases

## Context

The temptation is to claim support for everything, because the matrix looks impressive. It is also the fastest way to destroy credibility: a single failed install on a claimed platform poisons every other claim.

At v0.2 the team physically possesses exactly one machine — an Apple M5 Pro. Every other platform claim is currently desk research.

## Decision

Four tiers. **A (Certified)** requires hardware we own or rent with a human having run it; full install→bench gates every release. **B (Supported)** has an exercised install path and heuristic tuning. **C (Detected)** offers fingerprint and advice only. **D (Known-unsupported)** explains the refusal.

At v0.2 only Apple Silicon is genuinely Tier A. All other "A" targets are conditional on hardware acquisition before M1.

## Consequences

**Positive**

- Claims are defensible and testable.
- A clear promotion path: buy hardware → run it → certify.
- Refusals become a feature, not an embarrassment.

**Negative / costs**

- The honest matrix is smaller than a marketing matrix.
- Every Tier-A platform is a permanent CI and support cost — roughly one FTE per family.
- Hardware acquisition is a real budget line, not a detail.

## Alternatives considered

- **Claim best-effort support everywhere.** Rejected: unbounded support burden with no verification.
- **Certify from emulation.** Rejected: emulation does not reproduce drivers, thermals, or unified-memory behaviour.

## Notes

The GB10 dossier refused to certify Spark for exactly this reason. That is the standard.
