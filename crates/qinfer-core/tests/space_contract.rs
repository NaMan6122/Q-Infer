//! Independent contract tests for `space` (T-012).
//!
//! These are written from the acceptance criteria, not from the implementation,
//! and deliberately use a different machine profile from the module's own unit
//! tests (a 32 GiB part rather than the 24 GiB M5 Pro) so that a test passing
//! cannot be an artefact of shared fixtures.
//!
//! The contract in one sentence: **a plan is computed against the ceiling this
//! machine will grant, and a figure is only called `measured` when the machine
//! is in a state where measuring it means something.**

use qinfer_core::space::{self, macos, DeviceInfo, SpaceSource, ASSUMED_AVAILABLE_FRACTION};
use qinfer_core::{Bandwidth, Cpu, Fingerprint, Gpu, Identity, Memory, Power};
use std::collections::BTreeMap;

/// A hypothetical 32 GiB part, chosen so the arithmetic differs from the
/// 24 GiB reference machine in the module's unit tests.
const TOTAL: u64 = 34_359_738_368;
/// 0.75 x RAM, i.e. what Metal would plausibly report for such a machine.
const WORKING_SET: u64 = 25_769_803_776;

const CALM_SWAP: &str = "total = 8192.00M  used = 0.00M  free = 8192.00M";
const PAGING_SWAP: &str = "total = 8192.00M  used = 7900.00M  free = 292.00M";

fn fingerprint(available: Option<u64>, swap: &str) -> Fingerprint {
    Fingerprint {
        schema: "qinfer/cfp@1".into(),
        collected_at: "2026-10-27T00:00:00Z".into(),
        identity: Identity {
            os: "macos".into(),
            os_version: Some("27.0.1".into()),
            os_build: Some("26A434".into()),
            arch: "arm64".into(),
            kernel: None,
            hostname: Some("contract-fixture".into()),
        },
        cpu: Cpu {
            brand: "Apple M9 Ultra".into(),
            physical_cores: 20,
            logical_cores: 20,
            arch_id: Some("apple-m9-ultra".into()),
            perf_levels: vec![],
            isa: BTreeMap::new(),
            l3_bytes: None,
            max_freq_mhz: None,
        },
        gpu: Gpu {
            model: Some("Apple M9 Ultra".into()),
            cores: Some(80),
            metal_support: Some("spdisplays_metal4".into()),
            vendor: Some("Apple".into()),
            unified_memory: true,
        },
        memory: Memory {
            total_bytes: TOTAL,
            available_bytes: available,
            swap_used_bytes: None,
            swap_raw: Some(swap.to_string()),
        },
        bandwidth: Bandwidth {
            copy: None,
            triad: None,
            single_thread_gbs: 200.0,
            thread_sweep: BTreeMap::new(),
            best_threads: Some(20),
            headline_gbs: 400.0,
            headline_method: Some("copy".into()),
            buffer_mib: Some(512),
        },
        power: Power::default(),
        provenance: BTreeMap::new(),
    }
}

fn device(working_set: Option<u64>) -> DeviceInfo {
    DeviceInfo {
        device_name: Some("Apple M9 Ultra".into()),
        architecture: Some("applegpu_g99".into()),
        memory_size: Some(TOTAL),
        recommended_working_set: working_set,
        max_buffer_length: Some(WORKING_SET / 4 * 3),
    }
}

/// Criterion 1. The whole reason this module exists: the OS's own working-set
/// figure must win over the half-of-RAM guess, or a machine refuses models it
/// can actually run.
#[test]
fn the_working_set_wins_over_half_of_ram() {
    let s = space::resolve(&fingerprint(Some(4 << 30), CALM_SWAP), Some(&device(Some(WORKING_SET))));
    assert_eq!(s.source, SpaceSource::MetalWorkingSet);
    assert_eq!(s.plan_budget(), WORKING_SET);
    assert_ne!(s.plan_budget(), TOTAL / 2, "half of RAM is a fallback, not an answer");
}

/// Criterion 2. A probe that reports more memory than the machine has is
/// broken. Clamping it to RAM would fabricate a number; accepting it would plan
/// an impossible install. It must fall through the chain instead.
#[test]
fn a_working_set_larger_than_ram_falls_through() {
    let s = space::resolve(&fingerprint(Some(4 << 30), CALM_SWAP), Some(&device(Some(TOTAL * 3))));
    assert_eq!(s.source, SpaceSource::VmStatReclaimable);
    assert_eq!(s.plan_budget(), 4 << 30, "must not be clamped to RAM, nor accepted");
}

/// Criterion 3. A zero is a failed probe, not a machine with no memory.
#[test]
fn a_zero_working_set_is_a_failed_probe() {
    let s = space::resolve(&fingerprint(None, CALM_SWAP), Some(&device(Some(0))));
    assert_eq!(s.source, SpaceSource::AssumedFraction);
    assert_eq!(s.plan_budget(), (TOTAL as f64 * ASSUMED_AVAILABLE_FRACTION) as u64);
}

