# Dossier 03 — Qualcomm Snapdragon (Hexagon NPU, Adreno, Oryon)

**Status:** research v1 · 2026-10-26 · desk research; we hold no Qualcomm hardware.
**Confidence:** specs **high**; Hexagon-arch mapping **medium** (cross-read: Qualcomm + LiteRT tables); performance **low–medium** (vendor harness, unreplicated); install paths **medium** (documented, not executed).

## 1. Verdict

- **Tier B** for X Elite, X2 Elite, 8 Elite, 8 Elite Gen 5 and IQ-9075 — catalog models ≤4B on a pre-compiled bundle only.
- **Tier C** for arbitrary GGUF: fingerprint, advise, default to CPU.
- The NPU is real and narrow: 1–4B, 4-bit, **fixed context**, compiled per chipset.
- Above ~4B we recommend the **CPU**, and show why: same X Elite, 7B Q4_0 CPU **24.8 tok/s** vs 8B w4a16 NPU **9.96 tok/s** (§5).
- We refuse "45 TOPS, therefore fast" — TOPS never enters a prediction.
- We refuse a default path needing Secure Boot disabled, which Windows NPU currently does (§6).
- We never redistribute QAIRT or bundles.

## 2. Hardware facts

| Part | CPU | GPU | NPU (Hexagon ver, TOPS) | Memory | BW GB/s | Node | v73+ |
|---|---|---|---|---|---|---|---|
| X Elite (X1E-84/00-1DE) | 12× Oryon, 3.8 / 4.2–4.3 boost | Adreno X1-85, 4.6 TF | **v73** (SC8380XP), 45 | LPDDR5X 8448 MT/s, ≤64 GB | 135 | 4 nm | yes, at the floor |
| X2 Elite (X2E-96/94-100) | 18× Oryon 3rd gen, ≤5.0 | Adreno X2-90 | **v81** (SC8480XP), 80 | LPDDR5X, ≤64 GB | **228** | 3 nm | yes |
| 8 Elite (SM8750) | 2× 4.32 + 6× 3.53 | Adreno 830 | **v79**, not published | LPDDR5X 5300 MHz, ≤24 GB | ~84.8 ᵈ | 3 nm | yes |
| 8 Elite Gen 5 (SM8850) | 2× 4.74 + 6× 3.62 | Adreno 840 | **v81**, not published (+37%) | LPDDR5X 5300 MHz, ≤24 GB | ~84.8 ᵈ | 3 nm | yes |
| Dragonwing IQ-9075 (QCS9075-AA) | 8× Kryo Gen 6 (v8.2), 2.36 | Adreno 663 @800 | **dual v73 HTP**, 100 dense | 6×16-bit LPDDR5, ≤36 GB ECC | ~76.8 ᵈ | `UNVERIFIED` | yes |

ᵈ derived (channels × width × rate). Qualcomm publishes MT/s for laptops, only MHz for phones.

- **X Elite sits exactly on the v73 floor** — older parts are excluded by Qualcomm's own GenAI prerequisite, not our taste.
- **Highest TOPS, oldest NPU.** IQ-9075's 100 dense TOPS is *two* v73 HTPs, 2022 phone microarchitecture. TOPS multiply; ISA does not.
- **X2 Elite spans two bandwidth classes** — X2E-96/94-100 at 228 GB/s, the rest at 152. Read the SKU, never the family.

## 3. The two runtimes, precisely

GenieX ships two stacks that both drive the NPU but share nothing. The runtime is chosen **per model**.

| | `llama_cpp` (ggml-hexagon) | `qairt` (AI Engine Direct / QNN) |
|---|---|---|
| Artifact | GGUF, any publisher | AI Hub bundle: `*.bin` context binaries + tokenizer + `genie_config.json` |
| Compute units | npu (`HTP0`) / gpu (OpenCL) / cpu / hybrid | **npu only**; others coerced with a warning |
| Precision | you choose (`Q4_0`,`Q8_0`,`F16`) | **baked** (`w4a16`, `w4`) |
| Context length | `--nctx` at runtime | **baked**; `--nctx` has no effect |
| KV cache | runtime, flippable to q8_0 | **baked** |
| Portability | any Snapdragon + GGUF | one chipset, one Hexagon arch, one compile-time QAIRT |

