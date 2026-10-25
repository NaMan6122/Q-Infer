# Dossier 02 — NVIDIA GB10 / DGX Spark (Blackwell, CUDA sm_121)

**Status:** research v1 · **2026-10-24** · desk research only; no GB10 in hand. Confidence: hardware **high** (vendor docs), sm_121 software state **high** (dated primary trackers), performance **medium** (mixed vendor/community, unnormalised). Anything not traceable to §11 is `UNVERIFIED`.

---

## 1. Verdict

**Tier B today. Tier A is blocked on hardware, not software** — PRD §8.1 requires a human to have run it on a box we control.

- **Out of the box:** Ollama, llama.cpp built for `sm_121`, TensorRT-LLM (beta, single-node), NIM, DGX OS.
- **Container-gated:** vLLM and SGLang. NGC is the supported path; pip wheels are *usually* fine now and were not a year ago.
- **The real problem isn't the wheel.** `sm_12x` has **no `tcgen05`/TMEM**, so datacenter-Blackwell FP4 kernels must be *rewritten*, not recompiled — a multi-quarter gap, not a flag. The 2025 "no sm_121 wheels" story is stale: aarch64 cu130 wheels now run via the `12.0` family target (§3.3).
- **We refuse to promise:** NVFP4 beats FP8 here (often not, §5); dual-Spark doubles decode; 200B+ is usable; the burst number is sustainable (§8).
- **One-liner:** a capacity machine at a bandwidth price — buy it for a 120B MoE, not a tok/s contest.

---

## 2. Hardware facts

| Field | Value | Src |
|---|---|---|
| SoC | GB10 Grace Blackwell Superchip (NVIDIA + MediaTek) | [1] |
| CPU | 20-core Arm: 10× Cortex-X925 + 10× Cortex-A725 | [1][2] |
| GPU | Blackwell, **6,144 CUDA cores**, 5th-gen Tensor Cores, 4th-gen RT | [2] |
| SM count | **48** — derived (6,144 ÷ 128/SM); not published | derived |
| Compute capability | **12.1 → `sm_121`**, `__CUDA_ARCH__` 1210 | [3][4] |
| Memory | 128 GB LPDDR5X **unified**, 256-bit, 16 ch, 8533 MT/s, **273 GB/s** | [1][2] |
| FP4 peak | 1 PFLOP **sparse** (~500 TFLOPS dense) | [1] |
| NIC | ConnectX-7 **200 Gb/s**, 2× QSFP, Ethernet/RoCE only | [2][6] |
| Cluster | 2 direct; 3-node ring direct; **4 requires a switch** | [7] |
| Power / OS / price | 240 W PSU, **GB10 SoC TDP 140 W**, GPU capped ≈120 W / DGX OS 7.6.0 (Ubuntu 24.04.5) / **$4,699** MSRP (from $3,999, 2026-02-25) | [1][8][9][10] |

**Adjacent, different silicon — never transfer results.**

| | DGX Spark | Thor T5000 | Thor T4000 | RTX PRO 6000 |
|---|---|---|---|---|
| CC | **12.1** | **11.0** | 11.0 | **12.0** |
| Cores | 6,144 | 2,560 (96 TC) | 1,536 (64 TC) | 24,064 (752 TC) |
| FP4 sparse | 1 PF | 2,070 TF | 1,200 TF | ~4,000 TOPS |
| Memory / BW | 128 GB / 273 | 128 GB / 273 | 64 GB / 273 | 96 GB GDDR7 / **1,792** |
| `tcgen05` | **No** | **Yes** (`sm_110a`) | No | No |
| Power | 140 W | 40–130 W | 40–70 W | 600 W |
| Src | [1][2] | [11] | [11] | [12] |

NVLink-C2C (~5× PCIe Gen5) is what makes that pool coherent [5]. Near-identical bandwidth, totally different tensor-core ISA: **Thor kernels do not port to Spark** [11][13].

---

## 3. The sm_121 problem, in detail

