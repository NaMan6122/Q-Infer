# Q-Infer — Product Requirements Document

**Working title:** Q-Infer
**One-liner:** *The right engine, on the right silicon, at the right settings — decided, installed and proven for you.*
**Document status:** v0.2 — Evidence-backed draft, for review
**Date:** October 2026
**Owner:** Founders / Product
**Audience:** Engineering, design, and anyone who will have to maintain this in two years

> **Provenance note.** Hardware figures and version numbers in this document were gathered from vendor documentation and public issue trackers in October 2026. Anything marked `UNVERIFIED` needs confirmation before it becomes a dependency in code. Specs change every quarter; §17 holds the numbers, but the *reasoning* in §2–§6 is the durable part.

### Changelog

**v0.3 (2026-10-26) — the dashboard.** ADR-0009 promotes the TUI from an installer wizard (FR-51) to a **first-class read-only dashboard** over the core's state, built with Ratatui, in the same static binary. A working skeleton exists at `crates/qinfer-tui` and renders the real M0 fingerprint and results. Requirements FR-55 … FR-59 added; the wizard remains in M2.

**v0.2 (2026-10-26) — evidence pass.** Three platform dossiers (`docs/research/`) and an M0 spike on Apple M5 Pro (`spikes/m0/REPORT.md`) are now the evidence base. Six corrections resulted, each of which changed a load-bearing claim:

1. **`η` → `k_cal`.** It is a probe-relative *calibration coefficient*, not an efficiency, and it is **not bounded by 1.0**. The spike measured **1.175** on Metal: the GPU pulls more bandwidth than a CPU memcpy can demonstrate.
2. **The bandwidth probe must saturate the memory system.** Single-threaded on the same machine: 131 GB/s. Fifteen threads: 228 GB/s. A silent 1.74× error in every downstream prediction.
3. **`t₀` is per model class, not global.** Measured 1.22 ms on a dense Metal path against ~8.4 ms reported for MoE on Ultra-class parts.
4. **NPU efficiency is 0.15–0.35, not 0.60–0.85.** On one Snapdragon X Elite, a 7B on CPU (24.8 tok/s) beats an 8B on the NPU (9.96 tok/s). Any "NPU is the fast path" assumption is now dead.
5. **The `sm_121` problem has moved.** It is no longer primarily wheel availability — the 2025 gap has largely closed via the `12.0` family target. The durable issue is that `sm_12x` lacks `tcgen05`/TMEM, so datacenter-Blackwell FP4 kernels must be rewritten rather than recompiled.
6. **Open questions Q1–Q10 now carry provisional decisions** (§16), reversible until M1.

---

## 0. How to read this document

| If you want… | Read |
|---|---|
| The pitch in 60 seconds | §1 |
| Why this is hard and why now | §2 |
| What we are deliberately refusing to build | §3.2 |
| Who pays and for what | §4 |
| The actual technical idea | §5 |
| Why Ollama doesn't already do this | §6 |
| What v1 must do, numbered | §7 |
| Which hardware we promise and which we don't | §8, §9 |
| How it's built | §10, §11 |
| How we'll know it worked | §12 |
| What will go wrong | §13 |
| When | §14 |
| Unresolved | §15 |
| Lookup tables | §17–§21 |

---

## 1. Executive summary

### 1.1 The product

Q-Infer is a **bare-metal inference provisioning and tuning toolkit**. You point it at a machine and a model; it fingerprints the silicon, predicts what that machine can actually sustain, chooses among competing inference frameworks rather than assuming one, installs a pinned and reversible configuration, runs a benchmark, and hands back a signed receipt that says *this is what we installed, this is what it achieved, and this is how close reality came to the prediction.*

It is not a chat UI, not a model runner with a friendly face, and not another wrapper around llama.cpp. It is the layer that decides **which** runtime, **which** quantisation, and **which** flags are correct for *this specific box* — and then proves it.

### 1.2 The problem in three sentences

Modern inference hardware has fragmented into at least six incompatible accelerator families — Apple Metal/ANE, NVIDIA CUDA (now with a second architecture `sm_121` for Grace-Blackwell desktops), AMD ROCm and Vulkan, Qualcomm Hexagon, Intel, and ARM server CPUs — each with its own runtime, its own model format, and its own definition of "quantised". Meanwhile every inference framework is excellent at one slice of that matrix and silently terrible outside it: vLLM is the right answer on two of the six families and an outright non-starter on the rest, llama.cpp runs everywhere but leaves every performance decision to the user, and Ollama hides the decisions entirely. The result is that a competent engineer who buys a $4,000 inference box still spends an evening in a terminal reading issue threads to discover that the wheels they just installed have no kernels for their compute capability.

### 1.3 The insight

**Decode throughput is a memory-bandwidth problem, and memory bandwidth is a number we can measure in ten seconds.**

A dense model decoding a token reads every active parameter once. So `tok/s ≈ achievable_bandwidth / bytes_per_token`, adjusted by a backend-efficiency factor that is stable enough to learn. That means we can *predict*, before downloading 40 GB of weights, roughly what any (machine, model, quant, runtime) combination will do — and therefore *choose* rather than guess. The prediction doesn't need to be perfect; it needs to be correct enough to rank candidates, and then be checked against a real measurement and corrected.

That single idea converts "inference setup" from a craft into a search problem, and gives us a flywheel: every benchmark a user runs makes the next plan more accurate.

### 1.4 What v1.0 is

- A single static binary (plus optional container), `qinfer`, for macOS, Linux and Windows-ARM64.
- `qinfer up` — zero-argument golden path: detect, propose three ranked plans, install one, benchmark it, print a receipt.
- A declarative spec (`qinfer.yaml`) for reproducibility, and a TUI for the "few clicks" audience.
- Certification for three platform families at launch (Apple Silicon, NVIDIA x86 + Grace-Blackwell, AMD/CPU fallback), with a further four at "supported" or "detected" tier (§8).
- A local OpenAI-compatible endpoint under supervision, with health, metrics and drift detection.

### 1.5 What success looks like

A DGX Spark owner runs one command and is serving a 70B-class model at a throughput number they can defend, in under fifteen minutes, without reading a GitHub issue. An Apple Silicon owner learns *before* trying that `vLLM` is not available to them, instead of after. A fleet operator can diff two machines and see that one of them is running 40% slower because the quant fell back to CPU.

---

## 2. Context and problem definition

### 2.1 The hardware bazaar

The machines our users own in 2026 do not share a single common denominator except "runs Linux-ish software badly".

| Family | Representative silicon | Memory model | Bandwidth | Real accelerator | The catch |
|---|---|---|---|---|---|
| Apple Silicon | M1→M4/M5 families, Ultra tiers | Unified, on-package | 68 → 800+ GB/s | Metal GPU; ANE nominally 38 TOPS | Metal ≠ CUDA; no vLLM/SGLang; ANE is not an LLM device |
| NVIDIA discrete | Ada / Blackwell RTX, RTX PRO | Dedicated VRAM | ~500 GB/s → 1.8 TB/s | CUDA cores + Tensor Cores | VRAM is the hard wall; sharing rules |
| Grace-Blackwell desktop | DGX Spark (GB10) | 128 GB **unified** LPDDR5X | **273 GB/s** | Blackwell GPU, `sm_121` | aarch64; `sm_12x` has **no `tcgen05`/TMEM**, so datacenter FP4 kernels don't port |
| Jetson-class | AGX Thor T5000 / T4000 | 128/64 GB unified LPDDR5X | 273 GB/s | Blackwell GPU, 40–130 W | JetPack pinning; embedded kernel quirks |
| AMD APU | Ryzen AI Max+ 395 "Strix Halo" | 128 GB unified LPDDR5X | 256 GB/s theoretical, ~215–239 measured | RDNA 3.5, `gfx1151` | ROCm lags; Vulkan often wins; GTT carveout is a boot-time decision |
| Qualcomm | Snapdragon X Elite / X2 Elite | LPDDR5X, shared | ~135–230 GB/s class | Hexagon NPU, Adreno GPU | NPU path needs pre-compiled per-chipset bundles |
| Intel | Lunar Lake / Arc / Xeon | shared / VRAM | varies | NPU, Xe GPU | OpenVINO/SYCL fragmentation |
| ARM server | Graviton 4, Axion, AmpereOne | DDR5, many channels | 300–500+ GB/s | CPU only (i8mm/SVE2/SME) | No GPU; bandwidth-per-core governs tok/s |
| Legacy/other | Ascend (CANN), Moore Threads (MUSA) | — | — | — | Small ecosystems, binary blobs |

The important observation is in the fourth column. **The machines people actually buy for local inference — Apple Ultra, DGX Spark, Strix Halo, Thor — are all unified-memory systems clustered around 200–800 GB/s.** They have wildly different compute, wildly different software stacks, and remarkably similar decode ceilings. That similarity is what makes a single predictive model feasible across the whole matrix.

### 2.2 The six walls

Every failed installation we studied collapses into one of six categories. These *are* the product requirements.

**(i) The fit wall.** The model must fit — weights + KV cache + activation headroom. Unified-memory machines make this worse, not better, because the OS, the runtime, and the model compete for one pool. DGX Spark's own guidance is explicit that `--gpu-memory-utilization` must leave room for the OS and container runtime in the same 128 GB that holds the weights [vLLM blog, DGX Spark, Jun 2026]. Meanwhile vLLM's default of 0.90 pre-allocates almost the entire device, which is correct on an H100 you own exclusively and hostile on a workstation you also use.

**(ii) The architecture wall.** A GPU is not "an NVIDIA GPU"; it is a specific SM target, and the target decides which kernels exist. DGX Spark's GB10 reports compute capability **12.1 (`sm_121`)**. The wall has two layers, and conflating them is how products end up reciting stale facts:

