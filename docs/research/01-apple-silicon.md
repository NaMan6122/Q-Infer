# Dossier 01 — Apple Silicon

**Status:** research v1 · 2026-10-26 · author: Anv (agent) · confidence: **high** on measured silicon and llama.cpp/MLX behaviour, **medium** on Ultra-tier scaling (cross-build confound), **low** on M5 Ultra (spec unverified). Host: MacBook Pro `Mac17,9`, Apple M5 Pro (15-core CPU / 16-core GPU), 24 GB, macOS 27.0.1 (26A434). Values marked *(measured here)* were re-verified on this host.

---

## 1. Verdict

We support **GPU-only** inference on unified memory ≥ 24 GB via two adapters: **llama.cpp Metal** (GGUF) and **MLX / mlx-lm**. Tier A gates CI; Tier B best-effort; below 24 GB we refuse to emit a plan.

- **Refuse** 8–16 GB machines: model + KV cannot fit inside Metal's working-set ceiling (§5).
- **Refuse** the ANE — unreachable outside CoreML, which cannot express modern decode.
- **Refuse** distributed meshes (JACCL/exo) as *supported* in v1: open intermittent upstream bug.
- **Refuse** Intel Macs, x86-under-Rosetta hosts, and plans assuming bandwidth scales with spec.
- **Accept** PyTorch MPS as documented-unsupported only; 10–30× off the pace.
- **Contract:** plans carry `t_token = t0 + weight_bytes / BW_eff`, never `BW_spec / weight_bytes`. `BW_eff` is 0.40–0.83 of spec by tier; `t0` is not negligible.

---

## 2. Hardware facts

Spec bandwidth is Apple's published figure. **Effective decode bandwidth is derived, not published**: `tg × 3.79 GB`, the LLaMA-2-7B Q4_0 footprint moved per token (6.74 B params × 4.5 bit ÷ 8). It folds in kernel inefficiency, `t0` and occupancy — and it is what capacity planning should use.

| Tier | GPU cores | Unified mem (GB) | BW spec GB/s | BW effective in decode GB/s (eff.) | ANE TOPS (rated) | Notes |
|---|---|---|---|---|---|---|
| M1 | 7–8 | 8 / 16 | 68.25 | 54 (0.79) | 11 | FP16-era ANE; 8 GB refused |
| M1 Pro | 14–16 | 16 / 32 | 200 | 138 (0.69) | 11 | |
| M1 Max | 24–32 | 32 / 64 | 400 | 232 (0.58) | 11 | |
| M1 Ultra | 48–64 | 64 / 128 | 800 | **317 (0.40)** | 22 | UltraFusion, 2 dies |
| M2 | 8–10 | 8 / 16 / 24 | 100 | 83 (0.83) | 15.8 | |
| M2 Pro | 16–19 | 16 / 32 | 200 | 147 (0.74) | 15.8 | |
| M2 Max | 30–38 | 32 / 64 / 96 | 400 | 250 (0.62) | 15.8 | |
| M2 Ultra | 60–76 | 64 / 128 / 192 | 800 | **357 (0.45)** | 31.6 | plateau begins |
| M3 | 8–10 | 8 / 16 / 24 | 100 | 81 (0.81) | 18 | |
| M3 Pro | 14–18 | 18 / 36 | 150 | 117 (0.78) | 18 | narrow-bus oddity |
| M3 Max | 30–40 | 36–128 | 300 / 400 | 251 (0.63) | 18 | 30-core bin = 300 GB/s |
| M3 Ultra | 60–80 | 96 / 192 / 512 | 800 | **349 (0.44)** | 36 | 512 GB = one-shot 70B |
| M4 | 8–10 | 16 / 24 / 32 | 120 | 91 (0.76) | 38 | 38 TOPS is INT8 |
| M4 Pro | 16–20 | 24 / 48 / 64 | 273 | 192 (0.70) | 38 | practical Tier-A floor |
| M4 Max | 32–40 | 36–128 | 410 / 546 | 265–315 (0.58–0.65) | 38 | 546 only on 40-core |
| M5 | 8–10 | 16 / 24 / 32 | 153 | 121 (0.78) | n/d | per-core Neural Accelerators |
| **M5 Pro** | 16 / 20 | 24 / 48 / 64 | 307 | **256 (0.83)** | n/d | **this host** (16-core); 2-die |
| M5 Max | 32 / 40 | 36–128 | 460 / 614 | 455 (0.74) | n/d | 614 only on 40-core |
| M5 Ultra | 80 | UNVERIFIED | 1228 | **679 (0.55)** | n/d | UNVERIFIED port |

