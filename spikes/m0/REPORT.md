# M0 Spike — Does the predictor actually predict?

**Date:** 2026-10-26
**Question:** PRD §5.2 claims `t_token ≈ t0 + bytes_per_token / BW_eff` predicts decode throughput well enough to rank plans. Is that true on real hardware?
**Answer:** Yes — but only the two-parameter form, and one detail in the PRD is wrong.

---

## 1. TL;DR

| # | Finding | Impact on the PRD |
|---|---|---|
| **F1** | A single-threaded memory probe reports **131 GB/s**; the same machine saturates at **228 GB/s** with 15 threads. | Fingerprint must sweep threads. Publishing the 1-thread number would halve every prediction. |
| **F2** | Naive `BW/bytes` is accurate for models ≥ 3 B (−0.1 %, −7.7 %) and **39 % wrong** on a 0.5 B model. The two-parameter model reduces MAPE from **15.6 % → 1.45 %**. | §5.2 is correct that the second term is needed — but only for small models and MoE, not universally. |
| **F3** | Fitted ε = **1.175**, i.e. **greater than 1**. The PRD says efficiency is "bounded above by 1.0". That is **false**. | §5.2 must redefine ε as a *probe-relative calibration coefficient*, not an efficiency. |
| **F4** | Fitted `t0` = **1.22 ms** on a dense Metal path, versus ~8.4 ms reported for MoE on Ultra-class parts. | `t0` is model-class dependent. A single global constant is wrong. |
| **F5** | Q4_K_M and Q8_0 of the same 3 B model have **near-identical prefill** (2 798 vs 2 820 tok/s) but decode differs **1.59×**. | Confirms the compute-bound-prefill / bandwidth-bound-decode split. Quant choice should be driven by decode. |

---

## 2. Setup

| | |
|---|---|
| Machine | Apple M5 Pro, 15-core CPU (5 × "Super" @ 16 MB L2, 10 × "Performance" @ 8 MB L2), 16-core GPU, Metal 4 |
| Memory | 24 GiB unified, **11.5 GiB swap in use**, 36 % free at test time |
| OS | macOS 27.0.1 (26A434), Darwin arm64 |
| Engine | llama.cpp 0.7.0, build 11669 @ `620d2225c`, backends `BLAS,MTL` |
| Offload | `-ngl 99` (full Metal) |
| Harness | `llama-bench -p 512 -n 128 -r 5 -o json` |
| Models | Qwen2.5-0.5B-Instruct Q4_K_M · Qwen2.5-3B-Instruct Q4_K_M · Qwen2.5-3B-Instruct Q8_0 |

Reproduce: `cd spikes/m0 && python3 fingerprint.py && python3 spike.py`

---

## 3. Fingerprint

```
brand        Apple M5 Pro          cores 15P / 15L
gpu          Apple M5 Pro, 16 cores, spdisplays_metal4
memory       24.0 GiB (swap used 11.5 GiB)
isa          DotProd, I8MM, BF16, SME, SME2, SME2p1     (FEAT_FP8 absent)
```

Bandwidth sweep — the headline number depends entirely on how you measure it:

| Threads | 1 | 2 | 4 | 8 | 15 |
|---|---|---|---|---|---|
| copy GB/s | 131.4 | 192.9 | 216.4 | 225.9 | **228.0** |
| read-only (sum) GB/s | 74.2 | 135.4 | 157.5 | 219.0 | 220.6 |

Against the 307 GB/s spec figure, the saturated measurement is **74 %**. Consistent with the 0.7–0.85 range in the PRD, and nowhere near the spec sheet.

> **Detection note.** An earlier run of this same probe returned 125 GB/s with **77 % spread** while the machine held 10 GiB of swap. The instability was the tell. The fingerprint must refuse to publish a bandwidth figure when spread is high or the box is paging — a low-confidence number silently poisons every downstream plan.

---

## 4. Measurements

| Model | Size | Prefill (pp512) | Decode (tg128) |
|---|---|---|---|
| Qwen2.5-0.5B Q4_K_M | 0.45 GiB | 16 323.6 tok/s | **337.6 tok/s** |
| Qwen2.5-3B Q4_K_M | 1.95 GiB | 2 798.3 tok/s | **108.8 tok/s** |
| Qwen2.5-3B Q8_0 | 3.36 GiB | 2 819.8 tok/s | **68.4 tok/s** |

---

## 5. Prediction

Fitted over all three points: **t0 = 1.219 ms, ε = 1.175, R² = 0.9996**.

| Model | Measured | Naive `BW/W` | Error | Two-param | Error | Implied ε |
|---|---|---|---|---|---|---|
| 0.5B Q4_K_M | 337.6 | 469.6 | **+39.1 %** | 330.0 | −2.3 % | 0.72 |
| 3B Q4_K_M | 108.8 | 108.6 | −0.1 % | 110.5 | +1.6 % | 1.00 |
| 3B Q8_0 | 68.4 | 63.2 | −7.7 % | 68.1 | −0.5 % | 1.08 |
| **MAPE** | | | **15.6 %** | | **1.45 %** | |

