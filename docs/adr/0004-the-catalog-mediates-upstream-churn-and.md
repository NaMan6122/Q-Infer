# ADR-0004 — The catalog mediates upstream churn and records per-artifact arch coverage

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §9, dossier 02 §3
- **Review trigger:** If catalog updates cannot keep pace with a Tier-A engine's releases

## Context

Upstream moves weekly. vLLM, llama.cpp and MLX all change flags, defaults and build matrices faster than any product team can re-release. A product that hardcodes engine knowledge is obsolete within a month.

Worse, capability is per-artifact, not per-version: a published wheel can load successfully, register its Python ops, and still have no cubins for the target GPU — failing at kernel launch rather than import (the `_qutlass_C` case).

## Decision

All engine and model knowledge lives in a signed, versioned catalog that is data, not code. Each artifact entry records its format, quantisation, hashes, licence, gating, architectural metadata, and **per-architecture coverage** (cubin targets, bundle chipset, declared support).

A new engine release that breaks something is handled by a catalog update that marks the version unusable — not by a core release, and never by breaking a user's working install.

## Consequences

**Positive**

- Upstream churn is absorbed without shipping.
- Users can pin a catalog version for reproducibility.
- Arch coverage becomes queryable, which is the only defence against silent kernel gaps.

**Negative / costs**

- The catalog becomes a product surface with its own correctness burden and signing infrastructure.
- Static arch lists are necessary but insufficient (`cuobjdump` cannot distinguish `12.0` from `12.0f`), so execution probes must still gate.
- Stale catalogs are a failure mode; plans must warn and refuse to guess.

## Alternatives considered

- **Hardcode knowledge in the binary.** Rejected: release cadence would be the bottleneck.
- **Scrape upstream at runtime.** Rejected: unreproducible and offline-hostile.

## Notes

See ADR-0008 for why bundle-based runtimes need their own catalog shape.