Notes. (a) ANE TOPS stopped being published at M4 and switched FP16 → INT8, so 38 is not comparable to 18; measured FP16 ANE throughput is **~10–12.6 TFLOPS** (M3 Pro → M5), making 38 TOPS unreachable for LLM shapes. (b) M5 Pro/Max are two-die "Fusion" packages; M5 Max is two M5 Pro GPUs. (c) M5 benchmark rows used a **different llama.cpp build** with flash-attention — cross-generation ratios are upper bounds (§8). (d) M6 is out of scope.

---

## 3. Software stack map

| Framework | Backend | Install | Precision | Maturity | Verdict |
|---|---|---|---|---|---|
| **mlx-lm** | MLX (Metal; TensorOps on M5) | `pip install mlx-lm` (Py≥3.11, arm64) | 2/3/4/5/6/8-bit, MXFP4, NVFP4, bf16 | High, fast-moving | **Primary.** Best sustained decode on Qwen-class |
| **llama.cpp** | Metal (GGML) | `brew install llama.cpp` / CMake | GGUF Q2–Q8, K/IQ, F16/BF16 | Very high | **Primary.** Broadest coverage, honest numbers |
| PyTorch MPS | Metal via ATen | `pip install torch` (arm64) | fp32/fp16/bf16; INT4/8 experimental | Medium | Escape hatch; ~7–9 tok/s where MLX does ~230 |
| CoreML / ANE | Neural Engine | `pip install coremltools` | fp16/int8, static graphs | Medium | **Rejected:** no dynamic shapes, no KV cache |
| Ollama | llama.cpp → MLX (0.19+) | `brew install ollama` | GGUF; MLX on 0.19 | High but opaque | Ergonomics; backend changed under users (§7) |
| LM Studio | MLX + GGUF | GUI app | MLX, GGUF | High | Not CLI-automatable |
| exo | llama.cpp/MLX over TB RDMA | from source | GGUF, MLX | Low | Not v1; rides the broken JACCL path |
| MLX distributed | JACCL (TB5 RDMA) / Ring (TCP) | in `mlx` | inherits MLX | Low / Medium | Ring works; JACCL documented, refused |

**Structural fact:** only CoreML reaches the ANE; llama.cpp, MLX, Ollama and PyTorch run on the GPU. M5's Neural Accelerators are exposed via Metal 4 `TensorOps`, and MLX needs **macOS ≥ 26.2**. That pays in prefill (3.3–4.1× TTFT vs M4) but not decode (1.19–1.27×), which is bandwidth-bound.

---

## 4. Measured performance corpus

`pp` = prefill, `tg` = decode. Third-party published values only.

