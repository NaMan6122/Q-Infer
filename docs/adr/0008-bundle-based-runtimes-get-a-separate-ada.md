# ADR-0008 — Bundle-based runtimes get a separate adapter class from text-generation engines

- **Status:** Accepted (v0.2)
- **Date:** 2026-10-26
- **Deciders:** engineering, product
- **Evidence:** PRD §9, dossier 03 §3
- **Review trigger:** If a bundle runtime exposes a first-class dynamic-context API

## Context

GGUF- and safetensors-based engines accept runtime parameters: context length, quantisation, KV dtype, batch size. Bundle-based runtimes — Qualcomm QAIRT context binaries, TensorRT-LLM engines — bake precision, context length and KV size at *compile* time.

Modelling them as ordinary engines produces plans that cannot be executed, and a catalog keyed on `(model, quant)` silently mismatches the chipset-specific artifact.

## Decision

Two adapter classes:

- **Dynamic engines** (llama.cpp, MLX, vLLM, SGLang, Ollama): parameters are supplied at launch.
- **Bundle engines** (QAIRT/QNN, TensorRT-LLM): the artifact *is* the configuration. The plan selects a pre-compiled bundle; context and precision are outputs of selection, not inputs.

For bundle engines the catalog key includes the chipset (and, for TensorRT, the SM target), and the planner may only offer bundles that exist for this exact target.

## Consequences

**Positive**

- Plans are executable by construction on constrained platforms.
- Chipset/SM targeting becomes explicit rather than an implicit assumption.
- Avoids over-promising tuning that is physically impossible.

**Negative / costs**

- Two code paths and two catalog shapes.
- Fewer plans are offerable on bundle platforms, which is honest but looks like weakness.
- Bundle availability becomes a hard dependency on a vendor catalog.

## Alternatives considered

- **One adapter with capability flags.** Rejected: the parameter model differs at the level of the API contract, not a flag.
- **Exclude bundle runtimes from v1.** Defensible; rejected because it abandons the entire NPU class.

## Notes

This is the adapter decision most likely to be revisited when Windows ML matures.
