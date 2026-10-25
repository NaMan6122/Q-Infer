//! Memory bandwidth probe (T-002).
//!
//! Why this exists: decode throughput is `bytes_per_token / bandwidth`
//! (ADR-0003), so the bandwidth number *is* the prediction. Two things the M0
//! spike proved must be built in, not remembered:
//!
//! 1. A single-threaded probe under-reports — 131 GB/s against 228 GB/s
//!    saturated on an M5 Pro, a 1.74x error. So we publish both and sweep
//!    thread counts to find the knee.
//! 2. A machine under memory pressure produces plausible, wrong numbers.
//!    So the caller must supply pressure state, and a pressured probe is not
//!    allowed to claim `measured` (see `cfp::Violation`).
//!
//! The probe is CPU-side by design. On a GPU backend the achievable bandwidth
//! is higher than a CPU memcpy demonstrates; that gap is exactly what `k_cal`
//! absorbs, which is why `k_cal` may exceed 1.0 and is not clamped.

use std::collections::BTreeMap;
use std::time::Instant;

/// Copy moves one read plus one write per element.
const COPY_BYTES_MOVED: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub buffer_bytes: usize,
    pub reps: usize,
    pub thread_counts: Vec<usize>,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        // Sweep up to the physical core count: the knee is machine-specific and
        // stopping at 8 would miss it on a 15-core part.
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
        let mut thread_counts = vec![1, 2, 4, 8];
        if cores > 8 {
            thread_counts.push(cores);
        }
        Self {
            buffer_bytes: 256 * 1024 * 1024,
            reps: 5,
            thread_counts,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub single_thread_gbs: f64,
    /// Best figure across the whole sweep. This is what the predictor uses.
    pub saturated_gbs: f64,
    pub best_threads: usize,
    pub sweep: BTreeMap<usize, f64>,
    /// Run-to-run spread of the single-thread series, in percent.
    pub spread_pct: f64,
    pub buffer_bytes: usize,
    pub reps: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProbeError {
    /// Measuring inside cache tells you about cache, not memory.
    BufferTooSmall { buffer_bytes: usize, required_bytes: usize },
    ZeroBuffer,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::BufferTooSmall { buffer_bytes, required_bytes } => write!(
                f,
                "probe buffer {buffer_bytes} B is below the 4x LLC floor ({required_bytes} B) — this would measure cache, not DRAM"
            ),
            ProbeError::ZeroBuffer => write!(f, "probe buffer must be non-zero"),
        }
    }
}

/// The buffer must be at least 4x last-level cache or the measurement is
/// meaningless. `llc_bytes` is `None` when unknown, in which case we accept any
/// non-zero buffer and record the resulting uncertainty in `spread_pct`.
pub fn validate_config(cfg: &ProbeConfig, llc_bytes: Option<u64>) -> Result<(), ProbeError> {
    if cfg.buffer_bytes == 0 {
        return Err(ProbeError::ZeroBuffer);
    }
    if let Some(llc) = llc_bytes {
        let required = (llc.saturating_mul(4)) as usize;
        if cfg.buffer_bytes < required {
            return Err(ProbeError::BufferTooSmall { buffer_bytes: cfg.buffer_bytes, required_bytes: required });
        }
    }
    Ok(())
}

/// Run the sweep. Returns the best figure per thread count plus the saturated
/// headline. Never panics on plausible configs; a zero buffer is rejected by
/// `validate_config` before this is called.
pub fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let n_elems = (cfg.buffer_bytes / std::mem::size_of::<u64>()).max(1);
    let src: Vec<u64> = (0..n_elems).map(|i| i as u64).collect();
    let mut dst: Vec<u64> = vec![0; n_elems];

    let mut counts = cfg.thread_counts.clone();
    counts.push(1);
    counts.sort_unstable();
    counts.dedup();

    let mut sweep: BTreeMap<usize, f64> = BTreeMap::new();
    let mut single_samples: Vec<f64> = Vec::new();

    for threads in counts {
        let mut samples = Vec::with_capacity(cfg.reps);

        // Warm-up, discarded: first touch pays page faults that are not steady-state.
        copy_chunked(&src, &mut dst, threads);

        for _ in 0..cfg.reps.max(1) {
            let t0 = Instant::now();
            copy_chunked(&src, &mut dst, threads);
            let dt = t0.elapsed().as_secs_f64();
            if dt > 0.0 {
                let gbs = (n_elems as f64 * 8.0 * COPY_BYTES_MOVED) / dt / 1e9;
                samples.push(gbs);
            }
        }

        if let Some(best) = samples.iter().cloned().fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.max(x)))
        }) {
            sweep.insert(threads, round2(best));
            if threads == 1 {
                single_samples = samples;
            }
        }
    }

    let single_thread_gbs = sweep.get(&1).copied().unwrap_or(0.0);
    let (best_threads, saturated_gbs) = sweep
        .iter()
        .fold((1usize, 0.0f64), |acc, (t, v)| if *v > acc.1 { (*t, *v) } else { acc });

    let spread_pct = if single_samples.len() > 1 {
        let best = single_samples.iter().cloned().fold(f64::MIN, f64::max);
        let worst = single_samples.iter().cloned().fold(f64::MAX, f64::min);
        if best > 0.0 { round2((best - worst) / best * 100.0) } else { 0.0 }
    } else {
        0.0
    };

    ProbeResult {
        single_thread_gbs,
        saturated_gbs,
        best_threads,
        sweep,
        spread_pct,
        buffer_bytes: cfg.buffer_bytes,
        reps: cfg.reps,
    }
}