### 3.1 What 12.1 is

`12.1` is a *minor revision* of `12.0`. In `cicc` v13.0 no code path distinguishes them; only the `__CUDA_ARCH__` macro (1200 vs 1210) differs [4]. NVIDIA's maintainer: *"`sm_121` is SASS binary compatible with `sm_120`"* [14]. DGX Spark and RTX Spark N1X report it [3]. The suffix is what carries meaning:

| Target | Meaning | Covers 12.1? |
|---|---|---|
| `sm_120` | arch-specific, no arch-conditional | yes (SASS-compatible) |
| `sm_120a` | arch-conditional: all `sm_120` instrs | yes |
| `sm_120f` | **family** target — covers 12.0 and 12.1 | **yes, by design** |
| `sm_121` / `121a` / `121f` | native | yes |

### 3.2 The distinction that matters: no `tcgen05`

| | SM 10.x (B200/GB300) | SM 12.x (GB10, RTX PRO, RTX 50xx) |
|---|---|---|
| 5th-gen tensor path | `tcgen05.*` | `mma.*` + `block_scale` |
| Tensor Memory | yes | **no** |
| Async tile-level MMA | yes | no — warp-level `mma.sync` |

NVIDIA staff, 2025-12-19: *"tcgen05, TMEM are not supported on Spark"* [13]. NVIDIA, 2026-04-24: SM 10.x uses `tcgen05.*`, SM 12.x uses `mma.*` — *"the difference in PTX naming reflects ISA compatibility conventions, not generational capability"* [15].

So every datacenter-Blackwell FP4 kernel (CUTLASS `tcgen05`, FlashInfer trtllm-gen FMHA, DeepGEMM) is **inapplicable**, not merely unbuilt: CUTLASS's CuTe DSL was told *"The tcgen05 instructions do not exist for SM121, so this is not possible to fix"* [16]. Community work reached **356 TFLOPS dense NVFP4** only by retargeting a *non*-`tcgen05` path with a one-line `sm_121a` whitelist patch [17] — the hardware can, but the software had to be rewritten.

### 3.3 Why published wheels lacked 12.1

Three mechanisms, routinely conflated:

1. **Build scripts never requested it.** vLLM's `CMakeLists.txt` listed `12.1` per-kernel, but CI/Dockerfile `TORCH_CUDA_ARCH_LIST` never did, so the intersection dropped it from every artifact [18][19].
2. **The CUDA-13 allow-list stripped it.** `CUDA_SUPPORTED_ARCHS` for CUDA ≥13.0 ended at `12.0`, so `12.1` was **silently downgraded** before any kernel guard saw it [19][20] — producing `Not building NVFP4 as no compatible archs were found.` [21]
3. **Runtime guards matched the literal `120`.** Marlin's non-MoE FP8 check rejected `major == 12` — kernel present, launch refused [20].

**Fixed:** #38126 (merged 2026-03-27) added arch guards [21]; **#43003 (merged 2026-05-25)** added `12.1` to the CUDA-13 allow-list and fixed the Marlin runtime check [20]. That pair is load-bearing.

**Not fixed:** #38484 (2026-03-29) is still open and `needs-rebase` [18]. #52708 was **closed unmerged 2026-09-14**, *"due to inability to reproduce"*, after its author withdrew the `sm_121a`-vs-`12.0f` justification under reviewer SASS evidence [19]. So **`12.1` never appears in published `TORCH_CUDA_ARCH_LIST`**; wheels rely on `12.0`/`12.0f`, which empirically works — on one NVFP4 MoE, published vs native-`sm_121` measured 16.4 vs 16.3 tok/s [25].

### 3.4 The `_qutlass_C` gap

The one hard, reproducible hole. Static audit of the published `vllm-0.29.0-cp38-abi3-manylinux_2_28_aarch64.whl` (295.7 MB; declares `torch==2.13.0`, `nvidia-cutlass-dsl==4.6.2`) [22]:

