//! Memory budget for a workload (T-012).
//!
//! Before this module, `session` answered "how much memory may this workload
//! use?" with **half of RAM**, stated as an assumption and then consumed as a
//! fact. On the reference machine — a 24 GiB M5 Pro — that is 12.0 GiB, which
//! **rejects `qwen3-30b-a3b-q4_k_m` at 17.3 GiB**: a model Apple ships running on
//! a 24 GB MacBook Pro. An assumption that refuses the correct answer is worse
//! than no plan at all, because it is indistinguishable from a hardware limit.
//!
//! Three sources, in order of honesty:
//!
//! 1. **`recommendedMaxWorkingSetSize`** from the GPU device info. This is the
//!    ceiling Metal itself will grant a workload — measured **17.76 GiB** on this
//!    24 GiB M5 Pro (19 069 665 280 B, 0.74 x RAM). It is a *measurement about
//!    the device*, not a policy we chose.
//! 2. **`vm_stat` reclaimable** (free + inactive + speculative + purgeable). This
//!    answers a different question — what could start *right now* — and is
//!    therefore a weaker substitute for a ceiling.
//! 3. **Half of RAM**, only when neither probe worked, and it says so.
//!
//! Ceiling and headroom are deliberately **not** collapsed into one number.
//! Planning asks "what would this machine serve after a fresh start?" and must
//! use the ceiling; a supervisor asks "can this start without evicting the
//! user's work?" and must use the headroom. Building one number that answers
//! both is how a planner ends up refusing a model that runs fine.

use crate::{Fingerprint, PressureState};

/// Fallback when neither probe worked. Kept public so surfaces can label a plan
/// as resting on an assumption rather than on a measurement.
pub const ASSUMED_AVAILABLE_FRACTION: f64 = 0.5;

/// How far below physical RAM a claimed ceiling must sit to be believable.
/// A probe that reports more memory than the machine has is broken, and taking
/// its word would produce a plan that cannot exist.
pub const MAX_CEILING_FRACTION: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceSource {
    /// Metal's `recommendedMaxWorkingSetSize` — the GPU's own ceiling.
    MetalWorkingSet,
    /// `vm_stat` reclaimable — a snapshot of what is free now, not a ceiling.
    VmStatReclaimable,
    /// Neither probe worked; a stated fraction of RAM.
    AssumedFraction,
}

impl SpaceSource {
    /// The name a receipt records. Never a brand, always a method.
    pub fn label(self) -> &'static str {
        match self {
            SpaceSource::MetalWorkingSet => "recommendedMaxWorkingSetSize",
            SpaceSource::VmStatReclaimable => "vm_stat reclaimable",
            SpaceSource::AssumedFraction => "assumed 50% of RAM",
        }
    }

    /// True for the two sources that came from the operating system.
    pub fn is_measured(self) -> bool {
        !matches!(self, SpaceSource::AssumedFraction)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub total_bytes: u64,
    /// Bytes a single GPU-backed workload may claim on an otherwise idle machine.
    pub ceiling_bytes: u64,
    /// Bytes obtainable *now*, when the OS told us. `None` is "unknown", never
    /// "zero" — a machine we cannot interrogate is not a full machine.
    pub headroom_bytes: Option<u64>,
    pub source: SpaceSource,
    pub pressure: PressureState,
}

impl Space {
    /// The budget a **plan** is computed against: what this machine would serve
    /// after a fresh start. Independent of how busy it happens to be right now.
    pub fn plan_budget(&self) -> u64 {
        self.ceiling_bytes
    }

    /// Can `need_bytes` be allocated *now*? Three-valued on purpose: a machine
    /// whose free memory we could not read must not answer `false`.
    pub fn fits_now(&self, need_bytes: u64) -> Option<bool> {
        self.headroom_bytes.map(|h| need_bytes <= h)
    }

    pub fn is_pressured(&self) -> bool {
        matches!(self.pressure, PressureState::Pressured { .. })
    }

