# Development Changelog

Append-only. Newest last. Commit refs are recorded where they exist.

## 2026-10-26

### v0.3 — the dashboard (ADR-0009)
- Ratatui dashboard in the same static binary; read-only view over `cfp@1` and results.
- `--snapshot WxH [--tab]` renders offscreen for CI and review.
- Layout breakpoints: two-column ≥76 cols, stacked below; bench sheds non-essential columns <84.
- PRD FR-55…FR-59 added (59 total). SDD §4.4 (TUI↔core contract) added. M0 gains D0.9 + AT-0.11/AT-0.12.

### v0.2 — evidence pass
Three platform dossiers and an M0 spike forced six corrections:
- `η` → `k_cal`; no longer bounded by 1.0 (measured 1.175).
- Bandwidth probe must saturate (131 → 228 GB/s, 1.74×).
- `t₀` is per model class (1.22 ms dense Metal vs ~8.4 ms MoE Ultra).
- NPU efficiency 0.15–0.35; CPU beats NPU above ~4B on Snapdragon.
- `sm_121` story moved from packaging to `tcgen05`/TMEM architecture.
- Q1–Q10 given provisional decisions; Q11–Q14 opened.

Spike result: two-parameter model **1.45 % MAPE** vs 15.6 % naive, R² 0.9996.

### v0.1 — initial PRD
- Product framing, six failure walls, personas, 54 requirements, platform tiers.
- SDD, 9 ADRs, 4 milestone specs (36 acceptance tests).

### Repo
- Rust workspace: `qinfer-core` (types + invariants), `qinfer-tui` (dashboard).
- 11 atomic baseline commits; protocol bootstrap on `chore/protocol-bootstrap`.

### M0 implementation
- **T-001** `cfp@1` validation — 5 invariants, 9 tests. PR **#1**.
- **T-002** bandwidth probe — thread sweep, 4x LLC floor, 8 tests. PR **#2**, stacked on #1.
- Rust probe reproduces the Python finding: 126.9 -> 259.0 GB/s (2.04x) at 15 threads.
- AOP v5 adopted; `instruction.md` v5 reconstructed (DRAFT), `init-charter.md` ACTIVE.

