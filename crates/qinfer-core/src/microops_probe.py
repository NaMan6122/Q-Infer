"""Q-Infer micro-operation probe.

Executed by `microops::run` as `python3 -c <this file> <comma-separated keys>`.
It is never imported by the crate (ADR-0001: engines are subprocesses).

Contract
--------
Print exactly one JSON object on stdout. Keys are `OpId::key()` values for the
ops that succeeded, plus an optional "device" object carrying MLX's device_info
payload. **Ops that could not run are omitted entirely.** The Rust side turns an
omitted key into `Refused{reason}`; the script must never emit a zero as a
stand-in for a failed measurement, because a zero bandwidth once produced a
published 0.48 tok/s "prediction".

Why every op is sized the way it is
-----------------------------------
A micro-benchmark smaller than last-level cache measures cache. On the reference
M5 Pro the same GEMV reported 51.9 GB/s at 9 MiB of weights, 208.8 GB/s at
144 MiB, and 250.3 GB/s at 288 MiB. The probe therefore refuses to run small.
"""

import json
import sys
import time

REPS = 9
SPREAD_BUDGET = 15.0


def _timeit(fn, reps=REPS):
    """Best-of-N wall time. MLX is lazy, so every call must be synchronised by
    evaluating its output — timing the graph construction instead of the work is
    how a benchmark reports 1,135,883 GB/s.

    The first call is discarded: it pays kernel compilation and first touch, and
    on the reference machine that single sample was enough to make a stable
    3 ms operation look like it had 67% spread."""
    out = fn()
    if out is not None:
        mx.eval(out)

    samples = []
    for _ in range(reps):
        t0 = time.perf_counter()
        out = fn()
        if out is not None:
            mx.eval(out)
        samples.append(time.perf_counter() - t0)
    return min(samples), _spread_pct(samples)


def _spread_pct(samples):
    """Interquartile spread about the median.

    Not (max - min) / max: one descheduled sample on a busy laptop would then
    condemn a perfectly good measurement, and the `cfp@1` rule is that a spread
    above 15% demotes the figure to `unknown`. The estimator has to be robust
    before the rule can be trusted."""
    s = sorted(samples)
    if len(s) < 4:
        return 0.0
    med = s[len(s) // 2]
    if med <= 0:
        return 0.0
    lo = s[len(s) // 4]
    hi = s[(3 * len(s)) // 4]
    return (hi - lo) / med * 100.0


def device_info():
    info = None
    try:
        info = mx.device_info()
    except Exception:
        # MLX renamed this; the old path is deprecated but still present.
        try:
            info = mx.metal.device_info()
        except Exception:
            return None
    keep = (
        "device_name",
        "architecture",
        "memory_size",
        "max_recommended_working_set_size",
        "max_buffer_length",
    )
    out = {k: info[k] for k in keep if k in info}
    return out or None


def gpu_bandwidth_gbs():
    """c = a + b over 256 MiB fp32 operands: two reads and one write per element,
    the same accounting the CPU `copy` probe uses."""
    n = 64 * 1024 * 1024
    a = mx.ones((n,), dtype=mx.float32)
    b = mx.ones((n,), dtype=mx.float32)
    mx.eval(a, b)
    best, spread = _timeit(lambda: mx.add(a, b))
    return {
        "value": round(n * 4 * 3 / best / 1e9, 2),
        "unit": "GB/s",
        "reps": REPS,
        "spread_pct": round(spread, 2),
        "method": "add_fp32@256MiB",
    }


def gemv_q4_gbs():
    """Quantised matvec at batch 1 — the decode shape. Sized so the weight stream
    exceeds last-level cache; a cache-resident matrix over-reports by 5x."""
    bits, group = 4, 64
    for rows, cols in ((16384, 32768), (8192, 8192)):
        try:
            w = mx.random.normal((rows, cols)).astype(mx.float16)
            wq, scales, biases = mx.quantize(w, group_size=group, bits=bits)
            v = mx.random.normal((1, cols)).astype(mx.float16)
            mx.eval(wq, scales, biases, v)
            del w
            nbytes = wq.nbytes + scales.nbytes + biases.nbytes
            if nbytes < 128 * 1024 * 1024:
                return None  # would measure cache
            best, spread = _timeit(
                lambda: mx.quantized_matmul(
                    v, wq, scales, biases, transpose=True, group_size=group, bits=bits
                )
            )
            return {
                "value": round(nbytes / best / 1e9, 2),
                "unit": "GB/s",
                "reps": REPS,
                "spread_pct": round(spread, 2),
                "method": f"quantized_matmul_q{bits}@{rows}x{cols}",
            }
        except Exception:
            continue
    return None


def gemm_bf16_tflops():
    """Batched matmul in bf16 — the prefill/TTFT shape. This is where the M5 GPU
    neural accelerators are supposed to show up, so it is the number that has to
    be measured rather than believed."""
    dims = ((1024, 4096, 4096), (512, 2048, 2048))
    for m, k, n in dims:
        try:
            x = mx.random.normal((m, k)).astype(mx.bfloat16)
            w = mx.random.normal((k, n)).astype(mx.bfloat16)
            mx.eval(x, w)
            best, spread = _timeit(lambda: mx.matmul(x, w))
            return {
                "value": round(2 * m * k * n / best / 1e12, 2),
                "unit": "TFLOP/s",
                "reps": REPS,
                "spread_pct": round(spread, 2),
                "method": f"matmul_bf16@{m}x{k}x{n}",
            }
        except Exception:
            continue
    return None


def ane_reachable():
    """Is the Neural Engine reachable for this workload?

    The honest answer on Apple silicon today is usually "no, and here is why":
    neither MLX nor llama.cpp routes LLM decode through the ANE, because CoreML
    requires static shapes and a decode loop's KV cache grows every token.

    Unreachable returns ``None``, so the op is *omitted* and the Rust side turns
    it into an explicit refusal with a reason and a fix. Emitting ``0.0`` would
    put a fabricated zero on the same axis as a real measurement, and a zero
    bandwidth once produced a published 0.48 tok/s "prediction".
    """
    try:
        import coremltools  # noqa: F401
    except Exception:
        return None
    return {
        "value": 1.0,
        "unit": "bool",
        "reps": 1,
        "spread_pct": 0.0,
        "method": "import coremltools ok",
        "note": "CoreML present; ANE use for LLM decode is still unproven and must be measured",
    }


OPS = {
    "gpu_bandwidth_gbs": gpu_bandwidth_gbs,
    "gemv_q4_gbs": gemv_q4_gbs,
    "gemm_bf16_tflops": gemm_bf16_tflops,
    "ane_reachable": ane_reachable,
}


def main():
    wanted = [k for k in (sys.argv[1] if len(sys.argv) > 1 else "").split(",") if k]
    out = {}

    dev = device_info()
    if dev is not None:
        out["device"] = dev

    for key in wanted:
        fn = OPS.get(key)
        if fn is None:
            continue
        try:
            result = fn()
        except Exception:
            result = None
        if result is not None:
            out[key] = result

    print(json.dumps(out))


try:
    import mlx.core as mx  # noqa: F401
except Exception:
    # Without MLX there is nothing to measure. Emit an empty object; every op
    # becomes `Refused` on the Rust side, which is the correct outcome.
    print(json.dumps({}))
    raise SystemExit(0)

if __name__ == "__main__":
    main()