| Extension | Published wheel | Local `12.0` build |
|---|---|---|
| `_C_stable_libtorch` | sm_80…sm_100 sm_110 **sm_120** | sm_80 sm_89 sm_90 **sm_120** |
| `_moe_C_stable_libtorch` | sm_80…sm_100 sm_110 **sm_120** | sm_80 **sm_120** |
| `_flashkda_C` | sm_90a sm_100 **sm_120** | **sm_120** |
| **`_qutlass_C`** | **sm_100 only** | **sm_120** |

On any SM12x device there is **no compatible image** for `matmul_mxf4_bf16_tn`, `fusedQuantize*`, or the compressed-tensors NVFP4 *transform* scheme.

**Why it fails late:** `platforms/cuda.py` imports under `contextlib.suppress(ImportError)` and `_custom_ops.py` gates on `hasattr(torch.ops, "_qutlass_C")`. The `.so` loads and ops register regardless of the cubins inside — both checks pass, the failure surfaces at *kernel launch* [22]. The "silent at install, loud at 3 a.m." class we must catch statically.

```bash
cuobjdump -lelf whl/vllm/_qutlass_C.abi3.so | grep -oE 'sm_[0-9]+[a-z]?' | sort -u
```

### 3.5 PyTorch CMake: 12.1 → 12.1(2.0)

`select_compute_arch.cmake` rewrites the legacy Fermi arch `2.1` → `2.1(2.0)` with an **unanchored substring replace**. Auto-detection returns `12.1` on GB10; the embedded `2.1` matches; the result dot-strips to `-gencode arch=compute_20,code=sm_121` → `nvcc fatal: Unsupported gpu architecture 'compute_20'` [14][23].

| Question | Answer |
|---|---|
| Who hits it? | **Only from-source builds using auto-detection**; wheels never run this code |
| Reproduced? | Yes, twice independently, real GB10, CUDA 13.0 [14][23] |
| Fixed? | #174065 (2026-02-02), #185993 still open — `UNVERIFIED` as merged |
| Workaround | `TORCH_CUDA_ARCH_LIST=12.1`; on CUDA 13 you also need `12.1` in `CUDA_SUPPORTED_ARCHS` (§3.3) |

### 3.6 PTX JIT fallback — real cost

torch `2.13.0+cu130` aarch64 reports `arch_list = ['sm_80','sm_90','sm_100','sm_110','sm_120']` plus PTX; the warning that fired on `2.10.0` is **suppressed from 2.13.0** via `DEVICE_REQUIREMENT[120] = >=12.0,<13.0` [24].

| Consequence | Note |
|---|---|
| First-run JIT | **Minutes** per kernel set; cache in `~/.nv/ComputeCache`; keep out of the timed run |
| Arch-conditional mismatch | **Launch abort** — JIT'd PTX cannot use `a`-only instructions it wasn't compiled for |

**Auditor caveat:** `cuobjdump -lelf` prints a family-target (`12.0f`) cubin as plain **`sm_120`**, indistinguishable by name from an arch-specific one. "Shows sm_120, not sm_121" is **not** evidence of a gap [22]. Catalog the *loader-visible* result, not just the static list.

### 3.7 Confirmed vs inferred

| Claim | Status |
|---|---|
| GB10 reports CC 12.1 | **Confirmed** [2][3][4] |
| SM 12.x lacks `tcgen05`/TMEM | **Confirmed** (NVIDIA staff) [13][15] |
| `_qutlass_C` sm_100-only in 0.29.0 aarch64 | **Confirmed** (reproducible audit) [22] |
| Runtime launch failure from that gap | **Inferred** from the missing image; not reproduced by reporter [22] |
| `12.0f` substitutes for `12.1a` on vLLM kernels | **Confirmed for one workload** [25]; disputed in general [19] |

---

## 4. Software stack map