    /// True only when the ceiling came from the OS **and** the machine is in a
    /// state where that number means something. A paging machine can report a
    /// perfectly good working-set size while producing garbage measurements
    /// (the reference host sat at 94% swap while the probe read 79% spread).
    pub fn is_measured(&self) -> bool {
        self.source.is_measured() && !self.is_pressured()
    }

    /// `"measured"` | `"unknown"` — the vocabulary `cfp@1` already uses for
    /// provenance, so a space figure and a bandwidth figure are demoted by the
    /// same rule.
    pub fn provenance(&self) -> &'static str {
        if self.is_measured() {
            "measured"
        } else {
            "unknown"
        }
    }

    /// One line for a receipt or a dashboard panel. Deliberately states the
    /// method, because "12.0 GiB available" with no method is the bug this
    /// module exists to fix.
    pub fn describe(&self) -> String {
        let gib = |b: u64| b as f64 / (1u64 << 30) as f64;
        let mut s = format!(
            "{:.1} GiB of {:.1} GiB usable ({})",
            gib(self.ceiling_bytes),
            gib(self.total_bytes),
            self.source.label()
        );
        if let Some(h) = self.headroom_bytes {
            s.push_str(&format!(" · {:.1} GiB free now", gib(h)));
        }
        if self.is_pressured() {
            s.push_str(" · ⚠ memory pressure, figure not trustworthy");
        }
        s
    }
}

/// What the GPU says about itself. Field names follow MLX's `device_info()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    pub device_name: Option<String>,
    /// e.g. `applegpu_g17s`. A fact about the silicon, not a brand.
    pub architecture: Option<String>,
    pub memory_size: Option<u64>,
    /// The number this module exists for.
    pub recommended_working_set: Option<u64>,
    /// Largest single buffer; smaller than the working set by ~25%, so it is
    /// recorded but never used as a budget — a model is not one buffer.
    pub max_buffer_length: Option<u64>,
}

pub mod macos {
    use super::DeviceInfo;

    /// Parse the JSON printed by the GPU device probe.
    ///
    /// Keys are accepted under their current names only. MLX renamed the module
    /// path (`mx.metal.device_info` -> `mx.device_info`) and deprecated the old
    /// name; the payload keys are stable, but a future rename must degrade to
    /// `None` here rather than to a zero budget.
    pub fn parse_device_info(json: &str) -> Option<DeviceInfo> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let obj = v.as_object()?;

        let num = |key: &str| obj.get(key).and_then(|x| x.as_u64());
        let text = |key: &str| obj.get(key).and_then(|x| x.as_str()).map(str::to_string);

        let info = DeviceInfo {
            device_name: text("device_name"),
            architecture: text("architecture"),
            memory_size: num("memory_size"),
            recommended_working_set: num("max_recommended_working_set_size"),
            max_buffer_length: num("max_buffer_length"),
        };

        // A document with none of the fields we understand is not a device
        // report; returning `Some(DeviceInfo::default())` would let a broken
        // probe masquerade as a successful one.
        if info.memory_size.is_none()
            && info.recommended_working_set.is_none()
            && info.max_buffer_length.is_none()
        {
            return None;
        }
        Some(info)
    }
}

fn sane(bytes: u64, total: u64) -> bool {
    bytes > 0 && total > 0 && bytes as f64 <= total as f64 * MAX_CEILING_FRACTION
}