The two-parameter model holds every point inside ±3 %. The naive model is not merely less accurate — it fails in a *structured* way: it over-predicts small models, because a fixed per-token cost dominates when there are few bytes to stream.

---

## 6. What this changes

### §5.2 — ε is not an efficiency

The PRD states ε is "bounded above by 1.0". The fit returns 1.175. This is not noise: the 3B Q8 model moves 3.61 GB per token at 68.4 tok/s, implying the GPU is pulling **247 GB/s** from DRAM, while a 15-thread CPU memcpy only reaches 228 GB/s.

The GPU is genuinely outrunning the CPU probe. So ε is not "how efficient the backend is" — it is **the ratio between the probe's measured bandwidth and the bandwidth the executing device actually achieves**. It can legitimately exceed 1, and it should be stored per `(backend, probe_method)`, not treated as a bounded efficiency.

Two consequences:

1. Rename it. `η`/efficiency implies a ceiling that does not exist. Call it `k_cal` — a calibration coefficient.
2. Either probe on the device that will do the work (a Metal buffer test), or accept ε > 1 and document it. The cheap CPU probe plus a learned coefficient is the pragmatic choice for v1.

### §5.1 — bandwidth needs two numbers, not one

The fingerprint must record `single_thread_gbs` **and** `saturated_gbs`, plus the thread count at which saturation occurred. They differ by 1.74× on this machine; either one alone is a wrong answer to a different question.

### §5.2 — `t0` is per model class

1.22 ms here (dense, Metal) versus ~8.4 ms reported for MoE on M3 Ultra. The planner must key `t0` on `(model_shape_class, backend)`, not carry a single global constant. MoE and small models are exactly where this term dominates.

### §9/§18 — quantisation guidance confirmed

Identical prefill for Q4_K_M and Q8_0 (2 798 vs 2 820, inside noise) with a 1.59× decode gap. Prefill is compute-bound and indifferent to weight precision; decode is bandwidth-bound and scales with bytes/token. Any tuning heuristic that trades quality for "faster inference" must say *which* phase it is buying.

---

## 7. Threats to validity

- **n = 3, one machine, one backend, one build.** R² with two parameters and three points has a single degree of freedom. The result is encouraging, not conclusive.
- **Machine was under memory pressure** (11.5 GiB swap). Absolute bandwidth — and therefore absolute ε — is likely pessimistic here. The *relative* result (two-param ≫ naive) is unaffected.
- **Single-stream decode only.** No batching, no long context, no KV-cache pressure. The PRD's §5.2.2 batching model is untested.
- **Dense models only.** MoE is where `t0` matters most and is not covered.
- **Cross-run variance is real:** our own bandwidth probe ranged 216–252 GB/s across runs on the same idle-ish machine (±8 %). Any prediction claim finer than ±10 % is not measurable on one box.

---

## 8. Next spikes, in priority order

1. **MoE on the same harness** — a 30B-A3B Q4 model would test the `t0`-dominance claim directly. (Needs a machine with ≥ 24 GB free.)
2. **CUDA reproduction** — the whole result could be a Metal artifact. Run the same three models on `sm_89`/`sm_120`.
3. **Batched decode** — validate the §5.2.2 two-regime model at B ∈ {1, 4, 16, 64}.
4. **Long context** — confirm the KV term at 8k/32k, and the q8_0 KV correction (53.1 %, not 50 %).

If (1) and (2) hold, §5.2 is load-bearing. If either fails, the planner's ranking logic needs rework before any of it ships.

---

## 9. Artifacts

| File | Contents |
|---|---|
| `fingerprint.py` | Capability fingerprint collector (no root, ~90 s) |
| `spike.py` | llama-bench harness + two-parameter regression |
| `fingerprint.json` | Raw fingerprint from this run |
| `results.json` | Raw measurements and predictions |
| `models/` | GGUFs (git-ignored; see `run.sh`) |

---

## 10. MoE addendum — 2026-10-26

The main experiment used only dense models. MoE was the largest untested assumption in §5, and the desk research predicted a `t₀` of **~8.4 ms** for MoE (backed out of a quant ladder on an M3 Ultra), which is what would break rank ordering.

### Method

`OLMoE-1B-7B-0924-Instruct` Q4_K_M — a real sparse MoE: **6.92 B total / 1.3 B active**, 64 experts top-8, 4.87 bit/param, 3.92 GiB. Same harness as §2 (`llama-bench -p 512 -n 128 -r 5 -ngl 99`), same machine, same probe bandwidth.

### Result