| Framework | Install path | Version | arm64+CUDA13 | FP4/NVFP4 | Maturity | Verdict |
|---|---|---|---|---|---|---|
| **TensorRT-LLM** | NGC, arm64 multi-arch | 1.2.1; 1.4.0rc2 | Yes, native | **Best** | Beta, single-node | **Peak perf.** Model list is a whitelist [26][27] |
| **vLLM** | pip `+cu130` aarch64 / `vllm-openai:cu130` | **0.29.0** | Yes, `12.0` family | Good | Stable w/ caveats | **Default engine.** `_qutlass_C` gap [22][28] |
| **SGLang** | NGC `sglang:26.06-py3` | 0.5.12.post1 | **Container only** | Good | Fragile | PyPI kernel has no sm121 dir [29][30] |
| **llama.cpp** | source | master | **Yes, best** | MXFP4 native | Solid | **Universal fallback.** `-DCMAKE_CUDA_ARCHITECTURES=121a-real` [31][41] |
| **Ollama** | binary/snap, preinstalled | ≥0.12.6 | Yes | MXFP4 | **Highest** | **Best out-of-box.** No flags or pinning [32][33] |
| **NIM** | NGC + API key | per-model | Yes | NVFP4 | Supported | **Licence-gated.** Best curated; needs a manifest air-gapped [34] |
| **ONNX Runtime** | source / community wheel | 1.30.x | **No official prebuilt** | limited | Poor | **Source build.** aarch64 CUDA only from 2026-04-29 [35][36] |
| **PyTorch** | pip cu130 aarch64 | **2.13.0** | Yes | via kernels | Stable | **Foundation**, not a server [24] |
| **MLC-LLM** | source | — | `UNVERIFIED` | `UNVERIFIED` | Unassessed | **Watch.** No dated sm_121 report found |

---

## 5. Measured performance corpus

Single-stream decode unless noted. Methodologies are **not** comparable across rows — read this as a distribution.

| Model | Quant | Backend | Decode | Prefill | Nodes | Src | Date |
|---|---|---|---|---|---|---|---|
| Qwen3-14B | NVFP4 | TRT-LLM | 22.71 | 5,929 | 1 | [37] | 2025-10-24 |
| gpt-oss-20b | MXFP4 | llama.cpp | **82.74** | 3,670 | 1 | [37] | 2025-10-24 |
| gpt-oss-120b | MXFP4 | llama.cpp | 55.37 | 1,725 | 1 | [37] | 2025-10-24 |
| Llama-3.1-8B | NVFP4 | TRT-LLM | 38.65 | 10,257 | 1 | [37] | 2025-10-24 |
| Qwen3-235B | NVFP4 | TRT-LLM | 11.73 | 23,477 | **2** | [37] | 2025-10-24 |
| gpt-oss-20b | MXFP4 | Ollama | 58.27 | 3,224 | 1 | [32] | 2025-10-23 |
| gpt-oss-120b | MXFP4 | Ollama | 41.14 | 1,169 | 1 | [32] | 2025-10-23 |
| llama3.1-8B | Q4_K_M | Ollama | 38.02 | 7,614 | 1 | [32] | 2025-10-23 |
| llama3.1-70B | Q4_K_M | Ollama | **4.42** | 1,911 | 1 | [32] | 2025-10-23 |
| qwen3-32B | Q4_K_M | Ollama | 9.41 | 705 | 1 | [32] | 2025-10-23 |
| gpt-oss-120b | MXFP4 | llama.cpp | 38.55 | 1,723 | 1 | [38] | 2025-10-15 |
| glm4moe-106B-A12B | Q4_K | llama.cpp | 18.45 | 817 | 1 | [38] | 2025-10-15 |
| gpt-oss-120b | MXFP4 | vLLM | 58.82 | — | 1 | [39] | 2025-10-14 |
| gpt-oss-120b | MXFP4 | vLLM | **75.96** | — | **2** | [39] | 2025-10-14 |
| Qwen3-Coder-Next | FP8 | SGLang | 60.51 | — | **2** | [39] | 2025-10-14 |
| Qwen3-235B-A22B | GPTQ-Int4 | vLLM TP=2 | 17.0 | — | **2** | [40] | 2026-05-03 |
| Qwen3.5-122B-A10B | NVFP4 | vLLM 0.27.1 | 16.4 | — | 1 | [25] | 2026-08-13 |

