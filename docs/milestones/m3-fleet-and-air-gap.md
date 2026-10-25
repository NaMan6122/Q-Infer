# M3 — Fleet & Air-gap — bundles, drift, CI.

**Objective:** Make one good configuration reproducible across many machines, including disconnected ones.

**Duration:** 5–6 weeks · **Depends on:** M2 · **Unlocks:** paid tier

## 1. Scope

Export/import bundles for air-gapped hosts, fleet drift detection, `qinfer.yaml` in CI, and a GitHub Action. This is the milestone that makes the product sellable to regulated and industrial buyers, and it is where the receipts from M0–M2 start paying compound interest.

## 2. Non-goals

- A hosted control plane.
- Scheduling or placement across a fleet.
- Multi-tenant isolation.

## 3. Deliverables

| # | Deliverable | Notes |
|---|---|---|
| D3.1 | `qinfer bundle export/import` | Artifacts + images + manifest, vendor SDKs referenced not embedded |
| D3.2 | Bundle verification + partial-import resume | |
| D3.3 | `qinfer receipts diff` | Explains deltas in terms of fingerprint differences |
| D3.4 | Drift detection across machines | Same plan, different measurement |
| D3.5 | `qinfer plan --check` for CI (exit 8 on drift) | FR-48 |
| D3.6 | GitHub Action wrapping D3.5 | |
| D3.7 | Catalog pinning across a fleet | FR-49 |

## 4. Acceptance tests

Every item is a test that must exist and pass before the milestone closes. Format is Given/When/Then where the test is behavioural.

| ID | Given | When | Then |
|---|---|---|---|
| AT-3.1 | A connected host with a working plan | A bundle is exported | Manifest is complete and verified |
| AT-3.2 | An air-gapped host | The bundle is imported | Apply succeeds with **zero network access** (verified by tracing) |
| AT-3.3 | Two machines with the same plan | Receipts are diffed | Differences are attributed to fingerprint deltas, not just reported as numbers |
| AT-3.4 | A fleet member whose perf regressed | Drift check runs | Flagged, with the likely cause named |
| AT-3.5 | A repo with `qinfer.yaml` | CI runs `plan --check` | Exit 8 when a materially better plan exists; CI fails |
| AT-3.6 | A pinned catalog | A newer catalog is published | The pinned plan is unaffected |
| AT-3.7 | A bundle with a missing artifact | Import runs | Fails before applying anything, names the missing piece |

**Air-gap test is binary:** the CI job runs in a network namespace with no route. If it passes there, it is real.

## 5. Exit criteria

- AT-3.1 … AT-3.7 green.
- One reference air-gapped deployment completed by a design partner.
- Drift detection catches an injected regression with no false negative.
- Paid tier feature-complete enough to price.

## 6. Risks specific to this milestone

| Risk | Mitigation |
|---|---|
| Vendor SDKs cannot legally be bundled | Reference-only manifests; document the manual step clearly |
| Container export is huge or flaky | Layer-level export, resumable, checksummed |
| Fleet features tempt us into orchestration | ADR-0007: integrate (emit plans), never schedule |

## 7. Demo script

Export on a laptop, walk to a disconnected box, import, serve. Then change a config on one machine and show the drift report. This is the demo that closes enterprise deals.