| Quantity | Value |
|---|---|
| Measured decode | **254.3 tok/s** ± 2.6 |
| Measured prefill (pp512) | 4 794.8 tok/s |
| Active bytes per token | 791 MB |
| Bandwidth term at `k_cal` = 1.175 | 2.95 ms |
| **Predicted (calibrated on dense models only)** | **239.6 tok/s** |
| **Error** | **−5.8 %** |
| Implied `t₀` for this MoE | **0.98 ms** |

### What this means

1. **The dense-fitted calibration transferred to a MoE within 6 %.** `k_cal` = 1.175 and `t₀` = 1.22 ms were fitted exclusively on three dense Qwen2.5 models; nothing was re-tuned. The predictor did not need to know it was looking at an MoE.

2. **`t₀` did not explode.** The implied value is **0.98 ms**, essentially the dense figure — not the ~8.4 ms the desk research predicted. That prediction is **rejected on this hardware and this model**.

3. **`ShapeClass` may be unnecessary.** `predict.rs` keys `t₀` on `Dense` vs `Moe`. If the parameter is genuinely transferable, that axis is dead weight; if it is real on larger MoEs, the current split is too coarse anyway (64 experts top-8 here, 128 top-8 on Qwen3-30B-A3B). Either way the conclusion is the same: **`t₀` must be fitted per observation set, never assumed from a class label.**

### Caveats — this is not yet a general MoE result

- **One MoE, one machine.** OLMoE is small and sparse (1.3 B active). Qwen3-30B-A3B reads ~2.3× more per token and routes across 128 experts.
- **The 8.4 ms figure came from a different method** — backing `t₀` out of a Q4/Q8/BF16 quant ladder, which conflates routing overhead with expert-working-set effects that a single-model measurement does not expose.
- **Active-parameter accounting is an estimate.** Published 1.3 B active; bytes-per-param taken from the file's own 4.87 bit average.

### Consequence

Rank ordering across dense and MoE models survives its first real test. The remaining risk is now narrower and better stated: **large MoEs on high-bandwidth machines**, not MoE as a class.

---

## 11. MLX vs llama.cpp on the same model — 2026-10-26

The assumption worth testing was "MLX is much faster on Apple Silicon". Two adapters exist (`engines.rs`), both runtimes are installed on the reference host, so it became measurable.

### Method

Same model family, same weights *semantics*, different containers:

| | llama.cpp | MLX |
|---|---|---|
| artifact | `Qwen2.5-3B-Instruct-Q4_K_M.gguf` | `mlx-community/Qwen2.5-3B-Instruct-4bit` |
| bytes/token | 2 104 932 768 (1.96 GiB) | **1 736 293 090** (1.62 GiB) |
| bits/param | 4.85 | 4.25 (4-bit + fp16 scale per group of 64) |

`llama-bench -p 512 -n 128 -r 5 -ngl 99` vs `mlx_lm.generate --max-tokens 128 --temp 0` (629-token prompt).

### Result

| | llama.cpp | MLX | delta |
|---|---|---|---|
| decode | 108.75 tok/s | **129.84 tok/s** | **+19.4 %** |
| prefill | 2 798.3 tok/s | 579.9 tok/s | **−79 %** |
| peak memory | — | 2.499 GB | |
| bytes read per token | 2.105 GB | 1.736 GB | −17.5 % |
| **implied `k_cal`** | **1.157** | **1.175** | +1.6 % |

### What this actually says

**The decode win is a packing win, not a kernel win.** MLX reads 21.2 % fewer bytes and decodes 19.4 % faster; the residual attributable to the backend is **−1.8 %**. Per byte streamed from DRAM, the two engines are equivalent on this machine — and `k_cal` agrees to 1.6 %.

That is a direct validation of the two-parameter model: `k_cal` behaves like a property of the **memory path**, not of the framework's marketing. It is also the answer to "is MLX much more powerful?" — *not per byte*. The advantage is that its default quantisation is denser.

**Prefill is the opposite story and matters.** llama.cpp's batched prompt processing is ~4.8× faster here. For chat, irrelevant. For RAG, summarisation, or long documents, decisive.

### Caveats — this is not a controlled experiment

- **Different quantisers.** Q4_K_M (4.85 bit, mixed k-quants) vs MLX 4-bit affine group-64 (4.25 bit). Fewer bits may also mean lower quality; the catalogue carries no measured quality delta for either, so nothing here proves the MLX artifact is quality-equivalent.
- **Prompt lengths differ** (512 vs 629 tokens).
- **`mlx_lm.generate`'s prefill number** reflects its own single-pass path, not a chunked-prefill implementation tuned for throughput.
- **One model, one machine, one run each.**

### Consequence for the product

The planner should not carry a "prefer MLX" rule, and it does not. It carries bytes-per-token, which is what actually determines decode — and a measured `k_cal` per backend. The choice between engines then reduces to **which quantisation packs tighter at acceptable quality**, and **which phase the workload is bound by**. That is a question the catalogue can answer and a preference cannot.