| gpt-oss-120b | MXFP4 | vLLM `--enforce-eager` | **26** (59 w/ graphs) | — | 1 | [40] | 2026-05-02 |

- **Vendor and community agree within ~25%** on gpt-oss-120b (41–59 tok/s, four sources). That is our noise floor — anything inside it is not a finding.
- **NVFP4 is not automatically faster:** GPTQ-Int4/vLLM beats NVFP4/TRT-LLM by **45%** on the same 235B model [40].
- **Dual-node buys capacity, not speed** — 1.3× on a MoE that already fits one node.
- **`--enforce-eager` costs ~55%.** Never a default.

---

## 6. Install paths that work

**Pin digests, never tags.** `nvcr.io/nvidia/tensorrt-llm/release:1.4.0rc2.dev202610250007` carries separate amd64/arm64 digests; a tag pull on a Spark silently resolves arm64 [27].

```bash
### 1 — vLLM, official cu130 image
docker run -d --name vllm --ipc=host --restart unless-stopped --gpus all -p 8000:8000 \
  -e HF_TOKEN="$HF_TOKEN" -v ~/.cache/huggingface:/root/.cache/huggingface \
  vllm/vllm-openai:v0.29.0-cu130 \
  nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-NVFP4 \
    --max-model-len 131072 --gpu-memory-utilization 0.85 --max-num-seqs 4
docker inspect --format '{{index .RepoDigests 0}}' vllm

### 2 — llama.cpp for sm_121 (GGUF breadth nothing else has)
sudo apt install -y git cmake clang libcurl4-openssl-dev
cmake -B build -DGGML_NATIVE=ON -DGGML_CUDA=ON -DGGML_CURL=ON \
      -DCMAKE_CUDA_ARCHITECTURES=121a-real && cmake --build build -j

### 3 — Ollama (ships on DGX OS; snap needs an explicit bind)
curl -fsSL https://ollama.com/install.sh | sh   # or: snap set ollama host="0.0.0.0:11434"

### 4 — toolkit sanity (DGX OS 7.6.0 ships Container Toolkit 1.20.0)
docker run --rm --gpus all nvcr.io/nvidia/cuda:13.0.1-devel-ubuntu24.04 nvidia-smi
```

| Pitfall | Detail |
|---|---|
| **Driver/CUDA skew** | 7.6.0 = driver **580.173.02**, CUDA **13.0.3**, TRT 10.14.1 — but NVIDIA's two docs disagree (580.173.02 vs 580.178.04). Pin what `nvidia-smi` says [9]. |
| **`gpu-memory-utilization` is a fraction of *total*, not free** | 0.85 asks 103 GiB; 35 GiB resident → *startup failure*. Empty the box; don't lower the flag (it silently shrinks KV cache) [42]. |
| **NCCL / cuDNN** | 7.5.0 → NCCL 2.29.7 / cuDNN 9.20.0; 7.6.0 → 2.31.2 / 9.24.0 [9]. PyTorch's bundled `libnccl.so.2.28.9` has **no sm_121 cubins** → cross-node TP deadlocks after ~30 min [44]. |
| **Disk / monitoring** | TRT-LLM images are 15.6–16.5 GB compressed *per tag* [27] — budget 200 GB. `nvidia-smi --query-gpu=memory.*` returns `N/A` on UMA; use plain `nvidia-smi` / `free -g` [45]. |

---

## 7. Tuning knobs