| Machine | Model | Quant | Backend | pp tok/s | tg tok/s | Source | Date |
|---|---|---|---|---|---|---|---|
| M1 (8c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 118.0 | 14.2 | llama.cpp #4167 | 2023-11-22 |
| M1 Max (32c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 530.1 | 61.2 | #4167 | 2023-11-22 |
| M1 Ultra (64c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 1030.0 | 83.7 | #4167 | 2023-11-22 |
| M2 Ultra (76c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 1238.5 | 94.3 | #4167 | 2023-11-22 |
| M2 Ultra (76c) | LLaMA-2 7B | Q4_0 | `c1d0e7a`+FA | 1489* | 125.2* | #4167, later build | 2026-08 |
| M3 Max (40c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 759.7 | 66.3 | #4167 | 2023-11-22 |
| M3 Ultra (80c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 1471.2 | 92.1 | #4167 | 2023-11-22 |
| M4 (10c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 221.3 | 24.1 | #4167 | 2023-11-22 |
| M4 Pro (20c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 439.8 | 50.7 | #4167 | 2023-11-22 |
| M4 Max (40c) | LLaMA-2 7B | Q4_0 | `8e672ef` | 885.7 | 83.1 | #4167 | 2023-11-22 |
| M5 (10c) | LLaMA-2 7B | Q4_0 | `c1d0e7a` | 722.8 | 31.9 | #4167 | 2026-08 |
| **M5 Pro (16c)** | LLaMA-2 7B | Q4_0 | `60081bb` | **1340.4** | **67.6** | #4167 | 2026-08 |
| M5 Max (40c) | LLaMA-2 7B | Q4_0 | `c1d0e7a` | 3220.0 | 119.9 | #4167 | 2026-08 |
| M5 Ultra (80c) | LLaMA-2 7B | Q4_0 | `c1d0e7a` | 4944.7 | 179.1 | #4167 | 2026-09 |
| M5 Max | Qwen3.5-35B-A3B | int4 MLX | Ollama 0.19 | 1851 | 134 | ollama.com/blog/mlx | 2026-03-30 |
| M4 Max (64 GB) | Gemma 4 31B | 4-bit MLX | mlx-lm | — | 22 | llms-explorer.com | 2026-06 |
| M3 Max | Gemma4-26B-A4B | UD-Q4_K_M + spec | llama.cpp | — | 206.4 | hiesch.eu | 2026-08-15 |
| M3 Max (40c) | Qwen3.5-35B-A3B | MLX 4-bit | oMLX | — | 90.1 | famstack-dev | 2026-06 |
| M2 Ultra (76c) | Qwen-2.5 (mixed) | 4-bit | MLX / llama.cpp / MPS | — | ~230 / ~150 / ~7–9 | arXiv 2511.05502 | 2025-11 |
| M5 (24 GB) | Qwen3-14B | 4-bit | MLX+TensorOps | TTFT 4.06× vs M4 | 1.19× vs M4 | machinelearning.apple.com | 2025-11-19 |
| M5 iPad Pro | Qwen3-8B | 4-bit MLX | MLX | 10k prompt: 18 s (M4: 81 s) | 1.5× vs M4 | macstories.net | 2025-11-20 |

**Do not average across this table.** Three confounds: build drift (`*` = M2 Ultra on a later build: same hardware, +20% pp / +33% tg); differing models and quants; and `pp` at 512 understating long-prompt cost — attention is quadratic, so 20k costs ~25× a 4k prompt. Seed from these; calibrate on the host.

---

## 5. Install paths that work

**llama.cpp.** `brew install llama.cpp` gives a Metal bottle immediately — the right default. Source build needs `cmake`, **absent here** *(measured)*: only `/usr/bin/make` and Homebrew `ninja` exist. `brew install cmake`, then `cmake -B build -DGGML_METAL=ON && cmake --build build -j`. **CLT suffices**; full Xcode does not — and since `xcodebuild -version` errors on a CLT-only host, any probe shelling out to it gives false negatives on most machines.

**mlx-lm.** `pip install mlx-lm`. Floors: Apple silicon, native arm64 interpreter, **Python ≥ 3.11**, **macOS ≥ 14.0** (`mlx`), **macOS ≥ 26.2** for M5 Neural Accelerators. Wheels are **arm64-only**: under Rosetta pip says "no matching distribution" — correct, but never why.

> **Trap, measured here:** the canonical MLX diagnostic `python -c "import platform; print(platform.processor())"` returns **empty string** on Python 3.14.7 / macOS 27.0.1 arm64 — not `arm`, not `i386`. A detector keyed on `== 'arm'` misclassifies a broken environment. Use `platform.machine() == 'arm64'` *(measured)* plus `uname -m`.

**The wire-memory limit.** Two ceilings; the first is advisory only:

- `recommendedMaxWorkingSetSize` — **19,069,665,280 B = 17.76 GiB** on this 24 GiB host, **74.0%** of physical *(measured)*. Exceeding it degrades rather than errors. A 48 GB M5 Pro measured 78.0% — **never hard-code a fraction, query it**.
- `maxBufferLength` — **13,639 MiB = 13.32 GiB**, exactly **0.75×** the working set *(measured)*. This *is* hard: a 20 GB tensor cannot exist in one buffer, so Q-Infer must never plan one allocation above it.
- `sysctl iogpu.wired_limit_mb` reads **0** *(measured)* — "OS default", not zero. `sudo sysctl iogpu.wired_limit_mb=N` raises policy at runtime only, reserves nothing, is unsupported, and resets on reboot. Pushing it near physical RAM triggers §7.1.

**Rosetta / arch traps.** A venv records the interpreter that made it; a later arm64 Python does not fix it — recreate. Homebrew must be `/opt/homebrew` (it is); `/usr/local` is x86.

**Gatekeeper.** Bottles and wheels are notarized; source builds are unsigned, and `xattr -d com.apple.quarantine <path>` unblocks a killed helper.

---

## 6. Tuning knobs that matter

| Knob | Range | Effect size | Notes |
|---|---|---|---|
| `-ngl` | 0–999 | **>5×**, all-or-nothing | Decides whether decode runs on GPU at all |
| `--flash-attn` / `-fa` | on/off | 0–15% tg, more at long ctx | Prerequisite for quantized `-ctk/-ctv`; M5 corpus rows assume it |
| `-ctk` / `-ctv` | f16, q8_0, q4_0 | KV 0.5× / 0.25× → longer ctx | <5% tg. q8_0 safe; q4_0 V-cache degrades long-context quality |
| `-b` / `-ub` | 2048 / 512 defaults | up to **~2×** prefill | `-ub` is the real knob (matmul tile); raising only `-b` does nothing. Single anecdote — **UNVERIFIED**. Costs compute buffer |
| `--parallel` | 1–8 | ~linear aggregate, per-stream latency up | Splits context across slots. For fan-out, not latency |
| `--mlock` / `--no-mmap` | flags | 0% → catastrophic | Avoids page-cache eviction. **Dangerous under pressure** — this host already has 10.7 GB swap used; `mlock` makes the kernel kill rather than reclaim |
| MLX quant bits | 4 / 6 / 8 | ~1.6–2× decode, 4-bit vs 8-bit | Bandwidth-proportional; 6-bit is the quality/latency knee |
| `OLLAMA_KEEP_ALIVE` | `0`, `5m`, `-1` | eliminates 0.5–60 s reload | `0` unloads at once; `-1` pins forever and is the usual cause of unexplained occupancy |
| `PYTORCH_MPS_HIGH_WATERMARK_RATIO` | 0.0–2.0, default **1.7** | hard OOM boundary | `1.0` = working set; `>1.0` exceeds it; `0.0` disables the limit and invites system OOM. Low watermark default 1.4 (unified) |
| `lowpowermode` | 0/1 | 10–30% decode | `0` on AC here *(measured)*; battery/Low Power Mode shifts results, so record it |
| `pmset -g thermlog` | n/a | n/a | Not a knob — it **never returns** (§7.2). Never call synchronously |

---

## 7. Failure modes

1. **Memory-pressure swap death spiral.** *Symptom:* decode collapses 10–100×, host hangs for minutes, no error. *Cause:* `iogpu.wired_limit_mb` near physical RAM; OS starved → compressor and swap thrash. Observed **live here**: 12.0 GB swap, **10.7 GB used**, 604,428 compressor pages. *Fix:* budget against working set **minus current usage**, cap ~70%.
2. **`pmset -g thermlog` never returns.** *Symptom:* provisioning hangs at "detecting thermal state". *Cause:* a **streaming log**, not a query — **verified: no return in 6 s**; `timeout(1)` is absent on stock macOS, so a plain wait blocks forever. *Fix:* 2 s deadline, then kill.
3. **`powermetrics` requires root.** *Symptom:* empty output, exit 1 — `must be invoked as the superuser`. *Fix:* degrade gracefully; `pmset -g therm` also fails with `0xe00002bc` *(measured)*, so **no unprivileged thermal read** exists here.
4. **ANE compile failures.** *Symptom:* CoreML compile error or silent GPU fallback. *Cause:* static fixed-shape graphs; channel counts constrained (M3 Pro: **only ch=512 compiles**). *Fix:* we refuse the ANE.
5. **MPS missing ops.** *Symptom:* `NotImplementedError`, or 100× slowdown. *Cause:* op falls back to CPU; the "4 GB tensor cap" failures on 3B+ models are this class. *Fix:* `PYTORCH_ENABLE_MPS_FALLBACK=1` makes it *run*, not fast.
6. **Ollama backend switch changes perf silently.** *Symptom:* a recipe benchmarked at 58 tok/s now differs with no config change. *Cause:* 0.19 (2026-03-31) moved Apple Silicon from llama.cpp Metal to MLX; Linux/Windows did **not**; only Qwen3.5-35B-A3B is accelerated, **>32 GB**. *Fix:* pin and record engine version.
7. **JACCL GID bug — `RTR failed with errno 22`.** *Symptom:* distributed init fails immediately and intermittently ("worked for days, failed after a reboot"). *Cause:* `Connection::info()` scans for an **IPv4-mapped** GID (`::ffff:a.b.c.d`); a TB RDMA port with no IPv4 address publishes only `fe80::`, the scan matches nothing, and an uninitialized GID reaches the peer. Regression from PR #3412; issue #3467 still **open**. *Fix:* static link-local IPv4 per port — `sudo ifconfig en3 inet 169.254.10.1 netmask 255.255.0.0 alias` (`networksetup -setmanual` for durability); disable `bridge0`. Errno 16 is separate: an orphaned runner holds the queue pair.
8. **`maxBufferLength` hard stop.** *Symptom:* Metal allocation abort on a model that "fits in RAM". *Cause:* one tensor exceeds 13.32 GiB though the total fits. *Fix:* shard.
9. **Build-drift misattribution.** *Symptom:* you "prove" M5 is 1.44× M4 and cannot reproduce it. *Cause:* M2 Ultra gains +33% tg from a rebuild alone. *Fix:* cross-commit ratios are upper bounds only.

---

## 8. Honest assessment

**Genuinely good at** capacity per dollar and per watt. A 128 GB M5 Max holds a 70B in 4-bit and decodes without swapping, in a laptop, silently. Unified memory means zero CPU↔GPU copies, which is why MLX reaches 0.74–0.83 of theoretical bandwidth — far above a discrete GPU over PCIe. M5's Neural Accelerators are real: prefill rose **3.3–4.1×** over M4 at equal core count, and M5 Max prefill of 3220 tok/s is ~43 TFLOPS effective.

**Weaker than the spec sheet suggests.** Three blunt points.

1. **Ultra-tier bandwidth plateaus.** Spec goes 800 → 800 → 800 → 1228 GB/s across M1–M5 Ultra, but *effective* decode bandwidth is **317 / 357 / 349 GB/s** for M1/M2/M3 Ultra — 0.40–0.45 of spec, vs 0.58–0.66 for the Max tier. Doubling memory controllers buys almost nothing. Only M5 Ultra breaks out (679 GB/s, 0.55), on a newer build with flash-attention, so its hardware share is smaller. **A claim of "350–460 GB/s plateau" overstates M1–M3 Ultra and understates M5 Ultra; the defensible statement is 317–357 GB/s on the 2023 build, M5 Ultra unresolved until measured same-build.** An M3 Ultra at ~2× the price of an M3 Max is ~1.4× the decode, not 2×.
2. **Decode is not a roofline.** `BW_spec / weight_bytes` over-predicts every row in §2 by 17–60%. Fitting `t_token = t0 + bytes/BW_eff` to the 7B rows gives a per-token fixed cost of ~5–8 ms, which dominates small weight footprints and MoE where only active parameters move. Naive calculators err optimistically.
3. **The software is the unstable part.** Engine choice moves decode 2.4× on identical hardware; one rebuild moved it 33%. Ollama changed backends under its users; JACCL breaks only when an APIPA address is absent; MPS is 10–30× off the pace. 2023 benchmarks are not comparable to 2026 builds, and ANE TOPS is INT8 marketing no LLM path reaches.

---

## 9. Implications for Q-Infer

**Tier assignment.** Tiers follow §2; A is the only one with a performance SLO.

| Tier | Definition | Support |
|---|---|---|
| **A (CI-gated)** | M5 family, M4 Pro/Max, M3 Ultra, M2 Ultra; ≥24 GB | Full — every release runs it |
| **B (best-effort)** | M1–M4 Max/Pro, ≥24 GB | Known-good configs; no perf SLO |
| **C (documented)** | Base M1–M5, 16 GB | Plan plus warning band; never CI |
| **Refused** | <16 GB, Intel, Rosetta, ANE-only, distributed | Hard error with reason |

**Engine adapters.** Two required: `llamacpp-metal` and `mlx`. `ollama` is optional **and must record `version`** — its backend moved at 0.19. `pytorch-mps` is documented-not-supported; `coreml-ane` and `jaccl` are out.

**Catalog must contain:** GGUF and MLX variants per model; per-quant weight bytes *and* peak-buffer bytes; KV bytes per token per quant so context cost is computed, not guessed; the MLX macOS floor (26.2) as a filter; the engine commit for every benchmark row.

**Fingerprint must detect:** `hw.model`, chip, GPU cores, RAM, `recommendedMaxWorkingSetSize`, `maxBufferLength`, `iogpu.wired_limit_mb`, `lowpowermode`, memory pressure and swap use, `rdma_ctl status` + per-port GIDs, arch via `platform.machine()` (**not** `platform.processor()`), Python/macOS/engine versions. Wired limit and swap-use separate a working plan from a swap spiral.

**CI hardware:** one Tier-A M5 Pro 24 GB (this class) for smoke and regression; one 128 GB Tier-A box for the 70B fit path; one Tier-B M1/M2 for fallback. No distributed CI in v1. Discard iteration 1 of any benchmark — warm-up poisons it.

**Tests that must exist:** (1) `recommendedMaxWorkingSetSize` / `maxBufferLength` parity against a Swift probe — match Metal, not a fraction; (2) known-answer decode on a pinned commit, tg within ±15% of corpus — the calibration canary; (3) a memory-refusal test rejecting a model that fits in RAM but exceeds the working set; (4) a hung-subprocess test on `pmset -g thermlog`; (5) an arch-misdetection test feeding `platform.processor() == ''`; (6) a corrupt-GID test asserting a readable error with the `ifconfig` remedy; (7) a cross-build guard refusing ratios across engine commits.

---

## 10. Sources

All accessed 2026-10-26.

1. Apple — *MacBook Pro Tech Specs*. https://www.apple.com/macbook-pro/specs/
2. Apple Support — *MacBook Pro 14-inch M5 Pro/Max Tech Specs*. https://support.apple.com/en-us/126318
3. Apple Newsroom — *M5 Pro and M5 Max* (2026-03-03, Fusion Architecture). https://www.apple.com/newsroom/2026/03/apple-debuts-m5-pro-and-m5-max-to-supercharge-the-most-demanding-pro-workflows/
4. Apple Newsroom — *Apple unleashes M5* (2025-10-15). https://www.apple.com/newsroom/2025/10/apple-unleashes-m5-the-next-big-leap-in-ai-performance-for-apple-silicon/
5. Ars Technica — *M5 Pro/Max departures* (2026-03-03). https://arstechnica.com/gadgets/2026/03/m5-pro-and-m5-max-are-surprisingly-big-departures-from-older-apple-silicon/
6. ggml-org — discussion #4167, *llama.cpp on Apple Silicon M-series* (opened 2023-11-22). https://github.com/ggml-org/llama.cpp/discussions/4167
7. Apple ML Research — *LLMs with MLX and the M5 GPU Neural Accelerators* (2025-11-19). https://machinelearning.apple.com/research/exploring-llms-mlx-m5
8. Ollama — *Powered by MLX on Apple Silicon* (2026-03-30). https://ollama.com/blog/mlx
9. arXiv 2511.05502, *Production-Grade Local LLM Inference on Apple Silicon*. https://arxiv.org/pdf/2511.05502
10. ml-explore — issue #3467, JACCL GID regression. https://github.com/ml-explore/mlx/issues/3467
11. ml-explore — PR #3412; PR #4191. https://github.com/ml-explore/mlx/pull/3412
12. MLX docs — *set_wired_limit*, *Build and Install*. https://ml-explore.github.io/mlx/build/html/install.html
13. PyPI — *mlx-lm 0.32.2*. https://pypi.org/project/mlx-lm/
14. PyTorch docs — *MPS Environment Variables*. https://docs.pytorch.org/docs/main/mps_environment_variables.html
15. maderix — *ANE Benchmark Report*. https://github.com/maderix/ane/blob/main/benchmarks/ANE_BENCHMARK_REPORT.md
16. hollance — *neural-engine: supported devices*. https://github.com/hollance/neural-engine/blob/master/docs/supported-devices.md
17. MacStories — *M5 iPad Pro Neural-Accelerated AI* (2025-11-20). https://www.macstories.net/stories/ipad-pro-m5-neural-benchmarks-mlx/
18. Seerum — *How much VRAM does a Mac have?* https://searoom.app/blog/unified-memory-gpu/
19. Hiesch — *LLM Benchmark Harness*. https://hiesch.eu/projects/llm-benchmark-harness/
20. famstack-dev — *local-llm-bench*. https://github.com/famstack-dev/local-llm-bench
21. Gingter — *Ollama Goes MLX* (2026-04-23). https://gingter.org/2026/04/23/ollama-goes-mlx/

**Local measurements (this host, 2026-10-26):** Mac17,9 / M5 Pro / 15 cores / 16 GPU cores; `hw.memsize`=25769803776; `recommendedMaxWorkingSetSize`=19069665280; `maxBufferLength`=13639 MiB; `iogpu.wired_limit_mb`=0; swap 12.0 GB / 10.7 GB used; `pmset -g therm` → `0xe00002bc`; `powermetrics` → root required; `pmset -g thermlog` → no return in 6 s; `platform.processor()` → `''`; `platform.machine()` → `arm64`; CLT only, `cmake` absent.