- **Packaging — largely solved.** Through early 2026, published vLLM aarch64 wheels compiled for `sm_120` and below, and GB10 users hit hard failures or silent PTX-JIT fallbacks [issue #36821; PR #38484]. That has substantially closed: the 0.29.0 aarch64 cu130 wheel now runs on GB10 via the **`12.0` family target** at **16.4 tok/s against 16.3 tok/s** for a native-`sm_121` build [dossier 02, citing PRs #38126 and #43003]. Any document still warning "no `sm_121` wheels" is reciting history.
- **Architecture — not solved.** `sm_12x` has **no `tcgen05` / TMEM**, confirmed by NVIDIA staff. Datacenter-Blackwell FP4 kernels cannot be *recompiled* onto a Spark; they must be **rewritten**. Jetson Thor (`sm_110`) *does* have `tcgen05`, so the Spark is the odd one out in its own bandwidth class. A multi-quarter gap, not a flag.

What survives to the product: **capability cannot be inferred from a model name, a version number, or a marketing tier.** One extension (`_qutlass_C`) still ships `sm_100`-only cubins and fails at *kernel launch* rather than at import, because the Python guard raises nothing when a shared object loads with the wrong cubins inside. So the catalogue must record **per-artifact cubin coverage**, not a version range — while knowing that `cuobjdump -lelf` cannot distinguish `12.0` from `12.0f`, which makes a static arch list necessary but not sufficient. Only an execution probe settles it.

**(iii) The quantisation–backend wall.** Quantisation is not a global choice; it is a function of the kernel that will consume it.

- On Hexagon NPU, `Q4_0` is the clean NPU path; `Q4_K_M` causes some tensors to fall back to CPU, silently destroying the reason you bought the NPU [Qualcomm GenieX docs].
- On ARM CPUs, `Q4_0` gains nothing from KleidiAI because llama.cpp's default aarch64 path already repacks it — but `Q8_0` gains **1.73× on prefill**, and repacking costs roughly **2× resident memory** regardless [kleidibench findings].
- On Blackwell, `NVFP4` is the intended format and roughly halves bytes-per-token versus FP8 — which, in a bandwidth-bound regime, is close to a 2× decode win for free.
- On Apple, MLX 4-bit DWQ is a different artifact from GGUF Q4_K_M, produced by a different toolchain, hosted by a different org.

A tool that offers "a 4-bit option" has understood nothing. The correct unit is **(model, artifact, backend, accelerator)**.

**(iv) The flag wall.** `--gpu-memory-utilization`, `--max-model-len`, `--max-num-seqs`, `--kv-cache-dtype`, `-ngl`, `--flash-attn`, `-ctk/-ctv`, `--ubatch-size`, `--spec-type draft-mtp`, `--enforce-eager`, `--tp`, `NCCL_P2P_DISABLE`, `iogpu.wired_limit_mb`, `amdgpu.gttsize`, `amd_iommu=off`, `ROCBLAS_USE_HIPBLASLT`… Each is documented somewhere, none are documented together, and several interact non-linearly. On Strix Halo the *documented best* backend differs per model — Vulkan beat ROCm by 29% on verification for one model and lost by 35–50% on another on the same box [strix-halo measurements]. There is no way for a human to keep this in their head; there is a way for a machine to search it.

**(v) The reproducibility wall.** Installations are not reproducible because they are not pinned. `pip install vllm` resolves to whatever is current; a Homebrew tap moves; Ollama silently changed its macOS backend from llama.cpp to MLX in v0.19 and decode on one published test roughly doubled (58 → 112 tok/s) without the user changing anything [llms-explorer]. Silent upgrades are indistinguishable from regressions. Nobody can answer "what exactly was running on this box last Tuesday?".

**(vi) The trust wall.** Setup instructions on the internet are `curl | sh` with root. In regulated and air-gapped environments — the places with the most bare metal — that is disqualifying. A toolkit that wants to be allowed near a DGX has to be auditable, offline-capable, and hostile to implicit privilege escalation.

### 2.3 The manual ritual, and what it costs

What a competent engineer does today, with a new box:

| # | Step | Typical time | Failure mode |
|---|---|---|---|
| 1 | Identify exactly what the machine is | 5–30 min | `nvidia-smi` lies about compute capability on new parts; ARM64 flags differ from x86 |
| 2 | Decide which framework is even possible | 10–60 min | Discovering vLLM doesn't support your platform *after* installing it |
| 3 | Find a model artifact in the right format | 5–40 min | GGUF conversion, MLX conversion, per-chipset QNN bundle |
| 4 | Choose a quantisation | 10–60 min | Picking Q4_K_M on Hexagon; picking Q8_0 on ARM CPU |
| 5 | Install, pin, and resolve driver/toolkit versions | 15–180 min | `sm_121` mismatch; CUDA/driver skew; aarch64 wheel absent |
| 6 | Tune flags by trial | 30–240 min | OOM loops; KV cache blowups; thermal throttling mid-benchmark |
| 7 | Benchmark properly | 15–90 min | Measuring a spec-decode config that emits `1. 1. 1.` and looks fast |
| 8 | Write it down so it can be repeated | 0–∞ min | Usually not done at all |

Call it **two to eight hours per machine, per model**, with a meaningful chance of ending in surrender. Our target is **under fifteen minutes, with a receipt**, and a hard guarantee that we never waste the first hour on a plan that cannot work.

### 2.4 Why now

Four things became true in the last eighteen months:

1. **Unified-memory inference went mainstream.** DGX Spark, Strix Halo, Thor and the Mac Ultra line all put 128 GB+ of addressable memory in a desktop box. A new buyer class exists that did not exist in 2023, and they arrive with no priors.
2. **The framework market stopped consolidating.** vLLM, SGLang, TensorRT-LLM, llama.cpp, MLX, ExecuTorch, QNN and ONNX Runtime are all actively maintained and none is winning outright. Multi-framework selection is now a permanent condition, not a transitional one.
3. **Quantisation formats proliferated.** NVFP4, MXFP4, FP8, AWQ, GPTQ, GGUF K-quants, MLX DWQ, QNN bundles. Choice is now a genuine optimisation problem with a quality/throughput Pareto frontier.
4. **Nobody owns the decision.** The tools that exist are either *engines* (which cannot recommend a competitor) or *app stores* (which cannot recommend anything but their own runtime). The advisory layer is vacant.

---

## 3. Goals, non-goals, and principles

### 3.1 Goals

| ID | Goal | Measure |
|---|---|---|
| G1 | Eliminate blind installation | ≥90% of Tier-A plans reach a serving endpoint on first attempt |
| G2 | Make the hardware decision explainable | Every plan ships a human-readable rationale and a stated risk |
| G3 | Make setup reproducible | Any receipt can be replayed to an identical state on identical hardware |
| G4 | Make performance predictable | Fit/no-fit decided exactly; decode tok/s within ±25% MAPE for dense models after on-box calibration (§5.2.4) |
| G5 | Make the whole surface auditable | Pinned hashes, signed receipts, offline mode, no implicit `sudo` |
| G6 | Cover the actual hardware people own | Tier-A certification on Apple, NVIDIA x86, Grace-Blackwell by GA |

### 3.2 Non-goals (v1)

We are **not** building, and will say so publicly:

- **A chat interface or model hub.** We serve an OpenAI-compatible endpoint and get out of the way. Open WebUI exists.
- **An inference engine.** We will never write our own kernels. We integrate, pin, and tune other people's. This is a hard architectural boundary — the moment we ship a kernel, we become a framework and lose the right to recommend one.
- **A trainer or fine-tuner.** Weights in, tokens out. Fine-tuning affects which artifacts exist, not what we do with them.
- **A cluster orchestrator.** No Kubernetes operator, no multi-tenant scheduler, no autoscaling. llm-d and KServe do this; we will integrate with them, not replace them.
- **A cloud service.** No hosted control plane in v1. Telemetry is opt-in and content-free.
- **An NPU miracle.** We will not claim that ANE or Hexagon gives you a fast 70B. See §8.4 — we will actively talk users *out* of it.

### 3.3 Principles

1. **Predict before you download.** Any question we can answer from a fingerprint and 10 seconds of microbenchmark, we answer before spending the user's bandwidth.
2. **Never install what we cannot explain.** A plan without a rationale is a bug.
3. **Pin, always.** Version ranges are a form of lying about reproducibility.
4. **Measure, then admit when we were wrong.** Every receipt records prediction error. Our model is allowed to be wrong; it is not allowed to be silently wrong.
5. **No surprise privilege.** We tell you exactly which steps need elevation and refuse to run them implicitly.
6. **Honest unsupported.** "This machine cannot run that model at a useful speed" is a valid, valuable output. Refusing is a feature.
7. **The engine is not the product.** We are a decision system with an installer attached.

---

## 4. Users and jobs to be done

### 4.1 Personas

**P1 — "Devon", the DGX Spark owner (primary).**
Spent $4k on a GB10 box. Competent Python user, not a CUDA engineer. Wants a 70B-class model serving locally for agent workloads. Has already lost a weekend to `sm_121`, possibly without knowing that's what happened.
*Job:* "Get the most capable model this box can serve, working tonight, and tell me if I bought the wrong box."
*Success:* one command, a number, and a URL.

**P2 — "Priya", the Apple Silicon power user (primary).**
M4 Max or M3/M4 Ultra. Runs Ollama because it works, but suspects she's leaving 30% on the table and can't tell whether that's true.
*Job:* "Tell me the fastest honest configuration for my Mac, and stop me from trying things that don't work here."
*Success:* learns that MLX 4-bit DWQ beats her current GGUF setup, or learns that it doesn't on her specific chip, with evidence.

**P3 — "Marcus", the edge fleet operator (secondary, high-value).**
Deploys identical workloads to Snapdragon or Jetson devices in the field. Cares about sustained power, thermals and drift, not peak tok/s.
*Job:* "Guarantee every unit in my fleet runs the same config and hasn't silently degraded."
*Success:* fleet diff and air-gapped bundle.

**P4 — "Dr. Chen", the performance engineer (secondary, credibility).**
Runs benchmarks for a living. Distrusts marketing. Will not use anything that hides its methodology.
*Job:* "Give me a reproducible harness and let me see the raw numbers."
*Success:* loves the receipt format, publishes it, becomes our best marketing.

**P5 — "Sam", the platform engineer evaluating on-prem (tertiary).**
Comparing a bought box against cloud GPU rental.
*Job:* "What throughput will I actually get per dollar?"
*Success:* uses the predictor's cost-per-million-tokens output to justify or kill a purchase.

### 4.2 Anti-personas

- Someone who wants a chatbot in five minutes and will never read a flag: **send them to Ollama/LM Studio.** They are not our user and pretending otherwise will ruin the product.
- Someone who wants to train: not us.
- Someone running 500 GPUs under Kubernetes: not v1; they have platform teams.

### 4.3 Jobs-to-be-done (ranked)

1. **"Which model can this machine serve at a speed I'd accept?"** — the fit question.
2. **"Which runtime should I use?"** — the selection question.
3. **"How do I install it without breaking my system?"** — the provisioning question.
4. **"Am I getting what I paid for?"** — the verification question.
5. **"Can I reproduce this next month / on the next machine?"** — the receipt question.
6. **"Why is this one slower than that one?"** — the diff question.

---

## 5. The core idea: Predict → Plan → Apply → Verify → Record

This is the spine of the product. Everything else is a surface on top of it.

```
   ┌────────────┐   ┌───────────┐   ┌─────────┐   ┌──────────┐   ┌─────────┐
   │ FINGERPRINT│──▶│  PREDICT  │──▶│  PLAN   │──▶│  APPLY   │──▶│ VERIFY  │
   │  what is   │   │  roofline │   │ ranked  │   │ pinned,  │   │ bench + │
   │  this box  │   │  ceiling  │   │ options │   │ reversible│  │ receipt │
   └────────────┘   └───────────┘   └─────────┘   └──────────┘   └────┬────┘
                                                                     │
                        ┌────────────────────────────────────────────┘
                        ▼
                 ┌──────────────┐
                 │  CALIBRATION │  predicted vs measured, per
                 │    STORE     │  (arch, backend, model-shape)
                 └──────────────┘
```

### 5.1 Fingerprint: what is this machine, really

A normalised **Capability Fingerprint (CFP)** — a single JSON document with per-field provenance and confidence. It is the only thing downstream components are allowed to consume; no component may shell out to `nvidia-smi` on its own.

Design rules:

- **Facts, not brands.** `{"vendor":"nvidia","arch_id":"sm_121","compute_cap":[12,1]}` — not `"gpu":"Blackwell"`. Brand names live only in the display layer.
- **Measured beats declared.** A spec sheet says 273 GB/s; we measure what the box actually sustains and use the measured figure with a confidence interval. Spec numbers are retained as `declared` for comparison, because the *ratio* between them is itself diagnostic (a machine at 60% of declared bandwidth has a thermal or channel problem).
- **Provenance per field.** Every value carries `source: "nvidia-smi" | "sysctl" | "microbench" | "catalog"` and `confidence: 0..1`. When we're guessing, we say so. When we guess wrong, the receipt tells us.
- **Three-layer resolution.** (a) identifier reads, (b) capability probes (does this backend actually execute a tiny kernel?), (c) microbenchmarks (bandwidth, GEMV, GEMM). Layer (b) is the one everybody skips, and it is the one that catches `sm_121`-class problems — *because a compute capability string is a claim, while a successful kernel launch is a fact.*

Representative shape (abridged; full field list in §17.3):

```jsonc
{
  "schema": "qinfer/cfp@1",
  "fingerprint_id": "cfp_01J8Z...",
  "identity": { "os":"linux", "distro":"dgx-os-7.5", "kernel":"6.11", "arch":"aarch64",
                "virt":"bare", "form_factor":"desktop", "chassis_power_w":240 },
  "accelerators": [
    { "id":"gpu0", "vendor":"nvidia", "arch_id":"sm_121", "compute_cap":[12,1],
      "sm_count":48, "unified":true,
      "memory": {"total_bytes":137438953472, "dedicated_bytes":0, "shared_with_host":true},
      "bandwidth": {"declared_gbs":273, "measured_gbs":251.4, "ci95":[247.1,255.8],
                    "method":"stream_triad_gpu@3GiB", "confidence":0.93},
      "features": {"fp4":true,"fp8":true,"bf16":true,"int8_dot":true,
                   "nvfp4_layout":true,"tensor_cores_gen":5},
      "runtime": {"cuda":"13.0.88","driver":"580.173.02","arch_supported":[100,110,120]} }
  ],
  "cpu": { "arch":"aarch64", "model":"Cortex-X925/A725", "cores":20, "isa":["neon","dotprod","i8mm"] },
  "memory": { "total_bytes":137438953472, "available_bytes":118000000000, "swap":{...} },
  "runtime_gaps": [
    { "component":"torch", "issue":"no sm_121 cubins in published wheel",
      "consequence":"PTX JIT or launch failure", "source":"probe:test_kernel_launch" }
  ]
}
```

`runtime_gaps` deserves emphasis: it is a first-class output, not a diagnostic afterthought. The single most useful sentence this product can say is *"your platform is known not to have working kernels for X — here is the workaround."*

### 5.2 Predict: the roofline ceiling

For a single-stream decode, the model is deliberately simple:

```
   t_token  ≈  t₀  +  bytes_per_token / BW_eff
   decode_tok_s  ≈  1 / t_token

   bytes_per_token ≈ active_params × bytes_per_param     # weight stream, dominant term
                   + kv_bytes_per_token                   # grows with context
```

Two parameters, and both matter:

- **`BW_eff`** is *effective* decode bandwidth, which is a measured constant and **not** the spec-sheet number. Backing it out of published benchmarks (`tok/s × bytes_per_token`) gives a remarkably consistent picture, and it is bad news for expensive hardware:

  | Machine | Spec BW | Effective decode BW | Ratio |
  |---|---|---|---|
  | M1 base / M4 | 68 / 120 GB/s | 54 / 92 | ~0.78 |
  | M2 Max / M4 Max | 400 / 546 | 252 / 317 | 0.63 / 0.58 |
  | **M3 Ultra** | **819** | **352** | **0.44** |
  | **M5 Ultra** `UNVERIFIED` | **1228** | **684** | **0.56** |
  | DGX Spark (GB10) | 273 | 231 | 0.85 |
  | Jetson Thor | 273 | 186 | 0.68 |
  | Strix Halo | 256 | 183 | 0.85 |

  **Apple's effective decode bandwidth plateaus at ~317–357 GB/s across the M1–M3 Ultra tiers**, whatever the spec sheet claims (819 in each case) — 0.40–0.45 of spec, derived from published Q4_0 runs [dossier 01]. A 679 GB/s M5 Ultra figure exists but comes from a newer build with flash-attention enabled, so it is *confounded, not contradictory*; treat the M5 Ultra as unresolved. A tool that used spec bandwidth would over-predict an M3 Ultra by **2.3×** — exactly the kind of confident wrongness that destroys trust in a product like this.

- **`t₀`** is a fixed per-token cost — kernel launch, routing, sampling, scheduler — of roughly **8–9 ms** on Apple Ultra-class parts. It is invisible on large dense models and dominant on small ones and on MoE. This single term explains the most counter-intuitive result in local inference: on one M3 Ultra, a 30B-A3B MoE decodes at **95 tok/s** versus **34 tok/s** for a dense 32B — a 2.8× win, not the ~10× the parameter ratio suggests. The active-parameter ratio correctly predicts *bytes read*; `t₀` correctly predicts *why you don't get the win you were promised*.

`k_cal` — the **calibration coefficient** — is the ratio between the bandwidth the probe measured and the bandwidth the executing device actually achieves, stored per `(backend, probe_method, arch_family, quant_class, model_shape_class)`.

It is **not an efficiency, and it is not bounded above by 1.0.** The M0 spike measured **`k_cal` = 1.175** on Apple Metal: a 3B model at Q8_0 moved 3.61 GB per token at 68.4 tok/s, implying the GPU drew **247 GB/s** from DRAM while a saturated fifteen-thread CPU memcpy reached only **228 GB/s** (§21). The GPU legitimately outruns the CPU probe. Clamping this to 1.0 because "efficiency cannot exceed one" would under-predict every plan on that machine by 17 %.

Observed bands, now including the NPU carve-out that dossier 03 forced:

| Path | `k_cal` | Note |
|---|---|---|
| Dense GPU, unified memory | 0.70–1.10 | may exceed 1 because the probe is CPU-side |
| MoE | 0.40–0.60 | `t₀` and routing dominate |
| NPU (Hexagon-class) | **0.15–0.35** | the CPU can be 2.5× faster on a larger model |
| Any path that spills | < 0.15 | not a bandwidth regime at all — see §5.2.1 |

Why this is the right primitive:

- It is **cheap** — one bandwidth microbenchmark and an artifact's metadata.
- It is **portable** — it applies to CUDA, Metal, Vulkan, ROCm, Hexagon and CPU alike, because they are all memory-bound in decode.
- It is **falsifiable** — we can check it against a real benchmark and record the error, which is the only way a performance model earns trust.
- It explains the counter-intuitive results users hit constantly: why a 30B-A3B MoE beats a dense 32B by ~2.8× rather than ~10× (`t₀` eats the difference); why `Q8_0` beats `Q4_0` on ARM CPU prefill but loses on decode past ~3B; why NVFP4 on Blackwell is close to a 2× decode win over FP8; why an M4 Max can beat a DGX Spark on a small model while losing badly on memory capacity.

Prefill is modelled differently — it is compute-bound, so it is `FLOPs / (achieved_TFLOPs)` with its own efficiency table. This is why the product reports **TTFT and TPOT separately** rather than one "tokens/sec" number: a plan can be excellent at one and bad at the other, and which one matters depends on whether the user is chatting, running RAG, or doing batch summarisation.

Batch size is handled by the standard two-regime model: at low concurrency, weights are streamed per sequence and aggregate throughput scales nearly linearly; past the knee, weights are amortised across the batch and the KV cache dominates memory. The planner must know which regime it is optimising for, so **concurrency is a required input, defaulting to 1, not an afterthought.**

Predictions are ranges, not points: `{p50, p10, p90, confidence}`. A wide interval is an honest signal that we should benchmark before promising.

#### 5.2.1 Where the model breaks, and what we do about it

Six failure modes are known and must be handled explicitly rather than averaged away:

| Failure | Symptom | Handling |
|---|---|---|
| **Spilling** — model + KV exceeds usable unified memory | Throughput an order of magnitude below the ceiling (a 235B-A22B on one Spark lands at **1.8 tok/s, ~8% k_cal**) | Detect by comparing measured against a floor; **invalidate the prediction** rather than report a 10× error, and tell the user they are paging |
| **Hybrid / sliding-window / MLA attention** | KV bytes wrong by 5–20× | Require `config.json` fields (`num_key_value_heads`, `head_dim`, `sliding_window`); **refuse to predict** if absent rather than guess |
| **New kernels** | Same box, same model, same quant, 2× difference (CUTLASS tile support moved GB10 from 12 → 24 tok/s) | That's what the calibration store is for; first-run prediction carries a wider interval |
| **Thermal / firmware clamps** | 27.5 → 2.2 tok/s from a firmware GPU clock clamp with **no thermal event reported** | Sustained-vs-burst ratio in the receipt (`thermal_derate`); never trust a single burst measurement |
| **MoE routing imbalance** | Batch-dependent expert union; removal of 2/8 experts gave 1.08–1.43×, and the gain differs by quant and batch | Treat MoE k_cal as a range keyed on batch; do not claim ±25% |
| **Speculative decoding acceptance** | Bimodal: **+50% to −80%**. Naive κ=8 measured **−79.9%** (56.5 → 11.4 tok/s) on one model | Never enable speculation without measuring acceptance; the planner treats it as a *candidate to test*, not a default |

The speculative case deserves emphasis because it is a trap the whole industry falls into: a configured draft depth that *looks* fast can be emitting degenerate output. Verification therefore includes a correctness check (§5.5), and the planner will not silently enable a 2× flag it has not measured on this box.

#### 5.2.2 Batching changes the shape of the answer

Single-stream decode is the simplest case, not the general one. At batch size B:

```
t_step(B) ≈ max( (W + B·ctx̄·k)/BW_eff , 2·N_active·B/F_eff )
Θ(B) = B / t_step(B)
```

Weight amortisation stops helping when `B·ctx·k = W` — for an 8B at 4-bit that is **B ≈ 7 at 4k context** and **B ≈ 0.9 at 32k**. Past that point KV read, not weights, sets the floor, and aggregate throughput begins to trade directly against per-user latency.

Critically, unified-memory machines have a **3–15× lower ridge point** than an H100 (their FLOPs-to-bandwidth ratio is far worse), which means they saturate with very few concurrent users and then regress. A DGX Spark can reach 14× single-stream throughput at B=256 on a MoE, but an M3 Ultra MoE plateaus at ~3.2×. **Concurrency is therefore a required planning input (§5.3), not a constant.**

#### 5.2.3 KV cache arithmetic is a first-class constraint

```
kv_bytes = 2 (K,V) × n_layers × n_kv_heads × head_dim × ctx × bytes_per_element
```

with one correction that matters: `q8_0` carries an fp16 scale per 32 elements, so it is **1.0625 B/element (53.1% of fp16), not 50%**.

| Model | layers / kv-heads / head_dim | fp16 per token | @4k | @32k | @128k |
|---|---|---|---|---|---|
| Llama-3.1-8B | 32 / 8 / 128 | 128 KiB | 0.50 GiB | 4.0 | 16.0 |
| Qwen3-8B | 36 / 8 / 128 | 144 KiB | 0.56 | 4.5 | 18.0 |
| Qwen3-32B | 64 / 8 / 128 | 256 KiB | 1.00 | 8.0 | 32.0 |
| Llama-3.3-70B | 80 / 8 / 128 | 320 KiB | 1.25 | **10.0** | **40.0** |

At 128k, a 70B's KV cache (40 GiB) **exceeds its own 4-bit weights** (35 GiB) at batch size 1. This is the arithmetic that makes "long context" a different product decision from "big model", and it is why the planner must treat context length as a hard constraint with a computed cost rather than a slider.

#### 5.2.4 What we are allowed to promise

| Claim | Honest answer |
|---|---|
| Fits / does not fit | **Exact.** This is arithmetic, not estimation — we must never be wrong here |
| Dense decode tok/s, first run, single stream | ±40–50% (wide interval, clearly labelled) |
| Dense decode tok/s **after one on-box calibration** | **±25%** claimed; **1.45 % MAPE measured** across three dense models on one Metal box (§21). Encouraging, not yet general |
| MoE decode tok/s | ±25% only with a per-model `t₀`; ±40% otherwise. Say which one you're getting |
| Prefill / TTFT | **±2× first run**, ±25% after calibration — MFU varies 20–60% by backend on identical silicon |
| Aggregate throughput at B > 1 | An **interval**, not a point: `min(bandwidth-bound, compute-bound)` |
| Rank ordering of plans | **The real promise.** We must get the *order* right even when the numbers are approximate |

That last row is the actual product requirement. A user needs to know which of six plans to pick; they do not need us to predict 31.2 tok/s when reality is 29.4. If we cannot order plans correctly on a given platform, that platform does not get Tier A.

### 5.3 Plan: choose, with reasons

A **plan** is a fully-resolved, executable proposal. Not a suggestion — every choice is made and pinned.

```yaml
plan_id: pln_7f3a91
objective: max_speed
inputs:
  model: qwen3-32b-instruct
  context: 32768
  concurrency: 4
  quality_floor: "within 2% of bf16 on our eval suite"
selection:
  framework: vllm
  framework_version: 0.29.0
  distribution: container@sha256:…            # digest, never a tag
  artifact: <org>/<model>-NVFP4               # format, quant, publisher
  artifact_sha256: …
  device: gpu0
  backend_kernel_path: nvfp4_gemm
predicted:
  ttft_ms:   {p50: 420, p10: 380, p90: 520}
  decode_tps:{p50: 31.2, p10: 27.0, p90: 35.5}
  vram_peak_gb: 96.4
  confidence: 0.78
flags:
  gpu_memory_utilization: 0.72     # leaves 35 GB for OS/container in the unified pool
  max_num_seqs: 4
  kv_cache_dtype: fp8
  enable_prefix_caching: true
  enforce_eager: false             # cuda graphs on; sm_121 cubins present
rationale:
  - "NVFP4 halves bytes-per-token vs FP8; decode is bandwidth-bound at 273 GB/s."
  - "gpu_memory_utilization lowered from 0.90 because memory is unified with the OS."
  - "max_num_seqs 4 because Spark's 273 GB/s is better suited to small batches."
risks:
  - "qutlass kernels absent for sm_121 in this wheel; MXFP4 transform path disabled."
  - "Thermal throttling possible after ~10 min sustained; receipt will record clocks."
fallbacks: [pln_7f3a92, pln_7f3a93]
```

Three things make this different from a config generator:

1. **`rationale` is mandatory and machine-generated from the model**, not hand-written prose. If the planner can't explain why, the plan is rejected.
2. **`risks` come from the fingerprint's `runtime_gaps`**, so known-broken paths are surfaced before install rather than discovered at launch.
3. **`fallbacks` are pre-computed.** When verification fails, we don't drop the user into a debugger; we move to the next plan.

**Objectives** are explicit optimisation targets, because "optimal" is meaningless otherwise:

| Objective | Maximise | Subject to |
|---|---|---|
| `max_speed` | decode tok/s at target concurrency | context ≥ N, quality floor |
| `balanced` (default) | tok/s × quality score | fit, context ≥ N |
| `max_context` | context window | decode ≥ 5 tok/s |
| `min_latency` | P95 TTFT at prompt length L | decode ≥ 10 tok/s |
| `min_power` | tokens per joule | decode ≥ 5 tok/s, thermals |
| `fit` | any working config | decode ≥ 1 tok/s |

The planner is a **ranked search with hard constraints**, not a rule engine. Hard constraints (does not fit / arch unsupported / licence forbids / capability probe failed) prune; the objective orders what remains. If nothing survives, we say why — and that is a legitimate, useful answer.

### 5.4 Apply: hermetic provisioning

Rules:

- **Every artifact is pinned by content hash** — container digests, wheel hashes, model shas, GGUF shas. Tags and version ranges are rejected by the schema.
- **Transactional.** Install into an isolated prefix (`~/.qinfer/envs/<plan_id>/`), or a container. Nothing outside that prefix changes without an explicit, logged step.
- **Rollback is one command and always available**, including after a partial failure.
- **No implicit privilege.** Steps requiring `sudo`/admin are enumerated in the plan, shown before execution, and can be executed by the user manually via `--print-sudo`. We never pipe a remote script into a shell. Ever.
- **Offline by design.** Every install resolves to a *bundle manifest* that can be exported, moved on a USB stick, and applied on an air-gapped box (§10.6).
- **Vendor SDKs are never redistributed.** QNN/QAIRT, CUDA, JetPack and CoreML toolchains are fetched from the vendor with the user accepting that vendor's terms, or supplied by the user. This is a legal constraint, not a preference.

### 5.5 Verify: the receipt

Verification is a fixed protocol, not "run it and see":

1. **Smoke** — model loads, one token generates, endpoint answers `/v1/models`.
2. **Correctness** — a deterministic-output check (fixed seed, temp 0, known prompt) compared against a recorded expectation. This catches the class of failure where speculative decoding produces confidently wrong output at impressive speed.
3. **Performance** — prefill (`pp`) and decode (`tg`) at fixed prompt/generation lengths, ≥3 repetitions, plus a concurrent-load run at the plan's target concurrency.
4. **Stability** — a sustained run long enough to expose thermal throttling (default: 10 min for desktop-class, 3 min for laptop/embedded), recording clock and power where available.
5. **Prediction check** — measured vs predicted, with the delta written into the receipt.
6. **Cache-state control** — every run declares cold or warm prefix cache, because the two are not comparable.

**Noise discipline.** This deserves its own paragraph because it is where benchmark tools quietly lie. Measured run-to-run variance on *identical* configurations reaches **±30% from prefix-cache warming alone**, and a llama.cpp build change alone moved `tg128` by **+33%** on identical hardware. Therefore every receipt records: cache state, engine build hash, repetition count with median and p95, memory pressure and swap at measurement time, and the ambient power/thermal mode. The first run is always discarded. **We do not report a difference below 10% as meaningful, and the tool says so out loud** rather than letting a user chase noise — which is the single most common way performance work wastes a week.

A **receipt** is the resulting signed record:

```jsonc
{
  "schema":"qinfer/receipt@1","receipt_id":"rcpt_…","plan_id":"pln_7f3a91",
  "fingerprint_id":"cfp_…","created":"2026-10-26T09:14:22Z",
  "environment":{"qinfer":"1.0.0","host":"spark-01","duration_s":742},
  "installed":[{"component":"vllm","version":"0.29.0","digest":"sha256:…"}],
  "measured":{
    "ttft_ms":{"p50":438,"p95":561},
    "decode_tps":{"p50":29.4,"p95":31.1},
    "prefill_tps":812,
    "vram_peak_gb":97.1,
    "sustained_min":{"decode_tps":26.8,"throttled":true,"clocks_sm_mhz":[1780,1520]}
  },
  "prediction_error":{"decode_tps_pct":-5.8,"ttft_pct":4.3},
  "verdict":"PASS",
  "notes":["thermal throttling observed at t+512s; sustained figure reported separately"]
}
```

Receipts are the artefact that makes the product trustworthy: they are diffable, archivable, attachable to a bug report, and comparable across machines. They are also, quietly, the training data for the calibration model.

### 5.6 Record: the calibration loop

Each receipt updates a small, local calibration store keyed by `(arch_family, backend, model_shape_class, quant_class) → k_cal`. Locally this makes a user's *second* plan better than their first. Globally (opt-in, anonymised, content-free) it becomes the thing that makes our predictions better than anyone else's — and the thing a competitor cannot copy without the same user base.

We should be clear-eyed: this is a data moat, and it only compounds if the first hundred users get real value before the moat exists. Hence M0 ships an **advisor with no installer** (§14) to start collecting receipts at the lowest possible trust cost.

---

## 6. Competitive landscape and the gap

### 6.1 What exists

| Tool | Category | Platforms | What it automates | What it leaves to you |
|---|---|---|---|---|
| **Ollama** | Runner + registry | macOS, Linux, Windows | Download, sensible defaults, API | Backend choice, quant choice, flags, multi-GPU, serving features, benchmarking |
| **LM Studio** | GUI runner | macOS, Windows, Linux | Model browsing, chat GUI | Headless automation, reproducibility, cross-vendor reasoning |
| **llama.cpp** | Engine | Everything | Nothing — it's a toolbox | All of it: build, quant, offload, flags |
| **vLLM / SGLang / TRT-LLM** | Serving engines | NVIDIA (+AMD for vLLM) | High-throughput serving | Whether they support your hardware at all; engine builds; tuning |
| **MLX / mlx-lm** | Engine | Apple only | Fast Apple inference | Conversion, quant selection, is-it-faster-than-llama.cpp |
| **GPUStack** | GPU fleet manager | Linux, NVIDIA-heavy | Deploy models across a GPU pool | Nothing for single-box, exotic-arch, or prediction |
| **exo** | Distributed runner | Apple-centric | Model-parallel across machines | Everything else; speedups over single-node are unclear |
| **LocalAI** | Self-hosted API | Multiple | Model management, API compat | Multi-framework selection, tuning, verification |
| **Docker/Podman model runners** | Packaging | Container hosts | Distribution | Hardware fit, quant choice, tuning |
| **NVIDIA NIM** | Vendor containers | NVIDIA | Optimised, supported inference | Licence, telemetry, offline, non-NVIDIA |
| **Benchmark tools** (llama-bench, `vllm bench`, GenAI-Perf) | Measurement | Per-engine | Measure one engine | Any decision; any comparison across engines |

### 6.2 The five capabilities nobody provides

| Capability | Who does it today | Evidence |
|---|---|---|
| Cross-vendor hardware fingerprint with **written rationale** | Nobody | Each engine detects only its own backend; vendor tools detect only their own silicon |
| **Framework selection** (choosing *between* engines) | Nobody | Engines are structurally incapable — vLLM will never recommend llama.cpp |
| **Reproducible pinned install with rollback** across pip/conda/brew/container/cargo | Nobody | Installers are per-ecosystem; nothing spans them |
| **Closed-loop autotuning on the actual box, with a receipt** | Nobody end-to-end | Benchmark tools measure; tuners (vLLM heuristics, Model Optimizer) tune within one engine |
| **Fleet drift detection for inference configs** | Partially (GPUStack, org-internal) | No cross-platform, no hardware-capability awareness |

The gap is not "an easier installer". It is **the decision layer**. Every existing tool is structurally disqualified from recommending a competitor, and every engine knows only its own backend's capabilities. That is a durable structural gap, not a temporary feature gap.

### 6.3 The Ollama question, answered honestly

Ollama is excellent and will beat us for the majority of casual users. We should not pretend otherwise. Specifically, Ollama cannot:

- choose a backend (it *is* one, modulo its internal engine swaps),
- expose quant choice (one quant per registry entry), KV-cache quantisation, or layer offload,
- do tensor parallelism worth the name (it spreads layers only when the model doesn't fit),
- serve high concurrency (defaults to 1 parallel request; measured 20–29× lower throughput and 8–19× worse P95 latency than vLLM under load on an H100),
- pin a version (auto-updates have silently changed the Mac backend's engine),
- benchmark and report.

So: **for the user with one Mac and one model, Ollama is the correct answer and we should say so.** Our users are the ones for whom *the choice itself* is the problem — the new unified-memory buyer, the multi-platform household, the fleet operator, the performance engineer, and anyone whose box is unusual enough that "just works" doesn't.

Strategic consequence: **treat Ollama as a runtime we can plan around, not an enemy** (Tier-B plan target). Positioning as a complement — "Ollama for the quick answer, Q-Infer when the answer matters" — is both honest and lowers adoption friction.

### 6.4 Prior art and why it failed

The graveyard of "universal local inference managers" is instructive:

- **Scope creep into a UI.** Tools that added chat, RAG, agents and model management died under their own surface area and stopped tracking upstream engines.
- **Abstraction that erased control.** Wrappers that hid flags were abandoned by exactly the users who needed performance, because they couldn't fix anything.
- **Maintenance cost of the matrix.** Supporting N engines × M platforms means N×M integration surfaces moving weekly. Projects that couldn't automate that collapsed.
- **Ignoring the long tail.** Tools that only supported CUDA covered 80% of users and 100% of the easy cases, leaving the differentiated ones unserved.

**Our mitigations are structural, not aspirational:** a small, versioned *contract* per engine (not per engine-version) with automated contract tests (§13, R1); a spec format that makes integrations declarative; and tiering (§8) so the long tail can be supported honestly without being supported expensively.

---

## 7. Functional requirements

Priority: **P0** = v1.0 must ship or we have no product. **P1** = v1.x. **P2** = later/never.
Each requirement is testable; acceptance criteria (AC) are given only where the test is non-obvious.

### 7.1 Detect

| ID | Requirement | Pri |
|---|---|---|
| FR-1 | Produce a Capability Fingerprint (CFP) for macOS, Linux, Windows-on-ARM without root/admin. | P0 |
| FR-2 | Identify accelerator `arch_id` precisely (`sm_121`, `gfx1151`, `apple-m4-max`, `X1E80100`), not just vendor/family. | P0 |
| FR-3 | Probe ISA/feature support by **execution**, not by string matching (§5.1 layer b). | P0 |
| FR-4 | Measure achievable memory bandwidth (read/copy) and a quantised GEMV; store method + CI. | P0 |
| FR-5 | Detect container/cgroup/VM constraints (memory limit, CPU quota, no GPU passthrough). | P0 |
| FR-6 | Detect *runtime gaps*: known-missing kernels, driver/toolkit skew, arch not in wheel. | P0 |
| FR-7 | Detect power/thermal mode and chassis class; record it as part of the fingerprint. | P1 |
| FR-8 | Emit CFP as stable, versioned JSON with per-field provenance and confidence. | P0 |
| FR-9 | Re-fingerprint cheaply (`--fast`, <2 s) for drift checks, using cached microbenchmarks. | P1 |

**AC (FR-3):** on a GB10 host, the fingerprint must report `arch_id: sm_121` **and** at least one `runtime_gap` entry if the installed torch/vLLM lacks kernels for it — derived from an actual kernel launch, not from a version string.

### 7.2 Catalog

| ID | Requirement | Pri |
|---|---|---|
| FR-10 | Maintain a signed catalog of model artifacts: format, quant, publisher, sha256, licence, gating, bytes-per-token metadata. | P0 |
| FR-11 | Include architectural metadata needed by the predictor: total/active params, layers, KV heads, head dim, vocab, max context. | P0 |
| FR-12 | Record a **quality delta** per quant relative to bf16 (perplexity/KL, with method and sample size). | P0 |
| FR-13 | Support user-supplied/local artifacts and local GGUF conversion, not only catalog entries. | P0 |
| FR-14 | Reflect gated model licences (e.g. Llama) and never circumvent acceptance flows. | P0 |
| FR-15 | Recommend artifact→backend pairs; never offer an artifact with no viable consumer on this machine. | P0 |
| FR-16 | Community-contributed catalog entries with provenance and a trust level. | P2 |

### 7.3 Plan

| ID | Requirement | Pri |
|---|---|---|
| FR-17 | Generate ranked plans for a given (model, objective, context, concurrency, quality floor). | P0 |
| FR-18 | Predict TTFT, decode tok/s, peak memory, and sustained (post-thermal) decode, with intervals. | P0 |
| FR-19 | Reject plans that violate hard constraints — fit, arch support, licence, probe failure — with a stated reason. | P0 |
| FR-20 | Emit mandatory `rationale` and `risks` per plan, generated from the fingerprint and catalog. | P0 |
| FR-21 | Pre-compute ≥2 fallback plans per primary plan. | P0 |
| FR-22 | Be runnable fully offline against a stale catalog, with an explicit staleness warning. | P0 |
| FR-23 | Support `--explain` that prints the full derivation, including the arithmetic. | P0 |
| FR-24 | Estimate cost per million tokens (local amortised vs a user-supplied cloud price). | P1 |
| FR-25 | Suggest the *cheapest hardware change* that would unlock a materially better plan ("+32 GB unifies the KV cache; +bandwidth would double decode"). | P2 |

### 7.4 Apply

| ID | Requirement | Pri |
|---|---|---|
| FR-26 | Install from a plan with every artifact content-addressed; refuse unpinned references. | P0 |
| FR-27 | Isolate into a per-plan prefix or container; touch nothing outside it without an explicit logged step. | P0 |
| FR-28 | Be transactional: any failure leaves the previous working state intact (AC: kill -9 mid-install → next command reports healthy or offers rollback). | P0 |
| FR-29 | Enumerate privileged steps; `--print-sudo` emits them for manual execution; never execute implicitly. | P0 |
| FR-30 | Verify checksums/signatures before execution; refuse on mismatch with an actionable error. | P0 |
| FR-31 | Export/import an offline bundle (manifest + artifacts + container images) for air-gapped hosts. | P1 |
| FR-32 | Support NVIDIA container runtime, ROCm, Metal, and bare-process installs as first-class modes. | P0 |
| FR-33 | Never redistribute vendor SDKs; guide the user through vendor acquisition with licence acceptance. | P0 |

### 7.5 Verify & record

| ID | Requirement | Pri |
|---|---|---|
| FR-34 | Run the fixed verification protocol (§5.5) and emit a signed receipt. | P0 |
| FR-35 | Compare measured vs predicted; record the error; update local calibration. | P0 |
| FR-36 | Detect and report thermal/sustained throttling separately from burst performance. | P1 |
| FR-37 | Detect the "fast but wrong" class: deterministic-output correctness check per plan. | P0 |
| FR-38 | Replay a receipt to re-derive the exact environment (`qinfer replay`). | P1 |
| FR-39 | Diff two receipts/machines and explain the delta in terms of fingerprint differences. | P1 |
| FR-40 | Export receipt as JSON and a human-readable Markdown/HTML report. | P1 |

### 7.6 Serve

| ID | Requirement | Pri |
|---|---|---|
| FR-41 | Supervise the runtime (restart on crash, bounded backoff) as a user service (launchd/systemd/Windows service). | P0 |
| FR-42 | Expose an OpenAI-compatible endpoint and proxy it so switching backends doesn't change the client. | P0 |
| FR-43 | Expose health, readiness, and Prometheus-style metrics (tok/s, TTFT, queue depth, memory). | P1 |
| FR-44 | Support multiple named plans installed side by side with a single active endpoint. | P1 |
| FR-45 | Refuse to start a plan that would exceed available memory; explain which component doesn't fit. | P0 |

### 7.7 Fleet (v1.x)

| ID | Requirement | Pri |
|---|---|---|
| FR-46 | Export a plan+receipt bundle; import and enforce it on another machine. | P1 |
| FR-47 | Detect drift: same plan, different measured performance or different fingerprint. | P1 |
| FR-48 | Open a PR against a repo containing `qinfer.yaml` when a newer, materially better plan exists (CI integration). | P2 |
| FR-49 | Centrally pin a catalog version across a fleet. | P2 |

### 7.8 Surfaces

| ID | Requirement | Pri |
|---|---|---|
| FR-50 | `qinfer up` with zero arguments works end-to-end on Tier-A machines. | P0 |
| FR-51 | Interactive wizard (M2): detect → choose objective → review plan → confirm → live verify. | P0 |
| FR-52 | Declarative `qinfer.yaml` for CI and reproducibility, round-tripping to/from plans. | P0 |
| FR-53 | Non-interactive mode with stable exit codes (§10.5) and JSON output for all commands. | P0 |
| FR-54 | Actionable errors: every failure names the cause, the fix, and the doc link. | P0 |
| FR-55 | No subcommand opens a **read-only dashboard** over the same state files the CLI writes — same process, no socket, no daemon. | P0 |
| FR-56 | Dashboard panels: machine identity · measured bandwidth (single-thread vs saturated) · predictor calibration (`t₀`, `k_cal`, MAPE) · per-model predicted-vs-measured · plans · receipts. | P0 |
| FR-57 | Every fact shown in the dashboard is obtainable headlessly (`--json`). The TUI is never the only path to a value. | P0 |
| FR-58 | `--snapshot WxH [--tab name]` renders one frame to an offscreen buffer and prints it, for CI and review. | P1 |
| FR-59 | Usable at 80×24; below 76 columns panels stack and shed detail in a defined order rather than clipping mid-word. Nothing destructive is bound to a bare key. | P0 |

---

## 8. Platform support: what we promise, and what we refuse to promise

### 8.1 Tiers

| Tier | Meaning | What the user gets | CI gate |
|---|---|---|---|
| **A — Certified** | We own or rent the hardware; a human has run it | Install + tune + verify, guaranteed | Full install→bench on every release |
| **B — Supported** | Install path exercised; tuning is heuristic only | Install + predict; expect to tune | Install smoke in CI (VM/emulation/community runner) |
| **C — Detected** | We can identify it and advise | Fingerprint, recommendation, rationale | Fingerprint unit tests only |
| **D — Known-unsupported** | We can identify it and explain why not | A clear, kind refusal + what would work | Test asserts the refusal |

Tiering is a marketing asset, not an apology. A tool that claims everything and delivers nothing on half of it is worse than one that says *"Snapdragon X Elite: we support three model families on the NPU, and here is exactly why the rest will run on CPU."*

**Certification status today (v0.2).** Only **Apple Silicon is genuinely Tier A**, because it is the only hardware the team possesses — the M0 spike ran on an M5 Pro (§21). Every other "A" below is a *target*, contingent on acquiring the hardware before M1. We do not certify what we have not run; that rule is the only reason this section is worth reading.

### 8.2 Matrix

| Platform | Accelerator path | Tier | Realistic verdict |
|---|---|---|---|
| Apple M1–M4 (all tiers), macOS 14+ | Metal (llama.cpp), MLX | **A** | Excellent for single-user. Decode is bandwidth-limited; MLX generally leads on sustained decode, llama.cpp leads on flag surface and model breadth. |
| Apple ANE | CoreML | **D** (LLMs) | Not an LLM device. Static shapes, FP16-internal, op gaps, monolithic-INT4 fallback above ~1.4 GB. We say so and stop. |
| DGX Spark / GB10 | CUDA `sm_121` | **B → A (target)** | The flagship, and the machine this product is most obviously *for*. Native FP4, 128 GB unified, 273 GB/s. But `sm_12x` has no `tcgen05`/TMEM, so datacenter-Blackwell FP4 kernels do not port — NVFP4 on Spark is a separate implementation, not the same kernel recompiled. |
| NVIDIA x86 (Ada `sm_89`, Blackwell `sm_120`) | CUDA | **A** | Best-supported ecosystem in existence. VRAM is the wall. |
| NVIDIA Hopper `sm_90` | CUDA, FP8 | **A** | vLLM/SGLang/TRT-LLM all first-class. |
| Jetson AGX Thor / Orin | CUDA, JetPack | **B** | Same bandwidth class as Spark in a different power/thermal envelope. JetPack version pinning is the main hazard. |
| AMD Strix Halo `gfx1151` | ROCm, Vulkan | **B** | Terrific capacity-per-dollar (128 GB unified). Backend choice is *model-specific* — Vulkan and ROCm each win sometimes. GTT carveout is a boot-time decision we can only detect, not fix. |
| AMD RDNA3/4 dGPU | ROCm, Vulkan | **B** | Works; ROCm wheel/version discipline is the tax. |
| Snapdragon X / X2 Elite (Windows-ARM64) | Hexagon NPU (QNN bundles), Adreno (OpenCL), CPU | **B** (catalog models) / **C** (arbitrary GGUFs) | Real but narrow, and *slower than the CPU for most useful models*: on one X Elite a 7B Q4_0 on CPU reached **24.8 tok/s** while an 8B w4a16 on the NPU reached **9.96 tok/s**. `Q4_0` is the only clean GGUF-on-NPU quant; bundles bake context/KV/precision; the NPU driver path may require `TESTSIGNING` (Secure Boot implications), so it can never be a silent default. |
| Snapdragon 8 Elite (Android) | Hexagon | **C** | Out of scope for v1 beyond detection. |
| ARM server (Graviton 4, Axion, AmpereOne) | CPU (NEON, dotprod, i8mm, SVE2, SME) | **B** | Viable for small models at moderate concurrency. KleidiAI matters; repack costs ~2× RAM. |
| Intel (Lunar Lake / Arc / Xeon) | OpenVINO, SYCL, oneAPI | **B/C** | Large surface, low demand among our personas. Detection + basic CPU path first. |
| Huawei Ascend (CANN), Moore Threads (MUSA) | Vendor backends | **C** | Detect, explain, do not automate. |
| WSL2 / containers | Passthrough | **B** | Detect `/dev/dxg`-style passthrough and cgroup caps; predictions must subtract host overhead. |

### 8.3 What we will not claim

- That an NPU makes a 70B fast. It does not; the memory ceiling and op coverage decide, and both are hostile.
- That Thunderbolt-clustered Macs beat one bigger Mac on decode. TB5 moves ~12 GB/s against 800 GB/s of local bandwidth; cross-node links buy **capacity**, not **speed**.
- That "4-bit" is a single thing. It is at least six incompatible artefacts (§18).
- That our predicted number is exact. It is a ceiling with an error bar, and the receipt publishes the error.
- That any given vLLM release supports any given new accelerator on day one. It demonstrably does not (§2.2-ii).

### 8.4 The honest NPU paragraph

Users will ask why their 45-TOPS NPU is slower than their CPU. The answer must be built into the UI, not buried in docs:

> An NPU is optimised for small, static, quantised graphs. An LLM decode loop is a large, stateful, memory-bound workload with growing KV state. NPU TOPS describes matrix throughput you cannot fully use, while decode is limited by how fast you can stream weights from DRAM. On Snapdragon-class silicon the NPU becomes the right choice below roughly 4B parameters with a pre-compiled, per-chipset bundle — and the CPU/GPU path is usually the right choice above that.

---

## 9. Framework support matrix

| Framework | Backends it serves | Install reality | Strength | Weakness | Our role |
|---|---|---|---|---|---|
| **llama.cpp** | CUDA, Metal, HIP, Vulkan, SYCL, OpenCL, Hexagon, CANN, CPU | Build or prebuilt; many knobs | Universal; first to new formats; GGUF ecosystem | Zero automation; large flag surface; slower than MLX on Apple | **Universal baseline** — always a candidate |
| **MLX / mlx-lm** | Apple Metal | pip, arm64-only | Fastest sustained decode on Apple; 4/6/8-bit + DWQ | Apple-only; thinner sampler/tooling | **Apple fast path** |
| **vLLM** | CUDA (`sm_80`–`sm_121` via the `12.0` family target), ROCm | pip/container; arch-gated | PagedAttention, continuous batching, FP8/NVFP4 | No Apple support; pre-allocates ~90% VRAM; per-extension cubin gaps persist | **NVIDIA high-throughput** |
| **SGLang** | CUDA | pip/container | RadixAttention, strong at prefix-heavy workloads | Narrower hardware support | NVIDIA alternative where prefix reuse dominates |
| **TensorRT-LLM** | NVIDIA | Container, engine build step | Best raw NVIDIA perf with FP4/FP8 | Engine compilation cost; model-support lag | NVIDIA peak-perf option when a model is supported |
| **Ollama** | llama.cpp/MLX internally | One binary | Ergonomics, model registry | No quant/flag control, no TP, no pinning | **Plan target**, not competitor |
| **ONNX Runtime (+ GenAI)** | CUDA, TensorRT, QNN, DirectML, OpenVINO, CPU | pip/native | Broad EP coverage; the only real Windows-NPU path | GenAI layer less mature for LLMs | **NPU/Windows path** |
| **Qualcomm Genie / QAIRT** | Hexagon | Vendor SDK + per-chipset bundles | Only true NPU path on Snapdragon | Bundle baked (precision/ctx/KV fixed); vendor licence | **Snapdragon NPU path** |
| **OpenVINO** | Intel CPU/GPU/NPU | pip | Best Intel path | Intel-only | Intel detection + basic path |
| **ExecuTorch** | Many (mobile) | pip/build | Deployable to mobile/edge | Early for large LLMs | **Watch** — revisit at M4 |
| **MLC-LLM** | CUDA, Metal, Vulkan, mobile | Build-heavy | Compiler-based, portable | Build complexity | **Watch** |
| **ExLlamaV2/V3** | CUDA | pip | Very fast EXL2 quants on consumer GPUs | NVIDIA-only, small ecosystem | NVIDIA efficiency alternate |
| **KTransformers** | CUDA + CPU offload | Build | Large MoE on limited VRAM | Very manual | **Watch** — MoE-on-small-VRAM case |

**Engine contract.** Every integration implements one versioned trait, so adding an engine is a data problem rather than a codebase problem:

```
capabilities()      -> which (backend, arch_id, quant_format) it can execute
plan(ctx)           -> flags for (model, ctx, concurrency, objective) + predicted k_cal hint
install(plan)       -> pinned, transactional, isolated
verify(plan)        -> adapter that produces standard metrics
serve(plan)         -> supervised process + endpoint
health()            -> liveness/readiness + drift signals
```

Contract tests run against every pinned engine version in CI. When an upstream breaks the contract, the *catalog* is updated to exclude the version — not the user's working install.

---

## 10. User experience

### 10.1 The golden path

```
$ qinfer up

  Q-Infer 1.0 · bare-metal inference provisioning

  Detected:  NVIDIA DGX Spark (GB10) · 128 GB unified · aarch64
             measured bandwidth 251 GB/s (declared 273) · CUDA 13.0 · sm_121
             ⚠ 1 runtime gap: published vLLM wheels lack sm_121 kernels for qutlass

  What do you want?
    ▸ 1  Best overall (balanced)            ← recommended
      2  Fastest generation speed
      3  Largest model that fits
      4  Longest context
      5  I know exactly what I want (spec file)

  Model: qwen3-32b-instruct  (32.8B dense, 128k ctx, Apache-2.0)

  Plan pln_7f3a91 ─────────────────────────────────────────────────
    runtime     vLLM 0.29.0 (container, digest pinned)
    artifact    Nemotron-3-Super-NVFP4  ·  19.2 GB  ·  4-bit NVFP4
    context     32 768      concurrency 4
    predicted   decode 31.2 tok/s (27.0–35.5) · TTFT 420 ms · 96.4 GB peak
    why         NVFP4 halves bytes/token vs FP8 and decode is bandwidth-bound;
                gpu_memory_utilization lowered to 0.72 because memory is unified
    risk        qutlass kernels absent on sm_121 → MXFP4 transform path disabled
    fallbacks   [pln_7f3a92 llama.cpp CUDA · pln_7f3a93 vLLM FP8]

  Proceed? [Y/n/details]
```

Then, without further input:

```
  ▸ resolving artifacts …            4 pinned downloads, 19.4 GB
  ▸ provisioning …                   isolated prefix ~/.qinfer/envs/pln_7f3a91
  ▸ smoke …                          model loaded, 1 token, endpoint OK
  ▸ correctness …                    deterministic output matches expected
  ▸ benchmark …                      pp512 812 tok/s · tg128 29.4 tok/s · p95 31.1
  ▸ stability …                      10 min sustained · ⚠ throttled at t+512s (26.8 tok/s)

  ✓ Serving on http://localhost:8080/v1   (OpenAI-compatible)

  Receipt rcpt_9c21  ·  prediction error −5.8% decode, +4.3% TTFT  ·  PASS
  Copy-paste:
     curl http://localhost:8080/v1/chat/completions -H 'Content-Type: application/json' \
       -d '{"model":"qwen3-32b","messages":[{"role":"user","content":"hello"}]}'
```

Note what the output is doing: it is **justifying itself in the same breath as acting**. The `why` and `risk` lines are the product. Anyone can print a progress bar.

### 10.2 Command surface

| Command | Purpose |
|---|---|
| `qinfer detect [--fast] [--json]` | Fingerprint only. Never installs. Safe on any machine. |
| `qinfer plan <model> [--objective …] [--ctx N] [--concurrency N] [--json]` | Ranked plans + rationale. Dry run. |
| `qinfer explain <plan_id>` | Full derivation with arithmetic and catalog sources. |
| `qinfer up [<plan_id>] [--yes] [--detach]` | Apply + verify + serve. |
| `qinfer bench <plan_id> [--protocol default\|long\|concurrent]` | Re-run verification, emit receipt. |
| `qinfer ls` / `qinfer status` / `qinfer down` / `qinfer rm` | Lifecycle. |
| `qinfer use <plan_id>` | Flip the active endpoint to an installed plan. |
| `qinfer receipts export\|diff\|replay` | Reproducibility surface. |
| `qinfer bundle export\|import` | Air-gap transfer. |
| `qinfer catalog update\|pin\|verify` | Catalog lifecycle (signed). |
| `qinfer doctor` | Diagnose an existing install against its receipt (drift). |

### 10.3 Declarative spec

```yaml
# qinfer.yaml — commit this
schema: qinfer/v1
machine:
  match: { arch_id: sm_121, min_memory_gb: 128 }   # advisory; warns on mismatch
model: { name: qwen3-32b-instruct, ctx: 32768, concurrency: 4 }
objective: balanced
quality_floor: "perplexity_delta <= 0.02"
pin:
  catalog: 2026.10.1
  vllm: "0.29.0"
  artifact: sha256:9f2c…
verify: { protocol: default, require: [correctness, stability] }
serve: { port: 8080, service: user }
```

`qinfer up -f qinfer.yaml` in CI becomes a reproducibility test. `qinfer plan -f qinfer.yaml --check` exits non-zero if the pinned plan is no longer optimal by more than a threshold — that is the drift signal a fleet operator actually wants.

### 10.4 Dashboard (TUI)

`qinfer` with no subcommand opens a dashboard. This is not the installer wizard (FR-51, M2) — it is the **standing view** of a machine: what it is, what we predicted, what it actually measured, and what is running. Configuration happens occasionally; observation is continuous, and today that means three terminals of `nvidia-smi`, a `powermon`, and a `tail -f`.

Built with **Ratatui** over Crossterm, in the same static binary (ADR-0009). It opens no socket and spawns no daemon — it reads the same JSON the CLI writes.

```
┌ Q-Infer  Apple M5 Pro · Tier A ──────────────────────────────────────────┐
│ Overview │ Bench │ Plans │ Help                                          │
├──────────────────────────────┬───────────────────────────────────────────┤
│ Machine                      │ Capabilities                              │
│  CPU    M5 Pro (15P/15L)     │  ISA      BF16 · DotProd · I8MM · SME2    │
│  GPU    M5 Pro 16c · Metal 4 │  Unified  yes                             │
│  Memory 24.0 GiB · swap 11.5 │  Threads  15                              │
├──────────────────────────────┴───────────────────────────────────────────┤
│ Memory bandwidth (measured, not declared)                                │
│  1 thread   ███████████████████████░░░░░░░░░░░░   131.4 GB/s             │
│  15 threads ████████████████████████████████████   228.0 GB/s  (1.73×)   │
├──────────────────────────────────────────────────────────────────────────┤
│ Decode throughput — predicted vs measured                                │
│  model                GiB   measured  predicted   Δ      k_cal           │
│  qwen2.5-3b-q4_k_m   1.95     108.8      110.5   +1.6%    1.00           │
│  …                                          MAPE 1.45% (naive 15.6%)      │
└──────────────────────────────────────────────────────────────────────────┘
```

Design rules, all of which are testable:

- **Three facts, unmissable**: what the machine is, what we predicted, what it measured. Anything that serves none of those is decoration and should be cut.
- **Read-mostly.** Observation by default; mutating actions require explicit confirmation. `q` quits; nothing destructive is bound to a bare key.
- **Never the only path.** Every panel has a `--json` equivalent (FR-57), so the dashboard stays usable over SSH, in CI, and for anyone who does not want a full-screen app.
- **Honest about noise.** The bench panel shows spread and cache state; it must not present a single point estimate as stable.
- **Degrades by design.** At 80×24 it is complete. Below 76 columns panels stack and shed corroborating columns, rather than clipping mid-word (FR-59).
- **Headless rendering.** `--snapshot 100x30 --tab bench` renders one frame to an offscreen buffer, so layout regressions are caught in CI and reviews do not need a human at a TTY.
- **Colour-free and screen-reader-safe** mode (`--no-color`, `--plain`) is P0. A terminal tool that is unreadable over SSH in a datacenter is a tool for laptops only.

### 10.5 Exit codes and errors

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Unsupported platform (detected, Tier C/D) |
| 3 | Model does not fit under any viable plan |
| 4 | Architecture/kernel incompatibility (e.g. no cubins for `sm_121`) |
| 5 | Verification failed (correctness or performance below floor) |
| 6 | Provisioning failure (network, checksum, licence) |
| 7 | User declined |
| 8 | Drift detected (`--check` found a materially better plan) |

Every error answers three questions in this order: **what happened** (fact), **why** (cause), **what to do** (action), plus a stable doc anchor. Example:

```
✗ Cannot plan: model needs 41.2 GB of KV cache at 32k context; 27.9 GB available.
  Cause:  32k context × 40 layers × 8 KV heads × 128 dim × 2 (K+V) × 2 B = 41.2 GB
          after 68.4 GB of weights on a 128 GB unified pool shared with the OS.
  Fix:    qinfer plan qwen3-32b --ctx 8192 --kv-cache-dtype fp8   (fits, ~24 tok/s)
          or add memory, or use a smaller quant:  qinfer plan qwen3-32b --quant awq-int4
  Docs:   https://…/errors/E-FIT-001
```

### 10.6 Air-gapped mode

`qinfer bundle export --plan pln_7f3a91 --out ./bundle` produces a directory containing the plan, every artifact, container image layers (via `skopeo`/`oras`-style export), checksums and a manifest. `qinfer bundle import` on the disconnected host verifies and applies. Vendor SDKs are *referenced*, not embedded — the operator supplies them under their own licence. This is the single feature that makes the product sellable to defence, healthcare and industrial edge, and it is a P1 that several of our design partners will need on day one.

---

## 11. Architecture

### 11.1 Components

```
┌──────────────────────────────────────────────────────────────────────────┐
│ SURFACES          CLI · TUI · qinfer.yaml · JSON API · CI action          │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │
┌───────────────────────────────▼──────────────────────────────────────────┐
│ CORE (single static binary, no Python import surface)                     │
│                                                                           │
│  ┌────────────┐   ┌───────────┐   ┌──────────┐   ┌───────────────────┐   │
│  │ FINGERPRINT│   │ CATALOG   │   │ PREDICTOR│   │ PLANNER (search + │   │
│  │ collectors │   │ signed DB │   │ roofline │   │ constraint solver)│   │
│  └─────┬──────┘   └─────┬─────┘   └────┬─────┘   └─────────┬─────────┘   │
│        │                │              │                   │             │
│  ┌─────▼────────────────▼──────────────▼───────────────────▼──────────┐  │
│  │ PROVISIONER   pin · fetch · verify · isolate · rollback · bundle   │  │
│  └─────────────────────────────┬──────────────────────────────────────┘  │
│  ┌─────────────────────────────▼──────────────────────────────────────┐  │
│  │ VERIFIER   smoke · correctness · perf · stability · receipt        │  │
│  └─────────────────────────────┬──────────────────────────────────────┘  │
│  ┌─────────────────────────────▼──────────────────────────────────────┐  │
│  │ SUPERVISOR  process lifecycle · health · metrics · proxy           │  │
│  └────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────┬──────────────────────────────────────────┘
                                │  spawns + supervises, never links
        ┌───────────────────────┼───────────────────────┬──────────────┐
        ▼                       ▼                       ▼              ▼
   vLLM/SGLang            llama.cpp/MLX          ONNX/QAIRT      containers
   (python procs)         (native binaries)      (SDK procs)     (OCI)
```

**Deliberate choices:**

- **The core does not import PyTorch, Transformers, or any ML library.** It orchestrates processes. This keeps the binary small (~15–30 MB), the install fast, and — critically — lets us support a runtime we cannot even import on the host (e.g. planning for a container on a machine with no CUDA toolchain).
- **Rust core** (or Go) for the binary: static linking, predictable cross-compilation, no interpreter prerequisite, good subprocess and signal handling. Python appears only where an engine requires it.
- **The catalog is data, signed and versioned**, so "new vLLM release breaks `sm_121`" is a catalog update, not a product release.
- **Every engine is a subprocess.** Uniform supervision, uniform metrics scraping, no ABI coupling to fast-moving upstreams.

### 11.2 State layout

```
~/.qinfer/
  fingerprint.json          # last CFP (+ .prev for diffing)
  calibration.json          # local k_cal table
  catalog/                  # signed, versioned
  plans/pln_*.json
  envs/pln_*/               # isolated install prefixes
  receipts/rcpt_*.json
  bundles/
  logs/
```

Everything is plain, versioned JSON/YAML with published schemas — a user must be able to `cat`, `diff`, `git commit`, and hand-edit their way out of any problem. No opaque database. This is a deliberate anti-lock-in stance and it costs us nothing.

### 11.3 Security model

| Threat | Control |
|---|---|
| Malicious artifact | Content hashes; optional sigstore/cosign verification; refuse on mismatch |
| Supply-chain substitution | Catalog is signed; pinned catalog version per plan; `catalog verify` |
| Privilege escalation | No implicit `sudo`; privileged steps enumerated and printed; never `curl \| sh` |
| Data exfiltration | No telemetry by default; if enabled, it is content-free (numbers and fingerprints), opt-in, and inspectable |
| Model/licence violation | Gated model flows respected; licence recorded in receipt; `--accept-licence` required |
| Prompt/data exfiltration via a runtime's phone-home | Document per-engine network behaviour; `--offline` blocks egress at the supervisor level |

### 11.4 Concurrency and failure model

Provisioning is a sequence of **idempotent, resumable steps** with a journal on disk. Any command can be interrupted (`SIGINT`, power loss, `kill -9`) and the next invocation either resumes or offers a clean rollback. The invariant: *a partially-applied plan never leaves the previously-working plan unusable.*

---

## 12. Non-functional requirements

| Area | Requirement | Target |
|---|---|---|
| Startup | `qinfer detect --fast` on a warm cache | < 2 s |
| Detection | Full fingerprint incl. microbenchmarks | < 20 s (P95) |
| Planning | 3 ranked plans with rationale | < 1 s (offline, catalog cached) |
| Time-to-serving | Tier-A, artifact ≤ 20 GB, warm network | **< 15 min** median, < 25 min P90 |
| Time-to-serving | Air-gapped, from local bundle | < 6 min |
| Verification | Default protocol | ≤ 12 min (perf + 10 min stability), skippable |
| Footprint | Core binary | < 35 MB; no runtime deps except the engines |
| Offline | Full plan/apply against a bundle | Must work with zero network |
| Determinism | Same fingerprint + same pinned plan ⇒ same artifacts | Byte-identical manifests |
| Reliability | Rollback after failure | 100% in tests (fault injection at every step) |
| Portability | macOS (arm64), Linux (x86_64, aarch64), Windows (arm64, x86_64) | P0; others best-effort |
| Observability | Structured logs + metrics for every supervised runtime | P0 |
| Privacy | No data leaves the machine unless explicitly enabled | Default off |
| Accessibility | Plain/colour-free output, no interactive-only paths | P0 |

---

## 13. Success metrics

### 13.1 North star

**Verified serving configurations per week** — the count of receipts with `verdict: PASS` produced by distinct machines. It captures acquisition, activation, and the compounding calibration loop in one number, and it is not gameable by downloads.

### 13.2 Supporting metrics

| Metric | Target at GA+90d | Why |
|---|---|---|
| First-attempt plan success (Tier A) | ≥ 90% | The core promise |
| Median time-to-serving (Tier A) | < 15 min | The other core promise |
| Decode prediction MAPE — dense, post-calibration | ≤ 25% | Calibration quality |
| Decode prediction MAPE — MoE | ≤ 40% | `t₀` and routing variance are real |
| TTFT prediction MAPE — post-calibration | ≤ 25% | MFU is measurable once you've measured it |
| Fit prediction accuracy | **100%** | Arithmetic must never be wrong |
| Plan rank-order accuracy | ≥ 85% (Kendall τ ≥ 0.7) | The promise that actually matters (§5.2.4) |
| Plan acceptance rate (user chooses our #1) | ≥ 65% | Are the rationales convincing? |
| Receipt export/share rate | ≥ 15% | Trust signal and flywheel input |
| `--offline` successful applies | ≥ 95% | Air-gap readiness |
| Support issues per 100 installs | < 0.5 | Maintenance burden is the real risk |
| % of plans that are "fit-only" | < 10% | Anti-gaming: we must not succeed by recommending 4B models |

### 13.3 Counter-metrics (things we must not optimise for)

- **Downloads/installs** — vanity; Ollama wins that forever.
- **Tok/s record-chasing** — encourages fragile, unrepresentative configs.
- **Hardware matrix width** — every Tier-A platform is a permanent CI + support cost. Growth in tiers without growth in TAM is failure.
- **Model support count** — the catalog should be *curated*, not exhaustive. A model that runs badly is worse than a model we don't list.

---

## 14. Risks

| # | Risk | L/I | Mitigation | Early warning |
|---|---|---|---|---|
| R1 | **Upstream velocity** — vLLM weekly, llama.cpp daily, breaking flags and formats | H/H | Engine contract + contract tests; catalog-mediated version gating; pin everything; never chase `latest` | Contract test failure rate rises above ~5%/release |
| R2 | **Hardware we cannot test** — claims become lies | H/H | Tier discipline; refuse Tier-A without hardware; recruit design partners for hardware access; community receipts as evidence only | A Tier-A install fails in the wild |
| R3 | **Vendor SDK licences** — QNN/QAIRT/CoreML redistribution | M/H | Never redistribute; user-acquired; document per-vendor terms; legal review before Tier-B NPU GA | A takedown notice or a blocked enterprise sale |
| R4 | **Ollama's gravity** — "good enough" wins | H/M | Complement, don't fight; be a runtime target; target the segment Ollama structurally abandons (exotic arches, serving, fleets) | Users asking "why not just use Ollama?" in demos |
| R5 | **Prediction is worse than promised** | M/H | Publish error bars; benchmark-before-you-promise; narrow the claim to *ranking* if point accuracy fails | MAPE > 40% on Tier-A hardware |
| R6 | **Maintenance collapse** — matrix grows faster than team | H/H | Tier gating; declarative integrations; refuse platforms; budget one FTE per Tier-A family | Issue backlog grows faster than releases |
| R7 | **NPU/first-party API churn** — Windows ML, Apple frameworks move | M/M | Treat NPU as Tier B/C; keep CoreML and QNN behind adapters | Two consecutive quarters of API breakage |
| R8 | **Security incident via a mirrored artifact** | L/Catastrophic | Hashes + signature verification; no mirrors we don't control; SBOM per receipt | Any checksum mismatch |
| R9 | **Scope creep into UI/agents** | M/H | Non-goals enforced in review; the "no kernels, no chat" boundary is a hard architectural rule | Roadmap items mentioning "chat" |
| R10 | **Unified-memory fit surprises** — OS/runtime competition | M/M | Leave explicit headroom in plans; refuse to over-commit; include OS in the fit equation | OOM kills in otherwise "qualified" plans |

**The meta-risk** is R1×R6: a matrix that grows while upstream churns means the team spends all its time re-validating and none of it improving the predictor. The mitigation is not heroics — it is *ruthless tiering* and making the catalog the thing that absorbs churn.

---

## 15. Release plan

| Milestone | Scope | Duration | Exit criteria |
|---|---|---|---|
| **M0 — Advisor** | Fingerprint + catalog + predictor + `plan`. **No installer at all.** | 4–5 wk | Runs on 4 platform families; a user can get a defensible answer with zero risk; 50+ receipts collected |
| **M1 — Apply** | Provisioner + verifier + receipts on Apple, NVIDIA x86, DGX Spark | 6–8 wk | Rollback proven under fault injection; ≥80% first-attempt success on Tier A |
| **M2 — Tune & Serve** | Closed-loop autotune, supervisor, OpenAI proxy, TUI | 5–6 wk | Predicted-vs-measured MAPE ≤ 25%; serving survives 24 h soak |
| **M3 — Fleet & Air-gap** | Bundles, drift, `qinfer.yaml` in CI, GitHub Action | 5–6 wk | One air-gapped reference deployment; drift detection catches an injected regression |
| **M4 — Widen** | Promote Snapdragon / Strix Halo / Jetson to Tier A as receipts justify | ongoing | One FTE per promoted family, funded |

**Why M0 first.** It is the highest-value, lowest-trust-cost artefact we can ship: no root, no installs, no risk — and it starts the calibration flywheel while the user base is small. It also forces us to get the *prediction* right before we make it load-bearing, which is the correct order of operations for a product whose credibility rests on being right about hardware.

---

## 16. Decisions and open questions

**v0.2 status:** Q1–Q10 now carry **provisional decisions** — enough to unblock the SDD. Each is reversible, but the "expensive after" column says when reversing stops being cheap. Anything not listed here is genuinely open.

| # | Question | Provisional decision (v0.2) | Expensive to change after | Rationale |
|---|---|---|---|---|
| Q1 | Single binary or Python CLI? | **Single static binary**; Python only where an engine demands it | M1 | Python-first caps the audience at people with a working Python env — the people who least need us |
| Q2 | Does Windows-ARM ship at GA? | **Detect-only at M0; install at M4** | M2 | A broken Windows story is worse than a late one |
| Q3 | Redistribute quantised weights? | **Never** — point at publishers only | never | Hosting invites licence and moderation liability |
| Q4 | Telemetry? | **Opt-in, content-free, inspectable** (`qinfer telemetry show`) | M3 | Without it the global calibration moat never forms; with clumsy telemetry we lose the enterprise |
| Q5 | Orchestrate, or integrate? | **Integrate** — emit plans GPUStack/llm-d/KServe can consume | M3 | Building orchestration is a different company |
| Q6 | Licence | **Apache-2.0 core + paid fleet/air-gap tier** | GA | The flywheel needs contribution; the business needs fleet features |
| Q7 | Pricing | **Per host / year**, unlimited models | GA | Per-token or per-seat misreads a fleet buyer |
| Q8 | Who measures quant quality? | **We do**, on a fixed eval suite | M2 | The only way a "quality floor" is a real constraint |
| Q9 | Default objective | **`balanced`**, with the quality floor explained on first run | M2 | A wrong default makes every first impression mediocre |
| Q10 | Private/fine-tuned weights at GA? | **Yes** — local artifact path (FR-13) | M1 | Enterprise buyers ask in the first meeting |

### 16.1 Newly opened (v0.2)

| # | Question | Why it is now open | Owner / by when |
|---|---|---|---|
| Q11 | Do we probe bandwidth on-device (Metal/CUDA) or keep the CPU probe plus `k_cal`? | The spike measured `k_cal` = 1.175 — the CPU probe understates GPU-achievable bandwidth, so the coefficient is carrying real signal (§21) | M1 |
| Q12 | How do we certify `sm_121` without owning a Spark? | Tier A requires hardware we have run; desk research cannot certify (§8.1) | Before M1 exit |
| Q13 | Does the planner expose NPU plans at all, given CPU wins above ~4B? | Dossier 03 shows CPU beating NPU on a larger model; a bad NPU plan is worse than none | M2 |
| Q14 | Do we ship a `--secure-boot-required` refusal for Windows NPU? | The Hexagon driver path may need `TESTSIGNING`, which is a boot-security change | M4 |

---

## 17. Appendix A — hardware reference and the roofline in practice

### 17.1 The numbers we plan against

| Machine | Mem / type | BW declared | BW measured (typical) | Notes |
|---|---|---|---|---|
| M1 / M1 Pro / Max / Ultra | 16–128 GB unified | 68 / 200 / 400 / 800 | ≈ 0.85–0.95 × declared | baseline of the family |
| M2 family | 24–192 GB | 100 / 200 / 400 / 800 | — | |
| M3 family | 24–512 GB | 100 / 150 / 300–400 / 800 | — | Pro tier varies with GPU bin |
| M4 family | 32–512 GB | 120 / 273 / 546 / 800 | — | Max 546 GB/s is the sweet spot |
| M5 family `UNVERIFIED` | 32–512 GB | 153 / 307 / 614 / ~1230 | — | confirm before shipping |
| DGX Spark (GB10) | 128 GB LPDDR5X unified | 273 | ~250–260 | 6144 CUDA cores, 48 SM, `sm_121` |
| Jetson AGX Thor T5000 | 128 GB LPDDR5X unified | 273 | — | 2560-core Blackwell, 40–130 W |
| Strix Halo (Ryzen AI Max+ 395) | 128 GB LPDDR5X unified | 256 | ~215–239 | `gfx1151`; GTT carveout is boot-time |
| Snapdragon X Elite | up to 64 GB LPDDR5X | ~135 | — | Hexagon NPU; bundle-gated |
| Graviton 4 / Axion | DDR5, many channels | 300–500 `UNVERIFIED` | — | CPU-only; k_cal well below 1 |
| RTX 4090 / 5090 | 24 / 32 GB GDDR | 1008 / 1792 | — | VRAM is the wall |
| H100 SXM | 80 GB HBM3 | 3350 | — | reference point, not a target |

### 17.2 Measured decode, side by side

Published single-stream numbers, 4-bit unless noted. This is the seed corpus for the calibration store — not a marketing table.

| Machine | ~8B dense | 30–32B dense | 30B-A3B MoE | Large (70B+/120B) |
|---|---|---|---|---|
| M1 (68 GB/s spec) | 14.2 *(7B Q4_0)* | ~2 | — | <1 |
| M4 Pro (273) | 50.7 *(7B Q4_0)* | ~5 | — | ~1.5 |
| M4 Max (546) | 83.9 *(Qwen3-8B Q4_K_M)* | ~10 | — | ~4 |
| M5 Max (614) | 119.9 *(7B Q4_0)* | ~14 | — | ~5 |
| M3 Ultra (819) | **95.8** *(Qwen3-8B Q4_K_M)* | **34.0** *(Qwen3-32B)* | **95** | ~10 |
| DGX Spark GB10 (273) | ~45 | ~13 | **80.9** *(Qwen3-Coder-30B-A3B)* | **6.0** *(Llama-3.3-70B)* · 60.7 *(gpt-oss-120b MXFP4)* |
| Jetson Thor (273) | **41.3** *(Llama-3.1-8B NVFP4)* | 13.2 | 61 | — |
| Strix Halo (256 spec) | ~40 | 11.0 *(Qwen2.5-Coder-32B)* | **69.6** *(Qwen3-Coder-30B)* | 3.8 *(70B Q6_K)* |

Four things jump out, and each one is a marketing claim we can quietly correct for a user:

- **The Ultra plateau is real.** An M3 Ultra has ~1.5× the spec bandwidth of an M4 Max and delivers ~1.14× the decode on the same 8B model — because effective decode bandwidth saturates near 350–460 GB/s. A naive roofline would have predicted 2.3× and been wrong in the most expensive direction.
- **A DGX Spark is a capacity machine, not a speed machine.** 6 tok/s on a 70B is genuinely useful — it is 70B-class output on a desk — but it is not fast, and no amount of tuning will make it fast. What makes the Spark sing is a 30B-class MoE at ~80 tok/s.
- **MoE wins, but by 3×, not 10×.** Same M3 Ultra: 95 tok/s (30B-A3B) versus 34 tok/s (dense 32B). The parameter ratio is 10×; the delivered ratio is 2.8×.
- **Spill is catastrophic and silent.** A 235B-A22B on a single Spark lands at 1.8 tok/s — an 8% efficiency that no bandwidth model explains. The product must detect this state and say "you are paging, not computing", rather than report a number.

Read this table before reading any benchmark headline, including the ones we will publish. It is also the reason the *first* thing our predictor does on a new box is measure bandwidth rather than look up a spec sheet.

### 17.3 Fingerprint field inventory (full)

`identity` (os, distro, kernel, arch, virt, form_factor, chassis_power_w) · `cpu` (model, cores, perf/efficiency split, isa[], freq governor) · `accelerators[]` (vendor, arch_id, compute_cap, cu_count, unified, memory{total,dedicated,shared}, bandwidth{declared,measured,ci95,method}, features{fp4,fp8,bf16,int8_dot,i8mm,sve2,sme2,ane,htp,mig}, runtime{driver,cuda,rocm,metal_family,qnn,oneapi}, arch_supported[]) · `memory` (total, available, swap, cgroup_limit) · `storage` (class, free_bytes) · `network` (loopback only by default) · `runtime_gaps[]` (component, issue, consequence, source) · `microbench` (bw_read, bw_copy, gemv_q4, gemm_bf16, npu_probe) · `confidence` per field · `collected_at`, `qinfer_version`, `schema`.

---

## 18. Appendix B — the quantisation × backend alignment matrix

This table is the single most valuable artefact in this document. Every row is a way to lose 2× performance or an evening of debugging.

| Backend / arch | Best artifact | Acceptable | Avoid | Why |
|---|---|---|---|---|
| Apple Metal via **MLX** | MLX 4-bit DWQ | 6-bit, 8-bit MLX | GGUF in mlx-lm | DWQ is distill-tuned for 4-bit; conversion is hours + RAM |
| Apple Metal via **llama.cpp** | GGUF Q4_K_M / Q5_K_M / Q6_K | Q8_0 | Q2/Q3 | K-quants are the Metal-tuned middle ground |
| **CUDA Blackwell `sm_120/121`** | **NVFP4** | FP8, AWQ/GPTQ INT4, GGUF Q4_K_M | GGUF Q8 on small VRAM | FP4 ≈ 2× fewer bytes/token; near-FP8 accuracy |
| **CUDA Hopper `sm_90`** | FP8 | AWQ/GPTQ INT4, GGUF | — | FP8 is native and well-supported |
| **CUDA Ada `sm_89`** | AWQ/GPTQ INT4, GGUF Q4_K_M | FP8 (limited) | — | INT4 is the practical best |
| **AMD `gfx1151` (ROCm)** | GGUF Q4_K_M / Q5_K_M | MXFP4 (for MoE), Q6_K | Q2/Q3 | No FP4 hardware; bandwidth-bound |
| **AMD (Vulkan)** | GGUF Q4_K_M | Q5_K_M | — | Often beats ROCm, model-specifically |
| **ARM CPU (KleidiAI)** | **Q8_0** for prefill-heavy; Q4_0 for large decode | Q4_K_M | F16 (size) | Q8_0+KleidiAI = 1.73× prefill; Q4_0 repack ≠ KleidiAI gain |
| **ARM CPU (RAM-constrained)** | Q4_0 with repack **off** | Q4_K_M | Q8_0 | Repack doubles resident memory |
| **Hexagon NPU (QNN)** | **Q4_0** (or vendor bundle) | Q8_0 | Q4_K_M, anything K-quant | Non-Q4_0 falls back to CPU per-tensor |
| **Intel (OpenVINO)** | INT4/INT8 OpenVINO IR | INT8 | GGUF | Different toolchain entirely |

**Quality floor, expressed honestly.** A "4-bit" label hides a spread from ~0.5% to >5% perplexity degradation depending on method and model. The catalog must carry a measured delta, and the planner must refuse a quant that violates the user's floor even if it is faster. Speed without a quality constraint is a benchmark, not a product.

**KV cache is part of the quant choice.** `fp8`/`q8_0` KV halves cache size and is nearly free in quality at moderate context; it is the difference between 8k and 32k context on the same box. On long-context plans it matters more than the weight quant.

---

## 19. Appendix C — glossary and sources

**Glossary.** *CFP* — Capability Fingerprint. *Plan* — a fully-resolved, pinned provisioning proposal. *Receipt* — a signed record of what was installed and measured. *k_cal* — calibration coefficient: the ratio between probe-measured bandwidth and the bandwidth the executing device actually achieves. **Not** an efficiency, and not bounded by 1.0. *Active parameters* — parameters actually read per token (MoE). *GTT* — AMD's graphics translation table (unified-memory carveout). *sm_121* — compute capability 12.1, GB10/DGX Spark. *DWQ* — distillation-aware weight quantisation (Apple/MLX). *NVFP4* — NVIDIA 4-bit float format.

**Primary sources consulted** (October 2026):

- NVIDIA DGX Spark product page and hardware guide — GB10 specs, 273 GB/s, 128 GB unified, 20-core Arm CPU.
- NVIDIA technical blog, "How DGX Spark's Performance Enables Intensive AI Tasks" — FP4 inference results, dual-node Qwen3-235B at 11.73 tok/s.
- vLLM issue #36821 and PR #38484 — `sm_121` cubin gaps on aarch64; `_qutlass_C` coverage.
- vLLM blog, "vLLM on the DGX Spark" (Jun 2026) — unified-memory flag guidance.
- vLLM installation docs — aarch64 wheel matrix and source-build path.
- Qualcomm GenieX documentation and README — `llama_cpp` vs `qairt` runtimes; Q4_0 vs Q4_K_M on HTP; bundle-baked precision/context.
- Qualcomm AI Hub "Prepare a GenAI model" — QAIRT ≥ 2.29, Hexagon v73+, memory requirements.
- Arm Learning Paths, KleidiAI/SME2 integration — repack path, kernel selection order.
- kleidibench findings — Q8_0 prefill 1.73×, Q4_0 parity, 2× RSS from repack.
- llama.cpp `docs/build.md` and backend list — Metal, CUDA, HIP, Vulkan, SYCL, CANN, OpenCL, Hexagon, RPC, WebGPU.
- HuggingFace/PyTorch MPS notes and PyTorch MPS PRs — bf16, SDPA coverage, macOS/Python floors.
- ml-explore/mlx issue #3467 — JACCL RDMA GID failure; TB5 bandwidth reality.
- AMD Strix Halo community benchmarks (lhl, remy.io, strix-halo-guide, apellegr) — ~215–239 GB/s measured, GTT boot config, Vulkan-vs-ROCm model dependence.
- NVIDIA Jetson Thor product page — 128 GB, 273 GB/s, 40–130 W.
- Ollama vs vLLM concurrency study (MDPI, May 2026) — 20–29× throughput gap, P95 latency.
- Ollama FAQ and engine-switch reporting — `OLLAMA_NUM_PARALLEL`, backend change to MLX.
- Qualcomm "Local LLMs" docs — Snapdragon X/X2 runtime options.

**Caveat.** Vendor TOPS figures are marketing numbers computed under favourable assumptions (often sparse, often INT8/FP4 peak). They are recorded for display and for detecting a *mismatch* between declared and achieved, never used directly in the predictor.

---

## 20. Appendix D — Detection field notes

The fingerprint layer is where this product is won or lost, and it is mostly a catalogue of traps. These were verified on real machines during discovery; they belong in code as tests, not as folklore.

### 20.1 Traps that silently produce wrong fingerprints

| OS | Trap | Correct approach |
|---|---|---|
| macOS | `hw.l3cachesize` **does not exist** on Apple silicon (empty output) | Guard every `int()`; absent ≠ 0 |
| macOS | i8mm flag is `hw.optional.arm.FEAT_I8MM` (**all caps**), while dotprod is `FEAT_DotProd` (mixed case) | Match the exact key; the lowercase form returns nothing |
| macOS | `FEAT_SVE` / `FEAT_SVE2` **never exist** on Apple silicon — Apple implements its own SME/streaming SVE | Absence means "not architectural SVE", not "error" |
| macOS | `pmset -g thermlog` **streams forever** and will hang a naive subprocess | Never call it without a timeout; prefer a bounded probe |
| macOS | `pmset -g therm` prints "No thermal warning level has been recorded" on Apple silicon | The field is Intel-era; treat as unknown |
| macOS | `powermetrics` is **root-only** (exits 1) but `--help` works unprivileged | Probe capability with `--help`, never by running a sample |
| macOS | `sysctl iogpu.wired_limit_mb` returns `0` when unset | `0` means "system default", not "zero MB" |
| macOS | `platform.processor()` returns **`''`** on Python 3.14/arm64 | Never use it as an architecture check — it silently misclassifies Apple Silicon |
| macOS | `recommendedMaxWorkingSetSize` = 74.0 % of RAM, `maxBufferLength` = 0.75 × that | Measure both per machine; neither ratio is documented |
| macOS | `vm.swapusage` is a **human string**, not JSON | Regex parse; a box with 10 GB swap silently degrades bandwidth probes (observed 238 → 11 GB/s) |
| macOS | `system_profiler` in text mode is locale-sensitive; `sppci_cores` is a **string** | Always `-json`; parse `sppci_cores` as text→int |
| macOS | Bare `ioreg -l` dumps 100 kB+ of IOKit diagnostics | `ioreg -rc IOAccelerator -l \| grep gpu-core-count` |
| macOS | Metal family symbols newer than the build SDK don't exist | Probe `MTLGPUFamily(rawValue:)` numerically, not by symbol |
| macOS | `recommendedMaxWorkingSetSize` is ~0.74 × RAM; `maxBufferLength` ~0.55 × RAM; neither is documented | Measure the ratio per machine; never hardcode |
| Linux | `/proc/cpuinfo` uses `flags` on x86 but **`Features`** (capital F) on arm64 | One parser keyed on `flags` returns empty on every ARM box |
| Linux | `/proc/device-tree/model` is **NUL-terminated** | `tr -d '\0'` or comparisons silently fail |
| Linux | cgroup v2 `memory.max` may be the string `"max"` | Sentinel handling; v1 uses different paths entirely |
| Linux | `nvidia-smi --format=csv,json` **does not exist** | `csv,noheader,nounits` for tabular; `-q -x` for XML |
| Linux | `nvidia-smi -q -d MEMORY,CLOCK -x` is invalid — `-d` and `-x` cannot combine | Two calls, not one |
| Linux | `nvidia-smi` exit codes carry the diagnosis (9 = driver not loaded, 15 = fell off bus, 4 = permission) | Map codes to tailored errors |
| Linux | `/proc/driver/nvidia/version` is **absent on WSL** | Never use it as the sole NVIDIA probe |
| Linux | WSL exposes `/dev/dxg` and **no** `/dev/nvidia*` | Detect GPGPU passthrough explicitly; predictions must subtract host overhead |
| Linux | `rocm-smi --showproductinfo` **is not a flag** | It is `--showproductname`; also accept `amd-smi` on ROCm 7 |
| Linux | On Strix Halo, `VRAM Total` may read ~1 GiB while **GTT** is the real budget | Model GTT as the effective capacity, and note the boot-time carveout |
| Linux | RAPL energy counters are root-readable and **wrap** | Two reads plus Δt; handle wrap via `max_energy_range_uj` |
| Windows | `Win32_VideoController.AdapterRAM` **saturates at ~2 GiB** | Useless for VRAM; DXGI `GetDesc3.DedicatedVideoMemory` is authoritative |
| Windows | DXGI enumerates software adapters | Exclude `DXGI_ADAPTER_FLAG_SOFTWARE` / WARP |
| Windows | `Get-PnpDevice -Class ComputeAccelerator` is the NPU probe, but a driverless NPU hides under `System` | Absence ≠ no NPU; also check the `{F01A9D53-…}` registry class |
| Windows | WoW64 processes report `PROCESSOR_ARCHITECTURE=AMD64` on ARM64 | Use `PROCESSOR_ARCHITEW6432` or `OSArchitecture` |
| Windows | `dxdiag /t` is 5–20 s, blocks on first-run prompts, and is locale-dependent | Never on the hot path |
| Windows | `Get-ComputerInfo` queries everything and takes seconds | Use targeted CIM classes |

### 20.2 Probe discipline for bandwidth

The bandwidth number drives every prediction, so its measurement protocol is a product requirement:

1. Buffer **≥ 4× last-level cache**, or you are measuring cache.
2. **Pin to one thread first**, then scale to physical cores to find the knee — and **publish both numbers**. On the M5 Pro the gap was **1.74×** (131 GB/s at 1 thread, 228 GB/s at 15); a probe that stops at one thread silently halves every prediction (§21).
3. Record `MemAvailable`, swap-in-use and memory pressure **at measurement time** and refuse to publish a bandwidth figure if the box is paging.
4. Take the best of N, and report median + spread. A 15% spread means the machine is thermally or electrically unstable, which is itself a finding.
5. On CUDA/ROCm, use the vendor bandwidth test or a D2D copy with the same discipline; on unified-memory parts, measure **both** host and device paths.

### 20.3 What is genuinely undetectable, and the honest fallback

| Quantity | Why it's hidden | Fallback |
|---|---|---|
| ANE/GPU clocks and die temperature on macOS | `powermetrics` is root; `pmset` has no Apple-silicon data | Report `null`; derive headroom from a 30 s sustained-vs-burst ratio |
| Thermal headroom on any OS | No unprivileged sensor path; RAPL is root and absent in VMs | `thermal_derate = sustained/peak` from a 30 s load — a *measurement*, not a sensor reading |
| NPU microarchitecture, real TOPS | No OS API exposes it on any platform | Vendor enum + `npu.usable` boolean, set only when an execution provider actually loads a model |
| fp8 support on Apple | `FEAT_FP8` is 0, but no API guarantees the negative | Runtime probe: compile and run a tiny kernel, or gate on Metal family |
| DIMM channel count on OEM/ARM boards | SMBIOS empty on ARM, OEMs omit slots | Assume 1 for soldered LPDDR; **measured bandwidth is the authority** |
| Kernel-shape efficiency | Depends on the kernel, not the silicon | Microbenchmark 4–6 shapes, including a prefill-length sweep |

The pattern is consistent: **where a sensor is unavailable, substitute a short controlled experiment.** That is more expensive than reading a file, and far more honest than reading a spec sheet.

---

## 21. Appendix E — Evidence base

This document is no longer purely analytical. Two bodies of evidence now back it, and both are versioned in-repo.

### 21.1 What exists

| Artefact | Path | What it establishes | Confidence |
|---|---|---|---|
| Apple Silicon dossier | `docs/research/01-apple-silicon.md` | Hardware tiers, effective decode bandwidth per chip, engine stack, install paths, failure modes | High on the host machine, medium on Ultra scaling |
| GB10 / DGX Spark dossier | `docs/research/02-nvidia-gb10.md` | `sm_121` timeline, the `tcgen05`/TMEM constraint, measured GB10 corpus, container paths | High on specs and trackers, medium on performance |
| Snapdragon dossier | `docs/research/03-qualcomm-snapdragon.md` | Hexagon runtime model, `Q4_0` requirement, NPU-vs-CPU reality, per-chipset bundle constraints | Low–medium on performance (no hardware) |
| M0 spike | `spikes/m0/REPORT.md` | The predictor, tested: MAPE 1.45 % vs 15.6 % naive on three models | High for this machine, narrow scope |
| Spike harness | `spikes/m0/{fingerprint,spike}.py` | Reproducible fingerprint + benchmark + regression | Working code |

### 21.2 What the spike settled

- The **two-parameter model** is materially better than the naive roofline where it matters (small models), and equivalent where it doesn't.
- **`k_cal` may exceed 1.0**, so the PRD's original "efficiency" framing was wrong and has been corrected.
- **Bandwidth must be measured saturated**, not single-threaded.
- **Prefill is compute-bound and quant-insensitive** on the same model (2 798 vs 2 820 tok/s for Q4 vs Q8), while decode scales with bytes — the split model in §5.2 holds.

### 21.3 What it did not settle

| Gap | Why it matters | Cheapest way to close it |
|---|---|---|
| No MoE measurement | `t₀` dominance is the whole MoE story; untested here | One 30B-A3B Q4 run on ≥ 24 GB free |
| No CUDA measurement | The result could be a Metal artifact | Same harness on any `sm_89`/`sm_120` box |
| No batched decode | §5.2.2's two-regime model is untested | Sweep B ∈ {1, 4, 16, 64} |
| No long context | The KV term and the q8_0 correction are unverified | 8k / 32k runs with `-ctk q8_0` |
| No NPU measurement | Dossier 03 is desk research | A borrowed X Elite, one afternoon |

**The honest position:** one platform, three models, one engine. That is enough to justify building M0 and enough to have corrected four claims — but it is not enough to claim the predictor generalises. The next spike exists to try to break it.

---

*End of v0.2. The document is now evidence-backed on one platform and evidence-*informed* on three others. The next artefact is the SDD: ADRs for the decisions in §16, milestone specs with acceptance tests, and a second spike aimed squarely at MoE and CUDA — the two gaps most likely to invalidate §5.2.*