| Knob | Range | Effect | Unified-memory note |
|---|---|---|---|
| `gpu_memory_utilization` | 0.60–0.90 | Large (KV) | **Fraction of the pool the OS also lives in.** ≤0.85 plain serving; **≤0.60** if FlashInfer autotune or `torch.compile` runs at startup [42][43] |
| `max_num_seqs` | 1–8 | Moderate, negative past ~4 | Small-batch silicon; 4 is the vendor recipe, 1 for latency [43] |
| `max_model_len` | 8k–262k | Large (KV reservation) | Don't size KV for the max — vLLM schedules on active context [43] |
| `kv_cache_dtype=fp8` | on/off | ~2× context | Halves KV bytes for ~free quality; the difference between 8k and 32k [40] |
| CUDA graphs | on/off | **~2.3×** (59 vs 26) | **Leave on.** SGLang *piecewise* graphs still fail on SM121 (BF16 `cublasGemmEx` → `CUBLAS_STATUS_INTERNAL_ERROR`) → `--disable-piecewise-cuda-graph` [40][46] |
| `--max-cudagraph-capture-size` | 32→2048 | Moderate | Default 32 caps coverage; 2048 costs nothing [40] |
| NVFP4 vs FP8 | — | **Not reliably positive** | Decide per model, measure. Marlin NVFP4 = FP4 storage + BF16 dequant: a bandwidth win, not a tensor-core win [17] |
| `VLLM_UNIFIED_MEMORY_HOST_RESERVE_GB` | 8 default | Prevents wedge | `max(8, 5% of pool)`; raise if the box is shared [47] |
| `nvidia-smi -lgc 300,2200` | 2100–2300 | **~0–3%** | Kills spontaneous shutdowns; uncapped boost trips a protection the OS cannot see [48][49] |

---

## 8. Failure modes

1. **Host hard-wedges at a *conservative* `util=0.70`.** `profile_run`'s transient isn't bounded by `gpu_memory_utilization`, and on UMA the "budget" is the OS's pool (83.7 GiB asked vs 63.3 available). → 4-bit weights; PR #49760 [47].
2. **`Marlin W4A8-FP8 only support SM89 or SM120 device`.** Literal `== 120` check. → #43003 (2026-05-25); pin ≥ that [20].
3. **`[sgl_kernel] CRITICAL: Could not load any common_ops library!`** Only `sm90`/`sm100` dirs shipped. → NGC container; PyPI is not viable [29][30].
4. **`_qutlass_C` imports fine, then launch fails mid-request.** sm_100-only cubins behind a passing `hasattr` guard. → Avoid transform-based NVFP4/MXFP4 [22].
5. **`nvcc fatal: Unsupported gpu architecture 'compute_20'`.** CMake `2.1` substring corruption. → `TORCH_CUDA_ARCH_LIST=12.1` [14][23].
6. **`Not building NVFP4 as no compatible archs found`, then "no kernel image".** `12.1` downgraded by the allow-list. → Patch it, or use `12.0f` [19][20].
7. **Powers off 8–11 s into a 131k prefill, no kernel log.** Boost-clock transient on a hot chassis; averaged wall power is *lower* than in surviving runs. → `-lgc 300,2200` at boot: 2/2 failed default, 0/6 locked (p = 0.036) [48].
8. **Stuck at 507 MHz / 8–10 W at P0, no throttle reason.** USB-PD wedge. → Full drain: unplug wall *and* unit, 30 s button, wait ≥60 s [50].
9. **Identical units, 20 °C apart.** Chassis/VRM variance. → Per-unit derate; measure sustained, never burst [49].
10. **Two-node TP hangs after ~30 min.** PyTorch-bundled NCCL lacks sm_121 cubins. → Locally built NCCL; set `NCCL_IB_HCA`, `NCCL_SOCKET_IFNAME` [44].
11. **4-node switchless ring: diagonal all-reduce hangs.** No 0↔2 / 1↔3 cable; NCCL's tree wants them. → `NCCL_ALGO=Ring` + tree-skip, or a switch (required for 4 nodes anyway) [7][51].
12. **Model >200B "runs" at ~1.8 tok/s.** Spilling — pages, doesn't compute. → Flag an η-floor violation and **refuse**. Dual-Spark 235B at 11.73 tok/s is the honest ceiling [37].