**What "baked" means.** A bundle fixes precision, context, KV allocation, graph partitioning, `rope-theta`, `kv-dim`, batch size and NSP core count at export. `context::size` must equal the `--context-length` it was built with; changing it means re-exporting via an AI Hub job.

**Why `Q4_0`, not `Q4_K_M`.** `ggml-hexagon` registers HTP matmul for `q4_0`, `q8_0`, `mxfp4`, `f32` only. K-quants have no HTP op: under `hybrid` the per-tensor scheduler sends those tensors to CPU — correct output, dead speed; under pinned `npu` there is no scheduler, so unsupported ops stay host-side and every token pays the round trip. Qualcomm's own docs: *"Q4_K_M is a suboptimal quant for HTP — use Q4_0 for a clean NPU run."*

**What `hybrid` means.** Empty `device_id` + `n_gpu_layers=-1`: nothing is pinned, and llama.cpp assigns each tensor to whichever backend implements the op. Not a fallback — the fast path. On X1E80100 + Qwen3-1.7B: 27 tok/s decode / 90 prefill hybrid, versus 22 / 60 pinned to `HTP0`, even though pinning also enables q8_0 KV and flash-attn.

**Catalog consequence.** The unit is `(model, chipset_id, hexagon_arch, bundle_id, precision, context_len, qairt_min_version)` — five of seven are artifact properties, not user intent.

## 4. Software stack map

| Runtime | Install path | Model source | NPU/GPU/CPU | Maturity | Verdict |
|---|---|---|---|---|---|
| **GenieX** | Win ARM64 `.exe` (unsigned), Linux `install.sh`, Docker, AAR | GGUF + AI Hub bundles | all three | v0.x, 2026, active | **Primary** — only stack abstracting both runtimes |
| **QAIRT SDK + Genie** | Qualcomm ID + QPM licence; direct zip on Ubuntu 22.04 | AI Hub bundles only | NPU | Mature (2.50.0.260828) | Fallback; needed for BYO bundles |
| **QNN EP (ORT)** | `onnxruntime-qnn==2.8.0` + `onnxruntime==1.30.0` | ONNX, HTP-quantised | all three | Mature; ships QAIRT libs | Viable; static shapes only |
| **ORT GenAI** | NuGet / win-arm64 zip | ONNX-QNN bundles | NPU | Works; short list | Watch |
| **ExecuTorch (QNN)** | Build from source; QNN 2.37 | `.pte` AOT | NPU / Adreno | Research-grade for LLMs | Watch |
| **llama.cpp `ggml-hexagon`** | CMake + Hexagon SDK 6.6/6.7; **no prebuilt Windows ARM64** | GGUF | all three | Experimental, 2026 | Baseline; Q4_0 only |
| **llama.cpp OpenCL** | Prebuilt `win-opencl-adreno-arm64` | GGUF | GPU | Shipped | **Against** (§5) |
| **Windows ML** | `WindowsAppSDK.ML` + dynamic EP | ONNX | NPU via QNN EP | New, driver ≥30.0.140.0 | Watch |
| **DirectML** | Bundled with Windows | ONNX | GPU only | Stagnant for LLMs | No |
| **CPU (ggml)** | Any prebuilt | GGUF | CPU | Mature | **Default above 4B** |

## 5. Measured performance corpus

Published numbers are scarce, vendor-produced and mutually inconsistent. **That scarcity is the finding:** no independent third-party corpus for Hexagon LLM decode exists, unlike CUDA or Metal. Every row below is vendor-harness output.

