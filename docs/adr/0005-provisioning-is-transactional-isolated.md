# ADR-0005 — Provisioning is transactional, isolated, content-pinned, and never escalates privilege implicitly

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §5.4, §11.3
- **Review trigger:** If a Tier-B platform cannot be installed without implicit elevation

## Context

Installation is where a tool earns or loses trust. The prevailing norm — `curl | sh` with sudo — is disqualifying in exactly the environments that own the most bare metal: regulated, air-gapped, and fleet-operated.

Partial failure is the common case: a download stalls, a wheel is missing, a container pull dies at 90 %.

## Decision

Every plan resolves to content-addressed artifacts (container digests, wheel and model hashes). Installation happens into an isolated prefix or container, journaled step by step, and is idempotent and resumable. Any failure leaves the previously working plan intact.

Steps requiring elevation are enumerated in the plan, shown to the user, and can be emitted for manual execution (`--print-sudo`). The tool never pipes a remote script into a shell and never invokes sudo on its own.

## Consequences

**Positive**

- Interruption-safe, which is the difference between a tool people trust and one they abandon.
- Auditable: the receipt shows exactly what changed.
- Usable in environments with no interactive root.

**Negative / costs**

- Isolation costs disk (per-plan prefixes) and requires careful cleanup.
- More code than a direct install; every step needs a compensation action.
- Some vendor installers resist isolation and must be handled as documented exceptions.

## Alternatives considered

- **Install into the system prefix.** Rejected: destroys rollback and reproducibility.
- **Containers only.** Rejected: not available on macOS or Windows-on-ARM, and not always desirable at the edge.

## Notes

Fault injection at every step is a P0 test, not a nice-to-have.
