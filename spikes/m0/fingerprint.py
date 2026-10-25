#!/usr/bin/env python3
"""
M0 spike — Capability Fingerprint for Apple Silicon.

Purpose: prove that the fields the PRD's planner needs are (a) obtainable
without root, and (b) accurate enough to drive a bandwidth-based prediction.

This is deliberately a spike, not the product: it collects only what the M0
question requires, and it records provenance so we can see which numbers we
trust and which we inferred.
"""

from __future__ import annotations

import json
import os
import platform
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field, asdict
from typing import Any

import numpy as np

SCHEMA = "qinfer/cfp-spike@1"


# --------------------------------------------------------------------------- #
# shell helpers
# --------------------------------------------------------------------------- #

def sh(argv: list[str], timeout: float = 20.0) -> str | None:
    """Run a command, return stripped stdout, or None if it fails."""
    try:
        out = subprocess.run(
            argv, capture_output=True, text=True, timeout=timeout, check=False
        )
        return out.stdout.strip() if out.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def sysctl(key: str) -> str | None:
    return sh(["sysctl", "-n", key])


def sysctl_int(key: str) -> int | None:
    raw = sysctl(key)
    if not raw:
        return None
    try:
        return int(raw)
    except ValueError:
        return None


# --------------------------------------------------------------------------- #
# identity
# --------------------------------------------------------------------------- #

def collect_identity() -> dict[str, Any]:
    return {
        "os": "macos" if sys.platform == "darwin" else sys.platform,
        "os_version": sh(["sw_vers", "-productVersion"]),
        "os_build": sh(["sw_vers", "-buildVersion"]),
        "arch": platform.machine(),
        "kernel": platform.release(),
        "hostname": platform.node(),
    }


def collect_cpu() -> dict[str, Any]:
    # NOTE: every one of these keys is Optional on some Mac. `hw.l3cachesize`
    # does not exist on Apple silicon at all -- calling int() on the empty
    # result is a classic fingerprint-layer bug.
    brand = sysctl("machdep.cpu.brand_string")
    physical = sysctl_int("hw.physicalcpu")
    logical = sysctl_int("hw.logicalcpu")
    nperf = sysctl_int("hw.nperflevels")

    perf_levels = []
    if nperf:
        for i in range(nperf):
            perf_levels.append({
                "name": sysctl(f"hw.perflevel{i}.name"),
                "physical_cores": sysctl_int(f"hw.perflevel{i}.physicalcpu"),
                "l1d_bytes": sysctl_int(f"hw.perflevel{i}.l1dcachesize"),
                "l2_bytes": sysctl_int(f"hw.perflevel{i}.l2cachesize"),
            })

    # ISA features. Key casing is inconsistent by design: `FEAT_DotProd` is
    # mixed case while `FEAT_I8MM` is all caps. `FEAT_SVE`/`FEAT_SVE2` never
    # exist on Apple silicon -- absence means "not architectural SVE", not error.
    feat_keys = [
        "hw.optional.arm.FEAT_DotProd",
        "hw.optional.arm.FEAT_I8MM",
        "hw.optional.arm.FEAT_BF16",
        "hw.optional.arm.FEAT_SME",
        "hw.optional.arm.FEAT_SME2",
        "hw.optional.arm.FEAT_SME2p1",
        "hw.optional.arm.FEAT_FP8",
    ]
    isa = {k.rsplit(".", 1)[-1]: (sysctl_int(k) or 0) for k in feat_keys}
    isa["SVE"] = 0  # not exposed; Apple implements SME, not architectural SVE
    isa["SVE2"] = 0

    return {
        "brand": brand,
        "physical_cores": physical,
        "logical_cores": logical,
        "perf_levels": perf_levels,
        "isa": isa,
        # Explicitly absent on Apple silicon -- recorded so downstream code
        # never confuses "missing" with "zero".
        "l3_bytes": None,
        "max_freq_mhz": None,
    }


def collect_gpu() -> dict[str, Any]:
    raw = sh(["system_profiler", "SPDisplaysDataType", "-json"], timeout=30)
    gpu: dict[str, Any] = {}
    if raw:
        try:
            data = json.loads(raw)
            entries = data.get("SPDisplaysDataType", [])
            if entries:
                e = entries[0]
                gpu = {
                    "model": e.get("sppci_model"),
                    # NOTE: this is a *string* in the plist, e.g. "16".
                    "cores": int(e["sppci_cores"]) if str(e.get("sppci_cores", "")).isdigit() else None,
                    "metal_support": e.get("spdisplays_mtlgpufamilysupport"),
                    "vendor": e.get("sppci_vendor"),
                }
        except (json.JSONDecodeError, KeyError, TypeError):
            gpu = {"parse_error": True}

    # recommendedMaxWorkingSetSize is the real budget the GPU will hand out;
    # it is ~0.74 x RAM on this class of machine and is NOT documented.
    gpu["unified_memory"] = True
    return gpu


def collect_memory() -> dict[str, Any]:
    total = sysctl_int("hw.memsize")
    swap_raw = sysctl("vm.swapusage")  # human string, not JSON
    swap_used = None
    if swap_raw:
        m = re.search(r"used\s*=\s*([\d.]+)M", swap_raw)
        if m:
            swap_used = int(float(m.group(1)) * 1024 * 1024)
    return {
        "total_bytes": total,
        "swap_used_bytes": swap_used,
        "swap_raw": swap_raw,
    }


