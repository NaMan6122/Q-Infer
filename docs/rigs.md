# Test Rigs

What we can actually test on, and — more importantly — what each rig **cannot** validate. ADR-0007 makes certification conditional on hardware we own, so this file is the evidence behind every tier claim in `PRD.md` §8.

Last updated: 2026-10-26.

## Inventory

| Rig | Hardware | OS | Status |
|---|---|---|---|
| `m5-pro` | Apple M5 Pro, 15-core CPU (5 Super + 10 Performance), 16-core GPU, 24 GiB unified, ~307 GB/s spec | macOS 27.0.1 | **Available** |
| `sd-8elite` | Snapdragon 8 Elite (SM8750), Hexagon **v79**, ~85 GB/s | Android | **Available — attachable** |
| `sd-8elite-g5` | Snapdragon 8 Elite Gen 5 (SM8850), Hexagon **v81**, ~85 GB/s | Android | **Available — attachable** |
| `cuda/none` | — | — | **Not available** |

## What each rig can and cannot prove

### `m5-pro` — Tier A for Apple Silicon

**Can:** fingerprint accuracy, bandwidth probe (thread sweep, pressure guard), the predictor's dense-model accuracy, KV geometry, `cfp@1` validation end-to-end, the dashboard.

**Cannot:** validate anything below 24 GiB (no 8/16 GiB refusal path), any discrete GPU, any NPU, sustained thermal behaviour (this chassis is not representative of a Mac Studio), or >24 GiB context plans.

**Known caveat:** this machine runs with ~10.7 GiB of swap in use, so bandwidth measurements are frequently flagged `unknown`. That is the guard working, but it means "measured" runs need a quiet machine.

### `sd-8elite` / `sd-8elite-g5` — Tier B/C for Android

**Can:** the **bundle-based adapter class** (ADR-0008) — the only untested adapter shape; the detector's `ro.soc.model` → chipset mapping; the NPU-vs-CPU honesty UX (the dossier claims CPU wins above ~4B — this is where we demonstrate it); and **two different Hexagon architectures (v79, v81)** on one desk, which is a real test of arch-keyed catalog entries.

**Cannot:** validate Snapdragon **X Elite** performance, certify any **v73** artifact, or say anything about Windows-on-ARM (different OS, different signing model, different packaging).

## The "amplitude" question, settled

The intuition — *mobile Snapdragon is the same architecture as laptop Snapdragon, just smaller* — is **right about the stack and wrong about the artifact**:

| | Holds | Breaks |
|---|---|---|
| CPU / GPU / NPU **IP families** | ✅ Oryon, Adreno, Hexagon | |
| **Software stack** | ✅ QAIRT/QNN, Genie, `ggml-hexagon`, AI Hub flow | |
| **Quantisation rule** (`Q4_0` for a clean HTP run) | ✅ | |
| **NPU binary compatibility** | | ❌ context binaries are compiled **per Hexagon arch**; v73 ≠ v79 ≠ v81 |
| **Performance transfer** | | ❌ 135 GB/s vs ~85 GB/s, and 5× lower TDP throttles in seconds |

Perf-per-watt scales like your amplitude analogy. Binary compatibility does not scale at all — it is discrete.

## Consequence for the catalog

A bundle-keyed artifact is **not** `(model, quant, "snapdragon")`. It is:

```
(model_hash, quant, hexagon_arch, qairt_version, chipset_id)
```

`qairt_version` matters directionally: runtime SDK must be ≥ the bundle's compile SDK, or the NPU path fails with error 5000. Two phones with different Hexagon archs are two catalog targets, not one.

## What is unblocked without CUDA

| Gap | Blocked on CUDA? | Status |
|---|---|---|
| MoE `t₀` behaviour | **No** — a 1.3B-active MoE fits in 24 GiB | In progress (`OLMoE-1B-7B`, ~3.9 GiB) |
| Batched decode | No | Pending |
| Long-context KV | No | Pending |
| CUDA / `sm_121` accuracy | **Yes** | Deferred — GB10 stays Tier B, never certified |
| Snapdragon NPU path | No — phones available | New track, after M0 core |