| Chipset | Model | Quant | Runtime / CU | Decode tok/s | Prefill tok/s (or TTFT) | Implied eff. GB/s (% spec) | Src |
|---|---|---|---|---|---|---|---|
| X Elite | Llama-3.2-3B (3.21B) | w4a16 | QAIRT | **18.6** | 0.114–3.64 s | 30 (22%) | [12] |
| X Elite | Llama-3.1-8B (8.03B) | w4a16 | QAIRT | **9.96** | 0.207–6.61 s | 40 (30%) | [12] |
| X2 Elite | Llama-3.2-3B | w4a16 | QAIRT | **35.5** | 0.061–1.96 s | 57 (25% of 228) | [12] |
| X2 Elite | Llama-3.1-8B | w4a16 | QAIRT | **19.6** | 0.110–3.52 s | 79 (35% of 228) | [12] |
| 8 Elite (SM8750) | Llama-3.1-8B | w4a16 | QAIRT | **11.2** | 0.162–5.17 s | 45 (53%) | [12] |
| 8 Elite Gen 5 | Llama-3.2-3B | w4a16 | QAIRT | **24.6** | 0.079–2.54 s | 40 (47%) | [12] |
| X Elite | Qwen3-1.7B | Q4_0 | llama.cpp hybrid | **27.4** | 91.5 | 35 (26%) | [9] |
| X Elite | Qwen3-1.7B | Q4_0 | llama.cpp **cpu** | **18.5** | 25.3 | 24 (18%) | [9] |
| X Elite | Llama-7B (6.74B) | Q4_0 | llama.cpp **cpu**, 12 thr | **24.8** | 177.7 | 94 (**70%**) | [19] |
| X Elite | Llama-7B | Q4_0 | llama.cpp OpenCL Adreno | **18.0** | 100.7 | 68 (50%) | [19] |
| IQ-9075 | Llama-2-7B | w4 | *datasheet* | **22** | — | 85 (**111% of spec**) | [4] |
| IQ-9075 | Llama-3.1-8B | w4a16 | QAIRT | **9.02** | 0.200–6.40 s | 36 (47%) | [12] |

1. **Qualcomm contradicts itself by 2.4×.** The QCS9075 datasheet claims 22 tok/s on Llama-2-7B; Qualcomm's own AI Hub measures 9.02 tok/s for Llama-3.1-8B on that part — the claim implies 111% of derived spec bandwidth, which is not a decodable measurement. **Use AI Hub, discount datasheets.**
2. **AI Hub aliases rows.** IQ-X7181 reproduces X Elite figures to twelve decimals; Q-8750 reproduces 8 Elite. Alias rows, not measurements — deduplicate before fitting.
3. **No MoE result exists** in either corpus; we predict nothing there.
4. **The one high-efficiency number is the CPU** (70%); the NPU peaks near 30% on the same part.

## 6. Install paths that work

**Floors.** Hexagon **v73+**; **Genie/QAIRT ≥ 2.29.0** (real: 2.29 fixed long-rope scaling, so 2.28-built bundles will not run on 2.27); runtime SDK **≥ the bundle's compile SDK**, current `2.50.0.260828`. RAM: **16 GB for 7B, ~12 GB for 3B** — ~8× the weights, since static KV and activation buffers live there.

**Windows ARM64 (X Elite / X2 Elite).**

```powershell
# drivers (admin, reboot); device must appear under "Neural processors"
# GenieX CLI is unsigned -> SmartScreen "More info / Run anyway"
Set-Alias geniex (where.exe geniex)
geniex pull unsloth/Qwen3-1.7B-GGUF        # choose Q4_0
geniex infer unsloth/Qwen3-1.7B-GGUF --device hybrid
geniex serve --port 8080                   # OpenAI-compatible
```

**Linux ARM64 (IQ-9075 / IQ-8275).**

```bash
sudo apt-get install -y qcom-adreno1 qcom-fastrpc1 libqnn1   # PPA pre-set on the EVK image
curl -fsSL https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-geniex/install.sh | sh
# from a tarball the backend needs the unversioned soname:
sudo ln -sf /usr/lib/aarch64-linux-gnu/libcdsprpc.so.1 /usr/lib/aarch64-linux-gnu/libcdsprpc.so && sudo ldconfig
geniex infer bartowski/Qwen_Qwen3-1.7B-GGUF:Q4_0 --compute hybrid
```

