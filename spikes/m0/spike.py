#!/usr/bin/env python3
"""
M0 spike — falsify (or confirm) the PRD's §5.2 predictor on real hardware.

The claim under test:
    t_token ≈ t0 + bytes_per_token / BW_eff
    decode_tok_s ≈ 1 / t_token

Method:
  1. Read the measured bandwidth from fingerprint.json (never a spec sheet).
  2. Run llama-bench (Metal, full offload) for each model, N reps.
  3. Compare naive (BW/bytes) vs two-parameter predictions against measurement.
  4. Back out the implied effective decode bandwidth per model, and fit t0.

If the naive model wins, the PRD's core thesis is wrong and §5.2 must be revised.
If the two-parameter model wins, the spike has earned it.
"""

from __future__ import annotations

import json
import os
import statistics
import subprocess
import sys
from dataclasses import dataclass

HERE = os.path.dirname(os.path.abspath(__file__))
REPS = 5


@dataclass
class Measurement:
    model: str
    file_bytes: int
    n_params: int
    pp_tps: float
    tg_tps: float
    tg_stddev: float
    backend: str
    build: str


def run_bench(model_path: str) -> Measurement:
    cmd = [
        "llama-bench", "-m", model_path,
        "-p", "512", "-n", "128", "-r", str(REPS), "-ngl", "99", "-o", "json",
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
    if proc.returncode != 0:
        raise RuntimeError(f"llama-bench failed: {proc.stderr[-500:]}")
    rows = json.loads(proc.stdout)

    pp = next((r for r in rows if r["n_gen"] == 0), None)
    tg = next((r for r in rows if r["n_gen"] > 0), None)
    if not tg:
        raise RuntimeError("no decode row in llama-bench output")

    return Measurement(
        model=os.path.basename(model_path),
        file_bytes=tg["model_size"],
        n_params=tg["model_n_params"],
        pp_tps=pp["avg_ts"] if pp else float("nan"),
        tg_tps=tg["avg_ts"],
        tg_stddev=tg["stddev_ts"],
        backend=tg["backends"],
        build=f'{tg["build_number"]}@{tg["build_commit"][:8]}',
    )


def predict_naive(file_bytes: int, bw_gbs: float) -> float:
    """The model the PRD argues is insufficient: tok/s = BW / bytes_per_token."""
    seconds_per_token = file_bytes / (bw_gbs * 1e9)
    return 1.0 / seconds_per_token


def predict_two_param(file_bytes: int, bw_gbs: float, t0_ms: float, eps: float = 1.0) -> float:
    seconds_per_token = t0_ms / 1000.0 + file_bytes / (eps * bw_gbs * 1e9)
    return 1.0 / seconds_per_token


def implied_effective_bw_gbs(file_bytes: int, tok_s: float) -> float:
    """Back out the bandwidth the machine actually delivered during decode."""
    return file_bytes * tok_s / 1e9


def fit_two_param(measurements: list[Measurement], bw_gbs: float) -> tuple[float, float, float]:
    """Fit t_token = t0 + W/(eps*BW) over all models.

    Linear regression of t on W gives intercept t0 and slope 1/(eps*BW), so both
    parameters come out of the same fit. With fewer than 3 points this is
    underdetermined (2 points fit exactly, R^2=1, and prove nothing) -- the
    caller must check `determined` before believing it.
    """
    xs = [m.file_bytes for m in measurements]
    ys = [1.0 / m.tg_tps for m in measurements]
    n = len(xs)
    mx, my = statistics.fmean(xs), statistics.fmean(ys)
    den = sum((x - mx) ** 2 for x in xs)
    if den == 0:
        return 0.0, 1.0, 0.0
    slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den
    intercept = my - slope * mx
    eps = 1.0 / (slope * bw_gbs * 1e9) if slope > 0 else float("nan")

    ss_res = sum((y - (intercept + slope * x)) ** 2 for x, y in zip(xs, ys))
    ss_tot = sum((y - my) ** 2 for y in ys)
    r2 = 1.0 - ss_res / ss_tot if ss_tot else 1.0
    return intercept * 1000.0, eps, r2


def main() -> int:
    fp = json.load(open(os.path.join(HERE, "fingerprint.json"), encoding="utf-8"))
    bw = fp["bandwidth"]["headline_gbs"]

    models = [
        os.path.join(HERE, "models", f)
        for f in sorted(os.listdir(os.path.join(HERE, "models")))
        if f.endswith(".gguf")
    ]
    if not models:
        print("no models found in spikes/m0/models", file=sys.stderr)
        return 2

    results = [run_bench(m) for m in models]

    t0_fit, eps_fit, r2 = fit_two_param(results, bw)
    determined = len(results) >= 3

    print(f"measured memcpy bandwidth : {bw:.1f} GB/s")
    print(f"fitted t0 / eps / R^2     : {t0_fit:.2f} ms  {eps_fit:.3f}  {r2:.4f}"
          f"{'' if determined else '   [UNDERDETERMINED: need >=3 models]'}\n")

    header = (f"{'model':<40}{'GiB':>7}{'measured':>10}{'naive':>9}{'err%':>8}"
              f"{'2-param':>9}{'err%':>8}{'impl eps':>10}")
    print(header)
    print("-" * len(header))
    rows = []
    for m in results:
        naive = predict_naive(m.file_bytes, bw)
        two = predict_two_param(m.file_bytes, bw, t0_fit, eps_fit)
        eff = implied_effective_bw_gbs(m.file_bytes, m.tg_tps)
        e_naive = (naive - m.tg_tps) / m.tg_tps * 100
        e_two = (two - m.tg_tps) / m.tg_tps * 100
        print(f"{m.model:<40}{m.file_bytes / 2**30:>7.2f}{m.tg_tps:>10.1f}"
              f"{naive:>9.1f}{e_naive:>+8.1f}{two:>9.1f}{e_two:>+8.1f}{eff / bw:>10.2f}")
        rows.append({
            "model": m.model, "gib": round(m.file_bytes / 2**30, 2),
            "measured_tps": round(m.tg_tps, 2), "pp_tps": round(m.pp_tps, 1),
            "naive_tps": round(naive, 2), "naive_err_pct": round(e_naive, 2),
            "two_param_tps": round(two, 2), "two_param_err_pct": round(e_two, 2),
            "implied_effective_gbs": round(eff, 2), "implied_eps": round(eff / bw, 3),
            "build": m.build, "backend": m.backend,
        })

    naive_mape = statistics.fmean(abs(r["naive_err_pct"]) for r in rows)
    two_mape = statistics.fmean(abs(r["two_param_err_pct"]) for r in rows)
    print(f"\nMAPE  naive {naive_mape:.1f}%   two-param {two_mape:.1f}%")

    out = {
        "bandwidth_gbs": bw,
        "bandwidth_single_thread_gbs": fp["bandwidth"].get("single_thread_gbs"),
        "reps": REPS,
        "fit": {"t0_ms": round(t0_fit, 3), "eps": round(eps_fit, 3),
                "r2": round(r2, 4), "determined": determined, "n_models": len(results)},
        "mape": {"naive_pct": round(naive_mape, 2), "two_param_pct": round(two_mape, 2)},
        "rows": rows,
    }
    with open(os.path.join(HERE, "results.json"), "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=2)
    print(f"written {os.path.join(HERE, 'results.json')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
