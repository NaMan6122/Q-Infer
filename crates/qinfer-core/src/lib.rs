//! Shared types for every Q-Infer surface.
//!
//! The rule from ADR-0002: this crate knows *nothing* about how a value was
//! obtained. It deserialises what the fingerprint layer produced and hands it
//! to the predictor, the planner and the TUI. Nothing here shells out.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod cfp;
pub use cfp::{parse_major, Violation, MAX_SPREAD_PCT, SUPPORTED_MAJOR};

// ---------------------------------------------------------------------------
// qinfer/cfp@1 — Capability Fingerprint
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Fingerprint {
    pub schema: String,
    #[serde(default)]
    pub collected_at: String,
    pub identity: Identity,
    pub cpu: Cpu,
    #[serde(default)]
    pub gpu: Gpu,
    pub memory: Memory,
    pub bandwidth: Bandwidth,
    #[serde(default)]
    pub power: Power,
    #[serde(default)]
    pub provenance: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    pub os: String,
    #[serde(default)]
    pub os_version: Option<String>,
    pub arch: String,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cpu {
    pub brand: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    #[serde(default)]
    pub perf_levels: Vec<PerfLevel>,
    #[serde(default)]
    pub isa: BTreeMap<String, i64>,
    #[serde(default)]
    pub l3_bytes: Option<u64>,
    #[serde(default)]
    pub max_freq_mhz: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PerfLevel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub physical_cores: Option<u32>,
    #[serde(default)]
    pub l2_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Gpu {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub metal_support: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub unified_memory: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Memory {
    pub total_bytes: u64,
    #[serde(default)]
    pub swap_used_bytes: Option<u64>,
    #[serde(default)]
    pub swap_raw: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Power {
    #[serde(default)]
    pub low_power_mode: Option<bool>,
    #[serde(default)]
    pub thermal_pressure: Option<String>,
    #[serde(default)]
    pub thermal_source: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bandwidth {
    #[serde(default)]
    pub copy: Option<Sample>,
    #[serde(default)]
    pub triad: Option<Sample>,
    #[serde(default)]
    pub single_thread_gbs: Option<f64>,
    /// Thread count -> GB/s. Keys arrive as strings from JSON.
    #[serde(default)]
    pub thread_sweep: BTreeMap<String, f64>,
    #[serde(default)]
    pub best_threads: Option<u32>,
    pub headline_gbs: f64,
    #[serde(default)]
    pub buffer_mib: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sample {
    pub method: String,
    pub best_gbs: f64,
    pub median_gbs: f64,
    pub spread_pct: f64,
}

// ---------------------------------------------------------------------------
// M0 spike results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SpikeResults {
    pub bandwidth_gbs: f64,
    #[serde(default)]
    pub bandwidth_single_thread_gbs: Option<f64>,
    #[serde(default)]
    pub reps: u32,
    pub fit: Fit,
    pub mape: Mape,
    pub rows: Vec<SpikeRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fit {
    pub t0_ms: f64,
    pub eps: f64,
    pub r2: f64,
    #[serde(default)]
    pub determined: bool,
    #[serde(default)]
    pub n_models: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mape {
    pub naive_pct: f64,
    pub two_param_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpikeRow {
    pub model: String,
    pub gib: f64,
    pub measured_tps: f64,
    #[serde(default)]
    pub pp_tps: f64,
    pub naive_tps: f64,
    pub naive_err_pct: f64,
    pub two_param_tps: f64,
    pub two_param_err_pct: f64,
    #[serde(default)]
    pub implied_effective_gbs: f64,
    #[serde(default)]
    pub implied_eps: f64,
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub backend: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Fingerprint {
    /// Saturated bandwidth is the number the predictor uses (PRD §5.2).
    pub fn saturated_gbs(&self) -> f64 {
        self.bandwidth.headline_gbs
    }

    pub fn single_thread_gbs(&self) -> Option<f64> {
        self.bandwidth.single_thread_gbs
    }

    /// How much the single-threaded probe under-reports. Anything above ~1.3
    /// means a naive probe would have halved the prediction (spike F1).
    pub fn thread_gain(&self) -> Option<f64> {
        self.single_thread_gbs()
            .filter(|v| *v > 0.0)
            .map(|st| self.saturated_gbs() / st)
    }

    pub fn total_gib(&self) -> f64 {
        self.memory.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn swap_gib(&self) -> f64 {
        self.memory.swap_used_bytes.unwrap_or(0) as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    /// A machine in deep swap produces garbage bandwidth numbers. The
    /// fingerprint is still valid, but predictions built on it are not.
    pub fn memory_pressured(&self) -> bool {
        self.total_gib() > 0.0 && self.swap_gib() > 0.5 * self.total_gib()
    }

    pub fn isa_enabled(&self) -> Vec<&str> {
        self.cpu
            .isa
            .iter()
            .filter(|(_, v)| **v != 0)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// e.g. "5 Super + 10 Performance". Names come from the OS and are not
    /// stable across generations (M5 uses "Super", M1–M4 use "Performance"),
    /// so this is display-only and never parsed.
    pub fn perf_summary(&self) -> String {
        let parts: Vec<String> = self
            .cpu
            .perf_levels
            .iter()
            .filter_map(|p| {
                let n = p.physical_cores?;
                Some(match p.name.as_deref() {
                    Some(name) if !name.is_empty() => format!("{} {}", n, name),
                    _ => format!("{} cores", n),
                })
            })
            .collect();
        if parts.is_empty() {
            format!("{} cores", self.cpu.physical_cores)
        } else {
            parts.join(" + ")
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Where the dashboard looks for data. M0 reads the spike artefacts directly so
/// the UI can be built and tested before the planner exists.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub fingerprint: Option<Fingerprint>,
    pub results: Option<SpikeResults>,
    pub error: Option<String>,
}

impl Workspace {
    pub fn discover() -> Self {
        let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let root = find_spike_dir(&start).unwrap_or(start);
        Self::load(root)
    }

    pub fn load(root: PathBuf) -> Self {
        let mut ws = Workspace { root: root.clone(), fingerprint: None, results: None, error: None };

        let fp_path = root.join("fingerprint.json");
        match std::fs::read_to_string(&fp_path) {
            Ok(s) => match serde_json::from_str::<Fingerprint>(&s) {
                Ok(fp) => ws.fingerprint = Some(fp),
                Err(e) => ws.error = Some(format!("{}: {}", fp_path.display(), e)),
            },
            Err(_) => {
                ws.error = Some(format!(
                    "no fingerprint at {} — run `python3 spikes/m0/fingerprint.py`",
                    fp_path.display()
                ))
            }
        }

        let res_path = root.join("results.json");
        if let Ok(s) = std::fs::read_to_string(&res_path) {
            match serde_json::from_str::<SpikeResults>(&s) {
                Ok(r) => ws.results = Some(r),
                Err(e) => {
                    if ws.error.is_none() {
                        ws.error = Some(format!("{}: {}", res_path.display(), e));
                    }
                }
            }
        }

        ws
    }

    pub fn reload(&mut self) {
        *self = Self::load(self.root.clone());
    }

    pub fn has_data(&self) -> bool {
        self.fingerprint.is_some()
    }
}

fn find_spike_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join("spikes").join("m0");
        if candidate.join("fingerprint.json").exists() {
            return Some(candidate);
        }
        if dir.join("fingerprint.json").exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_gain_reflects_single_vs_saturated() {
        let mut sweep = BTreeMap::new();
        sweep.insert("15".to_string(), 228.0);
        let fp = Fingerprint {
            schema: "qinfer/cfp@1".into(),
            collected_at: String::new(),
            identity: Identity {
                os: "macos".into(),
                os_version: None,
                arch: "arm64".into(),
                kernel: None,
                hostname: None,
            },
            cpu: Cpu {
                brand: "Apple M5 Pro".into(),
                physical_cores: 15,
                logical_cores: 15,
                perf_levels: vec![],
                isa: BTreeMap::new(),
                l3_bytes: None,
                max_freq_mhz: None,
            },
            gpu: Gpu::default(),
            memory: Memory { total_bytes: 24 * 1024 * 1024 * 1024, swap_used_bytes: None, swap_raw: None },
            bandwidth: Bandwidth {
                copy: None,
                triad: None,
                single_thread_gbs: Some(131.0),
                thread_sweep: sweep,
                best_threads: Some(15),
                headline_gbs: 228.0,
                buffer_mib: None,
            },
            power: Power::default(),
            provenance: BTreeMap::new(),
        };

        let gain = fp.thread_gain().expect("gain");
        assert!((gain - 1.74).abs() < 0.01, "got {gain}");
        assert!(!fp.memory_pressured());
    }

    #[test]
    fn heavy_swap_marks_memory_pressured() {
        let fp = Fingerprint {
            schema: "x".into(),
            collected_at: String::new(),
            identity: Identity { os: "macos".into(), os_version: None, arch: "arm64".into(), kernel: None, hostname: None },
            cpu: Cpu { brand: "x".into(), physical_cores: 1, logical_cores: 1, perf_levels: vec![], isa: BTreeMap::new(), l3_bytes: None, max_freq_mhz: None },
            gpu: Gpu::default(),
            memory: Memory { total_bytes: 8 << 30, swap_used_bytes: Some(6 << 30), swap_raw: None },
            bandwidth: Bandwidth { copy: None, triad: None, single_thread_gbs: None, thread_sweep: BTreeMap::new(), best_threads: None, headline_gbs: 100.0, buffer_mib: None },
            power: Power::default(),
            provenance: BTreeMap::new(),
        };
        assert!(fp.memory_pressured());
    }
}