**Android** differs again: no CLI, Kotlin AAR from Maven Central, `adb push` of NPU libs, both `LD_LIBRARY_PATH` and `ADSP_LIBRARY_PATH`, and an explicit chipset on every AI Hub pull (`SM8750`/`SM8850`) because `ro.soc.model` hides variants.

**Bring-your-own bundle.**

```bash
python -m qai_hub_models.models.llama_v3_2_3b_instruct.export \
  --device "Snapdragon X Elite CRD" --context-length 4096 \
  --output-dir ./genie_bundle                        # needs an AI Hub account
genie-t2t-run -c genie_config.json -p "hello"        # -c/-p stable; minor flags vary
```

**Vendor licence gate — not automatable.** QAIRT via QPM needs a Qualcomm ID plus `qpm-cli --license-activate qualcomm_ai_runtime_sdk`; the licence (`qcom-ai-stack`) is internal-use-only. Direct zip download needs no login — convenience, not a grant.

**Windows driver-signing gate.** HTP op libraries must sit inside a `.cat` signed by a trusted certificate: `bcdedit /set TESTSIGNING ON` (**Secure Boot may have to be disabled**), self-signed cert in *both* Trusted and Root stores, reboot. Prebuilt GenieX SDKs may ship Microsoft-signed; the `-selfsigned` flavour does not.

## 7. Tuning knobs

| Knob | Range | Effect | Notes |
|---|---|---|---|
| Compute unit | npu / gpu / cpu / hybrid | 18–27 tok/s spread on one 1.7B | `hybrid` fastest; `npu` deterministic, slower; `qairt` ignores it |
| Quant (GGUF) | Q4_0 / Q8_0 / F16 / MXFP4 | determines placement, not just quality | Only these reach HTP; K-quants fall back per-tensor |
| Precision (bundle) | w4a16 / w4 | fixed at export | w4a16 common default; w4 costs compute, gains accuracy |
| Context (bundle) | 1k–32k+ | fixed at export | Not a runtime knob; `--sliding-window` is the only mitigation |
| `nGpuLayers` | −1 (all) / 0 / n | 0 = CPU-only even with an HTP session open | `HTP0` + `ngl=0` opens a session, then runs on CPU |
| Threads | `-t 6..12`, mask `0xfc` | prefill scales; decode barely moves | Offloaded paths get 6 threads pinned to cores [2,8) |
| Power mode | `low_power_saver … burst` | burst is default, DCVS off | `sustained_high_performance` for receipts; burst = thermal cliff |
| KV dtype | f16 / q8_0 | halves cache | Forced q8_0 on pinned `HTP0`; bake-time on `qairt` |
| `--nctx` | any (GGUF only) | memory grows linearly | Silently ignored on `qairt` |

## 8. Failure modes

1. **`Q4_K_M` runs at CPU speed, silently.** *Cause:* no HTP kernel for K-quants → per-tensor fallback. *Fix:* re-quantise to `Q4_0`; verify via `GGML_HEX_VERBOSE=1` showing `Hexagon Arch version vNN`.
2. **Context length baked in.** *Symptom:* `Context length exceeded` mid-chat, `--nctx` inert. *Cause:* bundle compiled at 4096. *Fix:* `--sliding-window` or a longer-context bundle. A catalog problem, not a runtime one.
3. **Bundle/chipset mismatch → error 5000.** *Cause:* wrong SoC/Hexagon arch, or compiled on a *newer* QAIRT than the runtime. *Fix:* runtime ≥ compile SDK, arch exact (v73 ≠ v79 ≠ v81) — per SKU, not per family.
4. **Driver absent.** *Symptom:* hybrid runs at CPU speed with no error; `Device 'HTP0' not found`. *Cause:* missing `Qualcomm_HND` or unsigned HTP libs. *Fix:* install driver, confirm under Neural processors, then signing (§6).
5. **Windows ARM64 wheel scarcity.** `onnxruntime.quantization` will not install on ARM64 (`onnx` build failures); ExecuTorch AOT needs an AMD64 Python under x64 emulation; no prebuilt llama.cpp Hexagon binaries exist. *Fix:* quantise on x64, ship artifacts.
6. **Memory ceiling.** One HTP session exposes **~3.5 GB** of virtual address space (llama.cpp reports 2048 MB device memory). *Fix:* layer-split via `GGML_HEXAGON_DEVICES=HTP0:0,HTP0:1`, or stay ≤4B.
7. **Sustained thermal throttling.** *Cause:* `burst` disables DCVS by default; thin chassis plateau. *Fix:* `sustained_high_performance`, and report burst vs sustained separately — the receipt's `thermal_derate`.
8. **Startup ISA guard.** Linux needs armv8.2-a `fp16`/`dotprod`/`lse`/`rdm`; Android needs armv8.7-a `i8mm`. Older Dragonwing boards abort. *Fix:* the `--cpu-only` build, which has no NPU.