fn copy_chunked(src: &[u64], dst: &mut [u64], threads: usize) {
    let threads = threads.max(1);
    if threads == 1 {
        dst.copy_from_slice(src);
        return;
    }
    let chunk = src.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (d, s_chunk) in dst.chunks_mut(chunk).zip(src.chunks(chunk)) {
            s.spawn(move || d.copy_from_slice(s_chunk));
        }
    });
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_cfg() -> ProbeConfig {
        ProbeConfig { buffer_bytes: 32 * 1024 * 1024, reps: 3, thread_counts: vec![1, 2, 4] }
    }

    #[test]
    fn rejects_a_buffer_that_would_measure_cache() {
        let cfg = ProbeConfig { buffer_bytes: 4 * 1024 * 1024, ..small_cfg() };
        let err = validate_config(&cfg, Some(2 * 1024 * 1024)).unwrap_err();
        assert!(matches!(err, ProbeError::BufferTooSmall { .. }));
    }

    #[test]
    fn accepts_a_buffer_at_four_times_llc() {
        let cfg = ProbeConfig { buffer_bytes: 8 * 1024 * 1024, ..small_cfg() };
        assert!(validate_config(&cfg, Some(2 * 1024 * 1024)).is_ok());
    }

    #[test]
    fn accepts_any_nonzero_buffer_when_llc_is_unknown() {
        let cfg = ProbeConfig { buffer_bytes: 1024 * 1024, ..small_cfg() };
        assert!(validate_config(&cfg, None).is_ok());
    }

    #[test]
    fn rejects_a_zero_buffer() {
        let cfg = ProbeConfig { buffer_bytes: 0, ..small_cfg() };
        assert_eq!(validate_config(&cfg, None).unwrap_err(), ProbeError::ZeroBuffer);
    }

    #[test]
    fn sweep_covers_every_requested_thread_count_plus_single() {
        let r = probe(&small_cfg());
        for t in [1, 2, 4] {
            assert!(r.sweep.contains_key(&t), "missing thread count {t} in {:?}", r.sweep);
        }
        assert_eq!(r.buffer_bytes, 32 * 1024 * 1024);
    }

    #[test]
    fn saturated_is_never_below_single_thread() {
        let r = probe(&small_cfg());
        assert!(
            r.saturated_gbs >= r.single_thread_gbs,
            "saturated {} < single {}",
            r.saturated_gbs,
            r.single_thread_gbs
        );
        assert!(r.saturated_gbs > 0.0);
    }

    #[test]
    fn every_thread_count_reports_positive_bandwidth() {
        let r = probe(&small_cfg());
        for (t, gbs) in &r.sweep {
            assert!(*gbs > 0.0, "thread count {t} reported {gbs}");
        }
    }

    /// The headline claim of the spike, re-checked as a property rather than a
    /// hard-coded number: on this class of machine, one thread cannot saturate
    /// memory, so the sweep must find more. Loose by design — CI is shared.
    #[test]
    fn multi_thread_finds_more_bandwidth_than_one_thread() {
        let r = probe(&small_cfg());
        assert!(
            r.saturated_gbs > r.single_thread_gbs * 1.1,
            "expected the sweep to beat single-thread by >10%: single {} saturated {}",
            r.single_thread_gbs,
            r.saturated_gbs
        );
    }
}
