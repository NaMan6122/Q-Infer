# ADR-0003 — Prediction uses a two-parameter roofline with an unbounded calibration coefficient

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §5.2, spike REPORT §5–6
- **Review trigger:** If MAPE exceeds 25% on dense models across two additional platforms

## Context

The naive model `tok/s = bandwidth / bytes_per_token` is accurate for large dense models and wrong for the cases users care most about. On the M0 spike it over-predicted a 0.5B model by 39 % while landing within 0.1 % on a 3B.

Two corrections are load-bearing: a fixed per-token cost `t₀`, and the fact that the bandwidth a GPU achieves is not the bandwidth a CPU probe demonstrates.

## Decision

Predict with `t_token = t₀ + bytes_per_token / (k_cal × BW_probe)`, where:

- `t₀` is keyed on `(model_shape_class, backend)`, never global;
- `k_cal` is a **calibration coefficient**, stored per `(backend, probe_method, arch_family, quant_class)`, and is **not** clamped to 1.0;
- predictions are intervals, and the receipt records the realised error.

Rank ordering — not point accuracy — is the product requirement.

## Consequences

**Positive**

- MAPE 1.45 % vs 15.6 % on the spike, with every point inside ±3 %.
- The model degrades honestly: when it cannot know (MoE, spill, novel attention), it widens or refuses.
- One measurement updates all future plans on that machine.

**Negative / costs**

- Two parameters from few observations is underdetermined; we must refuse to fit from <3 points.
- `k_cal` > 1 invites "that can't be right" objections; the documentation cost is real.
- Still unvalidated for MoE, CUDA, and batching (spike REPORT §7).

## Alternatives considered

- **Naive roofline.** Rejected: structured, predictable failure on small models.
- **Per-model empirical lookup table.** Rejected: does not generalise to models we have not measured, which is the entire premise.
- **Clamp k_cal to 1.0.** Rejected: under-predicts by 17 % on measured hardware.

## Notes

If MoE or CUDA spikes break this, the planner's ranking logic is what must be reworked — not the arithmetic.