---

## 9. Honest assessment

**Capacity vs bandwidth.** 273 GB/s against ~500 TFLOPS dense FP4 is ~**1,800 FLOP/byte**; an H100 sits near 300. That ratio explains everything: a Spark is compute-rich and bandwidth-poor, so nearly every local workload is memory-bound and the tensor cores are largely decoration — hence a 120B MoE with 12B active decoding at ~56 tok/s while a dense 32B lands at ~9.

**Beats a Mac:** a 120B MoE at 50+ tok/s in 1.1 litres at 140 W. A Mac Ultra cannot reach that class at that speed, and costs more. FP4 is real hardware here, and CUDA is the only ecosystem where TRT-LLM, vLLM and NIM coexist.

**Loses:** any *dense* model above ~30B — 4.4 tok/s on a 70B Q4 is archival [32]. Against an RTX PRO 6000 it gives up 6.6× bandwidth and ~25× FP4 peak, winning only 32 GB of capacity. Against an M3/M4 Ultra it loses the single-stream case it is sold for: Apple's *effective* decode bandwidth still exceeds 273 GB/s.

**Genuinely for:** an always-on agent box holding one 100–130B NVFP4 MoE at 15–60 tok/s with a 128k window; a two-node pair for a 235B; a CUDA dev node for code headed to GB200. A **workstation for model capacity**.

---

## 10. Implications for Q-Infer

| Area | Decision |
|---|---|
| **Tier** | **B** at launch. **A** only with a Spark in CI and §6 run by a human — never on desk research. |
| **Engine adapters** | Four: `llamacpp-cuda`, `vllm` (container-first), `trtllm` (whitelist-only), `ollama`. SGLang: **detect and refuse** outside NGC. |
| **Catalog** | Per artifact: `cubin_archs: ["sm_120f", …]` **and** `verified_launch_ops: […]` — `cuobjdump` can't tell `12.0` from `12.0f`, and `_qutlass_C` passes every import-time check (§3.4, §3.6). |
| **Fingerprint fields** | `arch_id: sm_121`, `arch_family: 12`, `has_tcgen05: false`, `unified: true`, `pool_bytes`, `pool_available_at_probe`, `host_reserve_gb`, `cuda`, `driver`, `nccl_version`, `nccl_has_sm121` (probe, not string), `cudagraph_ok`, `piecewise_cudagraph_ok`, `clock_lock_supported`. |
| **Contract tests** | (a) per-op kernel launch; (b) 1B bf16 model, one decode, under a `MemAvailable` watchdog — **this catches the host wedge**; (c) graph on/off delta >1.5× or graphs are silently broken; (d) `_qutlass_C` audit asserting `launchable == expected`; (e) `-lgc` probe (thermal workaround is P0). |
| **CI hardware** | **One 128 GB Spark is a hard requirement** for Tier A; two for NCCL/dual-node rows. Rent until M1. |
| **PRD correction** | §2.2-ii/§9 say published wheels "compiled for `sm_120` and below" with "hard failures". True Q4-2025→Q1-2026; **not true of 0.29.0**. Durable replacement: *`sm_12x` has no `tcgen05`, so datacenter-Blackwell FP4 kernels cannot be recompiled onto it — only rewritten* [13][15][22]. |

---

## 11. Sources

Accessed **2026-10-24**.