/// Resolve the memory budget for a fingerprint.
///
/// `device` is the GPU's own report when the micro-op layer managed to get one.
/// `None` is a normal, expected input — on a machine with no GPU probe, or on
/// CI — and must degrade through the documented chain rather than panicking or
/// silently reporting half of RAM as if it were a measurement.
pub fn resolve(fp: &Fingerprint, device: Option<&DeviceInfo>) -> Space {
    let total = fp.memory.total_bytes;
    let pressure = fp.pressure();

    let reported_ws = device.and_then(|d| d.recommended_working_set).filter(|v| sane(*v, total));
    let reclaimable = fp.memory.available_bytes.filter(|v| sane(*v, total));

    let (ceiling_bytes, source) = match (reported_ws, reclaimable) {
        (Some(ws), _) => (ws, SpaceSource::MetalWorkingSet),
        (None, Some(v)) => (v, SpaceSource::VmStatReclaimable),
        (None, None) => (
            (total as f64 * ASSUMED_AVAILABLE_FRACTION) as u64,
            SpaceSource::AssumedFraction,
        ),
    };

    Space {
        total_bytes: total,
        ceiling_bytes,
        // Only a snapshot of current free memory may answer "can I start now?".
        // A working-set size must not, or a busy machine would look idle.
        headroom_bytes: reclaimable,
        source,
        pressure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bandwidth, Cpu, Gpu, Identity, Memory, Power};
    use std::collections::BTreeMap;

    /// The reference machine, as measured: 24 GiB, 0.74 x RAM working set.
    const TOTAL: u64 = 25_769_803_776;
    const MEASURED_WS: u64 = 19_069_665_280;

    const DEVICE_JSON: &str = r#"{
      "architecture": "applegpu_g17s",
      "device_name": "Apple M5 Pro",
      "max_buffer_length": 14302248960,
      "max_recommended_working_set_size": 19069665280,
      "memory_size": 25769803776,
      "resource_limit": 499000
    }"#;

    fn fingerprint(available: Option<u64>, swap: Option<&str>) -> Fingerprint {
        Fingerprint {
            schema: "qinfer/cfp@1".into(),
            collected_at: String::new(),
            identity: Identity {
                os: "macos".into(),
                os_version: None,
                os_build: None,
                arch: "arm64".into(),
                kernel: None,
                hostname: None,
            },
            cpu: Cpu {
                brand: "Apple M5 Pro".into(),
                physical_cores: 15,
                logical_cores: 15,
                arch_id: Some("apple-m5-pro".into()),
                perf_levels: vec![],
                isa: BTreeMap::new(),
                l3_bytes: None,
                max_freq_mhz: None,
            },
            gpu: Gpu::default(),
            memory: Memory {
                total_bytes: TOTAL,
                available_bytes: available,
                swap_used_bytes: None,
                swap_raw: swap.map(str::to_string),
            },
            bandwidth: Bandwidth {
                copy: None,
                triad: None,
                single_thread_gbs: 133.2,
                thread_sweep: BTreeMap::new(),
                best_threads: Some(15),
                headline_gbs: 256.9,
                headline_method: Some("copy".into()),
                buffer_mib: Some(256),
            },
            power: Power::default(),
            provenance: BTreeMap::new(),
        }
    }

    fn calm(available: Option<u64>) -> Fingerprint {
        fingerprint(available, Some("total = 10240.00M  used = 0.00M  free = 10240.00M"))
    }

    #[test]
    fn device_info_parses_the_measured_document() {
        let d = macos::parse_device_info(DEVICE_JSON).expect("device info");
        assert_eq!(d.recommended_working_set, Some(MEASURED_WS));
        assert_eq!(d.memory_size, Some(TOTAL));
        assert_eq!(d.architecture.as_deref(), Some("applegpu_g17s"));
        assert_eq!(d.max_buffer_length, Some(14_302_248_960));
    }

    #[test]
    fn a_document_with_no_known_fields_is_not_a_device_report() {
        assert!(macos::parse_device_info(r#"{"something":"else"}"#).is_none());
        assert!(macos::parse_device_info("not json").is_none());
        assert!(macos::parse_device_info("").is_none());
    }

    /// The bug this module exists to fix, stated with the artifacts that are
    /// actually on the reference machine.
    ///
    /// Honesty cuts both ways: the measured ceiling admits a 14.98 GiB model that
    /// half-of-RAM refused, and it *still* refuses a 17.32 GiB one. A larger
    /// budget is not permission to round up.
    #[test]
    fn the_measured_working_set_admits_what_half_of_ram_refused() {
        let d = macos::parse_device_info(DEVICE_JSON).unwrap();
        let s = resolve(&calm(Some(5 << 30)), Some(&d));
        assert_eq!(s.source, SpaceSource::MetalWorkingSet);
        assert_eq!(s.plan_budget(), MEASURED_WS);
        assert!(s.plan_budget() > TOTAL / 2, "17.76 GiB must beat the 12.0 GiB assumption");

        // qwen3.8-27b-4bit-mlx, the artifact sitting in this machine's HF cache.
        let qwen27b = 16_081_488_731u64;
        let need = qwen27b + qwen27b / 20;
        assert!(need <= s.plan_budget(), "14.98 GiB plus 5% headroom must fit");
        assert!(need > TOTAL / 2, "and the old assumption would have refused it");

        // qwen3-30b-a3b-q4_k_m: 17.32 GiB + headroom is still too large for 24 GiB.
        let qwen30b = 18_600_000_000u64;
        assert!(
            qwen30b + qwen30b / 20 > s.plan_budget(),
            "30B-A3B must not be admitted by a 17.76 GiB ceiling"
        );
    }

    #[test]
    fn an_impossible_working_set_falls_through_to_the_reclaimable_figure() {
        let absurd = DeviceInfo { recommended_working_set: Some(TOTAL * 4), ..Default::default() };
        let s = resolve(&calm(Some(5 << 30)), Some(&absurd));
        assert_eq!(s.source, SpaceSource::VmStatReclaimable);
        assert_eq!(s.ceiling_bytes, 5 << 30);
    }

    #[test]
    fn a_zero_working_set_is_treated_as_a_failed_probe() {
        let zero = DeviceInfo { recommended_working_set: Some(0), ..Default::default() };
        let s = resolve(&calm(None), Some(&zero));
        assert_eq!(s.source, SpaceSource::AssumedFraction);
        assert_eq!(s.ceiling_bytes, TOTAL / 2);
    }

    #[test]
    fn with_nothing_measured_it_says_assumed_and_does_not_claim_measured() {
        let s = resolve(&calm(None), None);
        assert_eq!(s.source, SpaceSource::AssumedFraction);
        assert_eq!(s.provenance(), "unknown");
        assert!(!s.is_measured());
        assert!(s.describe().contains("assumed"));
    }

    #[test]
    fn a_pressured_machine_cannot_claim_a_measured_budget() {
        let d = macos::parse_device_info(DEVICE_JSON).unwrap();
        // The reference host, verbatim: 10 GB configured, 654 MB free.
        let fp = fingerprint(
            Some(5 << 30),
            Some("total = 10240.00M  used = 9585.56M  free = 654.44M"),
        );
        let s = resolve(&fp, Some(&d));
        assert!(s.is_pressured());
        assert_eq!(s.source, SpaceSource::MetalWorkingSet);
        assert_eq!(s.provenance(), "unknown", "a paging machine is not a measurement");
        assert!(s.describe().contains("memory pressure"));
    }

    #[test]
    fn the_ceiling_is_a_plan_and_does_not_move_with_current_load() {
        let d = macos::parse_device_info(DEVICE_JSON).unwrap();
        let busy = resolve(&calm(Some(1 << 30)), Some(&d));
        let idle = resolve(&calm(Some(16 << 30)), Some(&d));
        assert_eq!(busy.plan_budget(), idle.plan_budget(), "a plan is not a snapshot");
        assert_eq!(busy.fits_now(9 << 30), Some(false));
        assert_eq!(idle.fits_now(9 << 30), Some(true));
    }

    #[test]
    fn unknown_headroom_is_none_not_false() {
        let s = resolve(&calm(None), None);
        assert_eq!(s.headroom_bytes, None);
        assert_eq!(s.fits_now(1 << 40), None, "an unreadable machine must not refuse");
    }

    #[test]
    fn a_zero_memory_fingerprint_does_not_panic() {
        let mut fp = calm(None);
        fp.memory.total_bytes = 0;
        let s = resolve(&fp, None);
        assert_eq!(s.ceiling_bytes, 0);
        assert_eq!(s.source, SpaceSource::AssumedFraction);
    }
}