/// Criterion 4. When nothing was measured, the module must say so in the
/// vocabulary the rest of the system already uses for provenance.
#[test]
fn an_assumed_budget_never_claims_to_be_measured() {
    let s = space::resolve(&fingerprint(None, CALM_SWAP), None);
    assert_eq!(s.source, SpaceSource::AssumedFraction);
    assert_eq!(s.provenance(), "unknown");
    assert!(!s.is_measured());
}

/// Criterion 5. The subtle one. A paging machine still reports a perfectly good
/// working-set size, so `source` looks healthy while nothing measured on it is
/// trustworthy. Provenance must be demoted on pressure alone: the reference
/// host sat at 94% swap while its bandwidth probe read 79% spread.
#[test]
fn a_paging_machine_cannot_claim_a_measured_budget() {
    let s = space::resolve(&fingerprint(Some(4 << 30), PAGING_SWAP), Some(&device(Some(WORKING_SET))));
    assert!(s.is_pressured());
    assert_eq!(s.source, SpaceSource::MetalWorkingSet, "the source is still real");
    assert_eq!(s.provenance(), "unknown", "but the figure is not evidence");
    assert!(!s.is_measured());
}

/// Criterion 6. Three-valued answer: a machine whose free memory we could not
/// read must not answer "no".
#[test]
fn unknown_headroom_is_none_never_false() {
    let s = space::resolve(&fingerprint(None, CALM_SWAP), Some(&device(Some(WORKING_SET))));
    assert_eq!(s.headroom_bytes, None);
    assert_eq!(s.fits_now(u64::MAX), None);
}

/// Criterion 7. A plan is not a snapshot. If the budget moved with current
/// load, the same machine would produce different plans minute to minute and no
/// receipt would be reproducible.
#[test]
fn the_plan_budget_does_not_move_with_load() {
    let d = device(Some(WORKING_SET));
    let busy = space::resolve(&fingerprint(Some(1 << 30), CALM_SWAP), Some(&d));
    let idle = space::resolve(&fingerprint(Some(24 << 30), CALM_SWAP), Some(&d));
    assert_eq!(busy.plan_budget(), idle.plan_budget());
    // The headroom question may differ, and must.
    assert_eq!(busy.fits_now(8 << 30), Some(false));
    assert_eq!(idle.fits_now(8 << 30), Some(true));
}

/// Criterion 8. Degenerate input must not panic; the tool runs on unknown
/// machines and a crash is the least useful failure mode.
#[test]
fn a_zero_memory_machine_does_not_panic() {
    let mut fp = fingerprint(None, CALM_SWAP);
    fp.memory.total_bytes = 0;
    let s = space::resolve(&fp, Some(&device(Some(WORKING_SET))));
    assert_eq!(s.plan_budget(), 0);
    assert_eq!(s.source, SpaceSource::AssumedFraction);
}

/// Criterion 9. The live MLX payload captured from a real M5 Pro, verbatim.
#[test]
fn the_live_device_payload_is_recoverable() {
    let json = r#"{"architecture":"applegpu_g17s","device_name":"Apple M5 Pro","max_buffer_length":14302248960,"max_recommended_working_set_size":19069665280,"memory_size":25769803776,"resource_limit":499000}"#;
    let d = macos::parse_device_info(json).expect("live payload must parse");
    assert_eq!(d.device_name.as_deref(), Some("Apple M5 Pro"));
    assert_eq!(d.architecture.as_deref(), Some("applegpu_g17s"));
    assert_eq!(d.memory_size, Some(25_769_803_776));
    assert_eq!(d.recommended_working_set, Some(19_069_665_280));
    assert_eq!(d.max_buffer_length, Some(14_302_248_960));
}

/// Criterion 9b. A document with none of the keys we understand is not a device
/// report. Returning a default-valued `Some` would let a broken probe pass as a
/// successful one and quietly halve the budget.
#[test]
fn an_unrecognised_payload_is_not_a_device_report() {
    assert!(macos::parse_device_info(r#"{"vendor":"acme","vram_gb":16}"#).is_none());
    assert!(macos::parse_device_info("null").is_none());
    assert!(macos::parse_device_info("").is_none());
}

/// The end-to-end shape of the bug, stated as arithmetic rather than as a
/// number: an artifact larger than half of RAM but smaller than the measured
/// working set is admitted by one rule and refused by the other.
///
/// (On the real 24 GiB reference machine the artifact that plays this role is
/// `qwen3.8-27b-4bit-mlx` at 14.98 GiB against a 12.0 GiB assumption. The
/// fixture here is a 32 GiB part, so the boundary sits elsewhere; the property
/// is what is being tested, not the model.)
#[test]
fn a_model_above_half_of_ram_is_admitted_by_the_measured_ceiling() {
    let s = space::resolve(&fingerprint(None, CALM_SWAP), Some(&device(Some(WORKING_SET))));
    let artifact = 20 * (1u64 << 30);
    let need = artifact + artifact / 20; // plus 5% headroom
    assert!(need <= s.plan_budget(), "20 GiB + headroom must fit a 24 GiB working set");
    assert!(need > TOTAL / 2, "and half of this machine's RAM would have refused it");
}
