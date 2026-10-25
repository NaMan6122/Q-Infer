# ADR-0006 — Receipts are the primary product artifact; state is plain files, not a database

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §5.5, §11.2
- **Review trigger:** If receipts exceed ~10 MB or a fleet needs server-side query

## Context

A user's real question after setup is not "did it install" but "am I getting what I paid for, and can I prove it next month". That requires a durable record of what was installed, what it measured, and how far reality diverged from the prediction.

Opaque state also creates lock-in: a user cannot inspect, diff, or repair what they cannot read.

## Decision

Every verification produces a signed JSON receipt containing the plan, fingerprint hash, installed artifacts, measured metrics, prediction error, and verdict. All state lives as versioned JSON/YAML under a single directory — no database, no binary blob.

Receipts are diffable, archivable, attachable to bug reports, and are the training signal for the calibration store.

## Consequences

**Positive**

- Reproducibility and drift detection fall out for free.
- Users can `cat`, `diff`, `git commit` and hand-repair their state.
- The calibration flywheel has a natural data source.

**Negative / costs**

- JSON on disk does not scale to fleet-wide querying; that becomes the paid tier's problem.
- Schemas must be versioned and migrated (or explicitly not migrated).

## Alternatives considered

- **SQLite.** Rejected for v1: opaque, harder to diff, no real benefit at single-box scale.
- **No receipt, just logs.** Rejected: logs are not comparable or provable.

## Notes

Stdout is for humans; the receipt is for machines and for the future.