1. nvidia.com/en-us/products/workstations/dgx-spark/ — specs
2. docs.nvidia.com/dgx/dgx-spark/hardware.html
3. developer.nvidia.com/cuda/gpus — CC table
4. gh.evko.io/nvopen-tools/cicc/targets/sm120.html
5. NVIDIA PR, DGX Spark ships, 2025-10-13
6. docs.nvidia.com/dgx/dgx-spark/spark-clustering.html
7. docs.nvidia.com/sync/latest/cluster-assistant.html
8. spheron.network — NVIDIA forums (NVES) power budget, 2026-10-06
9. docs.nvidia.com/dgx/dgx-os-7-user-guide/release_notes.html; …/dgx-spark/release-notes.html
10. forums.developer.nvidia.com/t/361713 — price change, 2026-02-23
11. NVIDIA, Jetson Thor datasheet + launch blog, 2025-08-25
12. nvidia.com/…/rtx-pro-6000/ + RTX Blackwell PRO whitepaper v1.1
13. github.com/NVIDIA/dgx-spark-playbooks **#22**, 2025-12-19
14. github.com/pytorch/pytorch **#172629**, **#174065**
15. forums.developer.nvidia.com/t/367781 — NVFP4 update, 2026-04-24
16. github.com/NVIDIA/cutlass **#2947**, 2026-01-11
17. forums.developer.nvidia.com/t/364607 — SM121 NVFP4, 2026-03-24
18. github.com/vllm-project/vllm **#38484**, open
19. github.com/vllm-project/vllm **#52708**, closed 2026-09-14
20. github.com/vllm-project/vllm **#43003**, merged 2026-05-25
21. github.com/vllm-project/vllm **#38126**, merged 2026-03-27
22. github.com/vllm-project/vllm **#36821** — cubin audit, 2026-09-10
23. pytorch/pytorch **#185993**; k2-fsa/k2 **#1354**
24. pytorch/pytorch **#178891**, **#172629** (2026-09-01 comment)
25. vllm **#38484** comment, 2026-08-13 (published-vs-native tok/s)
26. nvidia.github.io/TensorRT-LLM/release-notes.html + v1.2.0, 2026-03-12
27. catalog.ngc.nvidia.com — tensorrt-llm spark-single-gpu-dev tags
28. vllm.ai/releases — v0.29.0, 2026-09-09
29. github.com/sgl-project/sglang **#29317**, 2026-06-25
30. sglang **#11658**; github.com/ubehera/sglang-spark; xomoxcc/dgx-spark-sglang
31. dgx-spark-playbooks llama-cpp README; llama.cpp **#20405**
32. ollama.com/blog/nvidia-spark-performance, 2025-10-23
33. docs.ollama.com/gpu
34. build.nvidia.com/spark/nim-llm, 2026-09-30
35. microsoft/onnxruntime **#27005**, **#27760**
36. github.com/Albatross1382/onnxruntime-aarch64-cuda-blackwell, 2026-04
37. developer.nvidia.com/blog/how-nvidia-dgx-sparks-performance-enables-intensive-ai-tasks/, 2025-10-24
38. hardware-corner.net/first-dgx-spark-llm-benchmarks/, 2025-10-15
39. github.com/ggml-org/llama.cpp/discussions/16578, 2025-10-14
40. conselara.dev/notes/dgx-spark-benchmarks/, 2026-05-09
41. llama.cpp **#20405** benchmark comment, 2026-03
42. gpuwerk.com/docs/vllm/
43. vllm.ai/blog/2026-06-01-vllm-dgx-spark
44. github.com/ubehera/sglang-spark — NCCL 2.28.9 gap
45. build.nvidia.com/spark/vllm/troubleshooting
46. github.com/NVIDIA/TensorRT-LLM **#11799**
47. github.com/vllm-project/vllm **#46307**, **#49760**
48. petronellatech.com/blog/dgx-spark-shutdown-under-load-our-8-unit-gb10-diagnosis/, 2026-09-26
49. wildpines.ai/blog/your-dgx-spark-is-cooking-itself/, 2026-04-18
50. project.veneta.ai/dgx-spark/
51. github.com/josephdrose/nccl-spark-switchless

**Unverified:** MLC-LLM arm64+CUDA on sm_121 (no dated report); whether PyTorch #174065 merged; GB10 SM count from any primary source (48 is derived); the driver-version discrepancy between NVIDIA's two docs (§6).

---

*End of Dossier 02. Next artefact: an `sm_121` probe spike running §10's five contract tests on a rented Spark, converting Tier B to Tier A on evidence rather than confidence.*
