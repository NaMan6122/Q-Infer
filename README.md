# Q-Infer

**The right engine, on the right silicon, at the right settings — decided, installed and proven.**

A bare-metal inference provisioning and tuning toolkit. Point it at a machine and a model; it fingerprints the silicon, predicts what that machine can sustain, chooses between competing inference frameworks, installs a pinned and reversible configuration, benchmarks it, and hands back a signed receipt.

> Status: **pre-implementation.** The design is evidence-backed and the predictor has been validated on one machine. M0 has not shipped.

## Where things are

| Path | What |
|---|---|
| [`PRD.md`](PRD.md) | Product requirements — problem, thesis, 59 requirements, platform tiers, risks |
| [`SDD.md`](SDD.md) | System design — components, data contracts, interfaces, error taxonomy, test strategy |
| [`docs/adr/`](docs/adr/) | 9 architecture decisions with alternatives and review triggers |
| [`docs/milestones/`](docs/milestones/) | M0–M3 with 36 acceptance tests and exit gates |
| [`docs/research/`](docs/research/) | Platform dossiers: Apple Silicon, NVIDIA GB10, Qualcomm Snapdragon |
| [`spikes/m0/`](spikes/m0/) | The predictor, tested on real hardware — `REPORT.md` has the numbers |
| `crates/` | Rust workspace: `qinfer-core` (types) and `qinfer-tui` (dashboard) |

## The idea in one paragraph

Decode throughput is a memory-bandwidth problem. A dense model reads every active parameter once per token, so `tok/s ≈ bandwidth / bytes_per_token`. That is cheap enough to compute before downloading 40 GB of weights, which turns framework selection from a craft into a search problem. It needs two corrections — a fixed per-token cost `t₀`, and the fact that the bandwidth a GPU achieves is not the bandwidth a CPU probe demonstrates — and then it predicts well enough to *rank* candidate plans, which is the actual product requirement.

Validated at **1.45 % MAPE** against 15.6 % for the naive roofline, on three models, one Apple M5 Pro. Not yet validated for MoE, CUDA, or batching.

## Try the dashboard

```bash
# generate a fingerprint and a benchmark result (requires llama.cpp + a GGUF)
python3 spikes/m0/fingerprint.py
python3 spikes/m0/spike.py

# open the dashboard
cargo run -q -p qinfer-tui

# or render one frame headlessly
cargo run -q -p qinfer-tui -- --snapshot 100x30 --tab bench

# build once and link `qinfer` onto your PATH (symlink, so rebuilds are live)
./build.sh
qinfer                          # dashboard; `qinfer help` for the CLI
```

## Try the evidence

```bash
cargo test                      # core invariants
python3 spikes/m0/spike.py      # re-run the prediction experiment
```

## What this is not

Not a chat UI, not an inference engine, not a trainer, not a cluster orchestrator. It is the layer that decides *which* runtime, *which* quantisation and *which* flags are correct for this specific box — and then proves it. See `PRD.md` §3.2.