## 9. Honest assessment

NPU TOPS does not translate into decode, and Snapdragon shows why. A 100-TOPS IQ-9075 decodes 8B at 9 tok/s. TOPS describes int8 matmul on a static graph; decode emits one token at a time, reads every weight once, and round-trips over FastRPC per step. Decode tracks **bandwidth** almost linearly across the family — X Elite 135 GB/s → 9.96 tok/s on 8B; X2 Elite 228 GB/s → 19.6 tok/s, a 1.7× bandwidth ratio giving 1.97× decode — at efficiency **0.15–0.35**, a third of the same part's CPU (0.70). The rest is structural: `qairt` demands a **static, pre-compiled, per-chipset** bundle with precision, context, KV size and graph partition fixed at export — not a flag we tune, a build artifact bound to one Hexagon arch.

| Size class (4-bit) | NPU decode | Verdict |
|---|---|---|
| 0.5–3B | 12–52 tok/s | **Viable** — the only range where the NPU beats CPU with margin |
| 3–4B | 8–18 tok/s | Marginal; beats CPU only with `Q4_0` + `hybrid` |
| 7–8B laptop | 8–12 tok/s | **Loses to CPU** (24.8 tok/s on 7B Q4_0). Do not recommend |
| 13B+ | 2–9 tok/s | Mobile/IoT, batch-1, non-interactive |

The fallbacks differ. **CPU** is the honest default: 12 Oryon cores with i8mm and repacked `Q4_0` beat the NPU above 3B and need nothing signed or licensed. **Adreno via OpenCL** is worst — 18.0 vs 24.8 tok/s on the same 7B `Q4_0`, same build, plus one independent 2026 report of a 4–5× *generation* collapse. It helps prefill, hurts decode; calling it a tier would mislead.

## 10. Implications for Q-Infer

**Tiers.** X Elite / X2 Elite → **B** against curated AI Hub bundles ≤4B; arbitrary GGUF → **C** (fingerprint, advise, CPU plan); "NPU will accelerate my 70B" → **D**, with a kind refusal and the §9 table. Mobile 8 Elite → **C** (detect only) in v1; IQ-9075 → **C** until a design partner supplies hardware.

**The adapter must be bundle-shaped, not engine-shaped.** Our contract assumes `install(plan) → serve(plan)` with flags. On `qairt` there are no flags — the plan *is* the bundle selection. Add three methods: `resolve_bundle(model, chipset_id, ctx)`, `supports_context(bundle, n)`, `runtime_floor(bundle) → qairt_min_version`. A bundle that cannot satisfy the requested context is a **hard constraint failure**, not a degraded plan. `nCtx` and `nGpuLayers` must be *rejected* here: accepting parameters the SDK silently ignores is a lie.

**Catalog, per chipset not per model.** `chipset_id` (SC8380XP/SC8480XP/SM8750/SM8850/QCS9075), `hexagon_arch` (v73/v79/v81), `nsp_cores`, `bundle_id`+sha256, `precision`, `context_len_baked`, `kv_bytes_baked`, `compile_qairt_version`, `runtime_min`/`max`, `measured_tok_s` **with the harness named**, and `datasheet_claim`, which we never predict from.