### M0 core merged to `main`
Six PRs landed (#1–#6) after three Critic rounds on T-001:
- `BLOCK` → 8 required findings; `BLOCK` again → 14 new findings incl. an unr corroborated headline that would have produced a 10× over-prediction, and a TUI panic at 10–11 columns.
- `PASS-WITH-CHANGES` → fail-closed `headline_method`, three-valued pressure, sealed `Validated`.
- `main` at **73 tests + 1 doctest**, build clean.

Tasks: T-001 DONE; T-002/T-003/T-005 merged and awaiting independent verification; T-004 and T-006 next.

## 2026-10-27

### v0.4 — measurement layer (T-012, T-013)

The planner was predicting to ~2% MAPE and recommending the wrong thing, because
two of its inputs were guesses. Both are now measured.

- **T-012 `space`.** The memory budget was half of RAM — 12.0 GiB on the 24 GiB
  M5 Pro — which **rejected `qwen3.8-27b-4bit-mlx` at 14.98 GiB**, an artifact in
  this machine's own HF cache. Resolved instead from Metal's
  `recommendedMaxWorkingSetSize`: **17.76 GiB (1.48x)**. Provenance demotes to
  `unknown` on memory pressure, because a paging machine reports a healthy
  working set while nothing measured on it is trustworthy.
- **T-013 `microops`.** A `python3`/MLX probe run as a subprocess (ADR-0001):
  GPU stream bandwidth, quantised GEMV at a size above LLC, batched bf16 matmul,
  and ANE reachability. Every op is `Measured{method, reps, spread}` or
  `Refused{reason, fix}` — never `0.0`.

Measured on the reference M5 Pro:

| op | value | spread |
|---|---|---|
| CPU probe, saturated | 257.4 GB/s | 36.9% (machine noisy) |
| GPU stream (`add_fp32@256MiB`) | 257.9 GB/s | 20.7% |
| GEMV q4 (`16384x32768`) | 256.4 GB/s | 1.7% |
| GEMM bf16 (`1024x4096x4096`) | 23.1 TFLOP/s | 5.5% |
| ANE | unreachable | — |

**The result that matters:** all three bandwidth paths converge on ~257 GB/s, so
`k_cal` fell from **1.175 to 1.031**. The original >1 coefficient was an artefact
of an under-saturated CPU probe (228 GB/s), not evidence that the GPU draws more
bandwidth than a CPU copy can demonstrate. PRD Q11 is answered: probe the device.

Two corrections this forced on the docs' own claims:
- The M5 Pro's effective bandwidth is ~257 GB/s, not the 307 GB/s the M5 family
  table implies (0.84 of declared, the normal Apple gap).
- GEMV sized below LLC over-reports by 5x (51.9 GB/s at 9 MiB vs 250.3 at 288
  MiB). The 4x-LLC floor is a rule for *every* micro-benchmark, not just the copy
  probe — it is now enforced inside the op.

Tests: **199 pass** (155 lib, 11 `space_contract`, 23 `verification_m0`, 9
workspace, 1 doctest). `plan` is unchanged; `measure` is the new end-to-end
example (`probe -> microops -> fingerprint -> space -> session -> plans`).

### v0.5 — the sweet spot (T-014)

The planner is now accurate and still recommends the wrong thing. `Planner`
ranks by throughput, so on a bandwidth-bound machine the winner is always the
smallest artifact — a 0.5B at 339 tok/s. A quality floor does not fix it, because
`quality_placeholder` keys on quantisation and not on model size: a 0.5B Q4 and a
27B Q4 both score 0.93.

`sweetspot` adds the missing axis. The question becomes a curve, and the answer is
its knee — Kneedle's maximum-distance-from-chord, so no weighting constant has to
be defended. Capability is a **parameter-count prior** (total, never active, or a
30B-A3B would rank below a dense 14B) and is labelled as such everywhere.

Live on the M5 Pro:

```
  Qwen3.8 27B (MLX 4-bit)       16 tok/s  cap 1.00   16.7 GiB  prior (params)
  OLMoE 1B-7B (Q4_K_M)         245 tok/s  cap 0.66    4.6 GiB  prior (params)  <- knee
  Qwen2.5 0.5B (Q4_K_M)        339 tok/s  cap 0.00    0.5 GiB  prior (params)
```

The three 3B variants are correctly dominated (OLMoE is both faster and more
capable), so the frontier drops them. The knee lands on a 2024 model — which is
the catalogue's fault, not the algorithm's, and is what makes the next step
urgent: no modern coding model is plannable yet.

Tests: **210 pass** (166 lib, 11 `space_contract`, 23 `verification_m0`, 9
workspace, 1 doctest).

### v0.6 — a recommendation worth showing (T-015, T-016, T-017)

**The catalogue grew to 20 entries.** Six coding artifacts added with
publisher-declared sizes from the Hub manifest and KV geometry read from each
`config.json`. They are `bytes_source: "publisher"`, not `"measured"` — we did
not download and hash them, and `catalog.rs` requires a hash for a measured
claim, so the label tells the truth. Seven count-based tests were rewritten to
assert properties instead, because a test that pins "sixteen entries" turns
every catalogue update into a red build without protecting anything.

| model | size | predicted |
|---|---|---|
| Qwen3 Coder 30B-A3B (MLX 4-bit) | 16.00 GiB | 123 tok/s |
| gpt-oss-20b (MXFP4-Q8) | 11.25 GiB | 112 tok/s |
| Qwen2.5 Coder 7B (Q4_K_M) | 4.36 GiB | 53 tok/s |
| Qwen2.5 Coder 14B (MLX 4-bit) | 7.74 GiB | 31 tok/s |

The 30B-A3B reads 1.86 GB/token because only 3.3B of its 30.5B parameters are
active — 30B-class coding on a 24 GiB laptop. It leaves **0.56 GiB of headroom**,
which caps context at roughly **10k** tokens, not the model's 262k. That caveat
belongs on the surface, not in a footnote.

**T-015 — the floor.** The knee is a mathematical property of a curve; the user
asked a different question. Past ~20 tok/s extra speed is worth nothing for an
assistant, so the recommendation is now *most capable above the floor*:

```
knee    OLMoE 1B-7B (Q4_K_M) is the knee: 245 tok/s at 4.6 GiB ...
usable  Qwen3 Coder 30B-A3B (MLX 4-bit) is the most capable plan still above
        20 tok/s: 123 tok/s at 17.2 GiB — the faster Qwen2.5 0.5B at 339 tok/s
        is less capable.
```

**T-016 — the ladder.** `setup.rs` runs Gauge → Identify → Budget → Shortlist →
Choose, emitting `Begin`/`Stage`/`Failed` so a slow rung is visible *while* it is
slow. The order is the data dependency, not the PRD diagram: a fingerprint
contains the bandwidth result, so it cannot be collected first. Every expensive
step sits behind a `Measurements` trait, so the whole ladder is tested on a
machine with no GPU and no macOS. M0 is an advisor with no installer, so the
ladder ends at `Choose`.

**T-017 — the dashboard.** New `Frontier` tab, and the TUI no longer budgets
against half of RAM. Column shedding follows FR-59: the provenance column goes
below 84 columns, capability below 64, then the name truncates. Verified at
100, 80, 64 and 46 columns with no row wrapping — the first attempt did wrap,
because the width budget ignored the row marker and `Wrap { trim: true }` was
eating the leading spaces that carry it.

The TUI had no test suite at all, which is why its narrow-width behaviour was
assumed rather than known. `crates/qinfer-tui/tests/snapshot.rs` now renders
every tab at 100x30, 80x24, 64x20 and 46x16 and asserts the frame is exactly the
requested size. **It immediately caught a real defect**: `render_plans` computed
`inner.width as usize - 8`, which underflows below ten columns and panicked
before drawing anything (`qinfer --snapshot 9x20 --tab plans` exited 101). The
test was written first and left `#[ignore]`d rather than weakened, so the bug was
fixed in the renderer rather than absorbed by the assertion.

Also fixed: `multi_thread_finds_more_bandwidth_than_one_thread` was the flakiest
test in the suite and the *assertion* was wrong, not the machine. It demanded the
threaded sweep beat single-threaded by 10%, which a 32 MiB debug-build copy under
concurrent load does not. It now asserts the property that is always true
(threading must not make bandwidth worse) and leaves the size of the win to
`spread_pct`, which is published in the fingerprint where it can be judged.

Tests: **231 pass, nothing ignored, three consecutive clean runs.**

### v0.7 — the dashboard becomes the entry point (T-018)

`Enter` now runs the ladder. The Setup tab is the fifth rung made reachable: five
rungs, filled in as events arrive from a worker thread, with the explicit empty
state that says *nothing here is a measurement yet* rather than showing numbers
from whoever last ran `fingerprint.py`.

The ladder hands its fingerprint over in the event stream rather than writing it,
so a *view* never writes (ADR-0009); the dashboard prefers that live fingerprint
over the one on disk, which is the whole reason to press Enter.

Live, on the reference machine:

```
     ok Gauge     CPU 263 GB/s saturated (2.0x single-thread) · 266 GB/s device stream
     ok Budget    17.8 GiB of 24.0 GiB usable (recommendedMaxWorkingSetSize)
     ok Shortlist 11 plan(s) from 20 catalogued artifact(s) · 9 excluded
     ok Choose    Qwen3 Coder 30B-A3B (MLX 4-bit) is the most capable plan still
                  above 20 tok/s: 123 tok/s at 17.2 GiB
```

**Three layout defects found by looking at the render**, all of the same kind —
a fixed size that starved something else:

1. The body constraint was `Min(10)`, so at 46x12 ratatui satisfied the minimum
   by starving the header: the machine name and tab bar vanished entirely, and
   the frame still exited zero. Body is now `Min(3)`; `the_header_survives_a_short_terminal`
   asserts it.
2. The title line was clipped mid-word at 46 columns. It now truncates, and drops
   the certification tier before the machine name.
3. The tab bar clipped its last tab at every width below 88, because the `Tabs`
   widget pads labels invisibly and the width was being guessed. The bar is now a
   plain `Line` with exact arithmetic, shedding full names to abbreviations to
   digits.

Tests: **232 pass.**

Also fixed in passing: the micro-op spread estimator was `(max-min)/max`, which
let one descheduled sample condemn a good measurement (67% spread on a stable
3 ms op). Now interquartile about the median, with a discarded warm-up — GEMV
went from 70.5% to 1.7% spread with no change in the value.