def collect_power() -> dict[str, Any]:
    raw = sh(["pmset", "-g"]) or ""
    low_power = None
    for line in raw.splitlines():
        if line.strip().startswith("lowpowermode"):
            low_power = line.split()[1] == "1"
    return {
        "low_power_mode": low_power,
        # powermetrics is root-only; this is the honest unprivileged answer.
        "thermal_pressure": None,
        "thermal_source": "unavailable-without-root",
    }


# --------------------------------------------------------------------------- #
# bandwidth microbenchmark
# --------------------------------------------------------------------------- #

@dataclass
class BandwidthResult:
    method: str
    buffer_bytes: int
    reps: int
    best_gbs: float
    median_gbs: float
    spread_pct: float
    samples_gbs: list[float] = field(default_factory=list)


def measure_bandwidth(buf_mib: int = 256, reps: int = 7) -> dict[str, Any]:
    """Measure achievable memory bandwidth, single-threaded AND saturated.

    Buffer is sized well above LLC (Apple M-series L2 tops out at 16 MiB on
    the top perf level) so we measure DRAM, not cache.

    Why both numbers matter: on Apple Silicon a single-threaded memcpy reaches
    roughly half of what 4+ threads achieve. A fingerprint that publishes only
    the single-threaded figure under-predicts decode by ~2x -- which is exactly
    the kind of confident wrongness this product exists to avoid.

    copy : dst[:] = src        -> 2N bytes moved (1 read + 1 write)
    triad: c[:] = a[:] + b[:]  -> 3N bytes moved (2 reads + 1 write)
    """
    from concurrent.futures import ThreadPoolExecutor

    n = (buf_mib * 1024 * 1024) // 8  # float64 elements
    a = np.ones(n, dtype=np.float64)
    b = np.full(n, 2.0, dtype=np.float64)
    c = np.empty(n, dtype=np.float64)
    nbytes = n * 8

    def summarise(name: str, samples: list[float]) -> BandwidthResult:
        s = sorted(samples)
        best = s[-1]
        med = s[len(s) // 2]
        spread = (best - s[0]) / best * 100 if best else 0.0
        return BandwidthResult(name, nbytes, reps, round(best, 2), round(med, 2),
                               round(spread, 2), [round(x, 2) for x in samples])

    def serial(fn, multiplier: float) -> list[float]:
        fn()  # warm-up, discarded
        out = []
        for _ in range(reps):
            t0 = time.perf_counter()
            fn()
            dt = time.perf_counter() - t0
            out.append((nbytes * multiplier) / dt / 1e9)
        return out

    def chunked_copy(parts: int) -> list[float]:
        def worker(i: int) -> None:
            lo, hi = i * n // parts, (i + 1) * n // parts
            np.copyto(c[lo:hi], a[lo:hi])

        out = []
        with ThreadPoolExecutor(max_workers=parts) as ex:
            list(ex.map(worker, range(parts)))  # warm-up
            for _ in range(reps):
                t0 = time.perf_counter()
                list(ex.map(worker, range(parts)))
                dt = time.perf_counter() - t0
                out.append((nbytes * 2.0) / dt / 1e9)
        return out

    copy_samples = serial(lambda: np.copyto(c, a), 2.0)
    triad_samples = serial(lambda: np.add(a, b, out=c), 3.0)

    thread_counts = [2, 4, 8, 15]
    sweep = {tc: summarise(f"copy@{tc}t", chunked_copy(tc)) for tc in thread_counts}
    best_tc = max(sweep, key=lambda k: sweep[k].best_gbs)

    return {
        "copy": asdict(summarise("copy", copy_samples)),
        "triad": asdict(summarise("triad", triad_samples)),
        "single_thread_gbs": round(max(copy_samples), 2),
        "thread_sweep": {str(k): v.best_gbs for k, v in sweep.items()},
        "best_threads": best_tc,
        # Headline is the saturated figure: the analogue of what a GPU backend
        # can pull from the same DRAM during weight streaming.
        "headline_gbs": sweep[best_tc].best_gbs,
        "buffer_mib": buf_mib,
    }


# --------------------------------------------------------------------------- #
# assembly
# --------------------------------------------------------------------------- #

def build_fingerprint() -> dict[str, Any]:
    bw = measure_bandwidth()
    return {
        "schema": SCHEMA,
        "collected_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "identity": collect_identity(),
        "cpu": collect_cpu(),
        "gpu": collect_gpu(),
        "memory": collect_memory(),
        "power": collect_power(),
        "bandwidth": bw,
        "provenance": {
            "bandwidth": "measured",
            "cpu": "exact",
            "gpu": "exact",
            "thermal": "unknown",
        },
    }


def main() -> int:
    fp = build_fingerprint()
    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fingerprint.json")
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(fp, fh, indent=2)

    print(f"machine      {fp['cpu']['brand']}  ({fp['cpu']['physical_cores']}P/{fp['cpu']['logical_cores']}L)")
    print(f"gpu          {fp['gpu'].get('model')}  {fp['gpu'].get('cores')} cores  "
          f"{fp['gpu'].get('metal_support')}")
    print(f"memory       {(fp['memory']['total_bytes'] or 0) / 2**30:.1f} GiB  "
          f"swap used {((fp['memory']['swap_used_bytes'] or 0) / 2**30):.2f} GiB")
    bw = fp["bandwidth"]
    print(f"bandwidth    1t {bw['single_thread_gbs']:.1f} GB/s  ->  saturated "
          f"{bw['headline_gbs']:.1f} GB/s @{bw['best_threads']}t   sweep {bw['thread_sweep']}")
    print(f"isa          " + ", ".join(k for k, v in fp["cpu"]["isa"].items() if v))
    print(f"written      {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