**Planner rules.** (a) Context is an equality constraint, not a slider. (b) Every NPU plan carries a CPU plan as fallback #1 — here the fallback is often faster. (c) Predict with η ≤ 0.35 for Hexagon, never the 0.60–0.85 band for LPDDR5X CPU/GPU parts. (d) Rank `hybrid > npu > cpu > gpu` for GGUF, and verify. (e) No MoE decode prediction — zero Snapdragon data.

**What we cannot automate.** Qualcomm ID + QAIRT licence activation; an AI Hub account for bundle export; NPU/GPU driver install (admin, reboot); Windows test-signing, Secure Boot changes, certificate import; Hexagon + OpenCL SDK acquisition. Each is a privileged step the user runs, recorded in the receipt, never implicit.

## 11. Sources

All accessed 2026-10-26.

1. Qualcomm, Snapdragon X Elite product brief. https://www.qualcomm.com/content/dam/qcomm-martech/dm-assets/documents/Product-Brief-Snapdragon-X-Elite.pdf
2. Qualcomm, Snapdragon X Series comparison table. https://www.qualcomm.com/laptops/products
3. Qualcomm, Snapdragon 8 Elite / 8 Elite Gen 5 product briefs. https://docs.qualcomm.com/doc/87-83196-1/
4. Qualcomm, Dragonwing IQ-9075 and QCS9075 data sheet. https://docs.qualcomm.com/doc/80-73417-1/
5. Google LiteRT, `supported_soc.csv` (SoC→Hexagon arch). https://github.com/google-ai-edge/litert/blob/main/litert/vendors/qualcomm/supported_soc.csv
6. Qualcomm, QAIRT SDK supported devices (V68–V81). https://docs.qualcomm.com/doc/80-63442-10/topic/SNPE_general_overview.html
7. Qualcomm, GenieX — Platforms & runtimes. https://geniex.aihub.qualcomm.com/en/get-started/platforms
8. Qualcomm, GenieX — Models & precisions. https://geniex.aihub.qualcomm.com/en/models/supported
9. Qualcomm, GenieX — geniex-bench tutorial. https://geniex.aihub.qualcomm.com/en/tutorials/benchmarking
10. Qualcomm, `GenieX/notes/run.md`. https://github.com/qualcomm/GenieX/blob/main/notes/run.md
11. Qualcomm, GenieX troubleshooting. https://geniex.aihub.qualcomm.com/en/resources/troubleshooting
12. Qualcomm AI Hub model cards (Llama-v3.1-8B, Llama-v3.2-3B). https://huggingface.co/qualcomm/Llama-v3.1-8B-Instruct
13. Qualcomm, Prepare a GenAI model using AI Hub. https://docs.qualcomm.com/doc/80-80022-15B/topic/genie-prepare-ai-hub.html
14. Qualcomm, Genie Dialog JSON configuration. https://docs.qualcomm.com/doc/80-63442-10/topic/dialog_json.html
15. Qualcomm, Install the QAIRT SDK. https://dragonwingdocs.qualcomm.com/Key-Documents/AI-Developer-Workflow/topic/qairt-install
16. Qualcomm, QAIRT SDK legal notice. https://docs.qualcomm.com/bundle/publicresource/topics/80-63442-10/Legalnotice.html
17. llama.cpp, Snapdragon backend README. https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/snapdragon/README.md
18. llama.cpp, Windows on Snapdragon guide. https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/snapdragon/windows.md
19. llama.cpp Discussion #8336 (X Elite CPU/OpenCL benchmarks, Jul 2024). https://github.com/ggml-org/llama.cpp/discussions/8336
20. ONNX Runtime, QNN Execution Provider docs. https://onnxruntime.ai/docs/execution-providers/QNN-ExecutionProvider.html
21. Microsoft, Windows ML execution providers. https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers
22. PyTorch, ExecuTorch Qualcomm backend docs. https://docs.pytorch.org/executorch/stable/backends-qualcomm.html
23. qualcomm/ai-hub-models issue #164 (2.29 floor, error 5000). https://github.com/qualcomm/ai-hub-models/issues/164
24. S. Joffraud, Adreno OpenCL decode (Oct 2026). https://blog.l4w.fr/posts/llama-cpp-snapdragon-gpu-adreno-opencl-vs-cpu/
