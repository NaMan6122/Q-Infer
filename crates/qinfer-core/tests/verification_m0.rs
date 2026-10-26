//! Independent verification of the M0 acceptance criteria (AOP v5 §7).
//!
//! This file was written by the verification sub-agent, not by the implementers
//! of T-002/T-003/T-005. Two rules it follows deliberately:
//!
//! 1. **Behaviour, not transcription.** Nothing here calls into a unit-test
//!    helper or repeats an implementer's constant. Fixtures are derived from the
//!    committed M0 evidence in `spikes/m0/` (the recorded macOS sysctl surface,
//!    `results.json`, `fingerprint.json`) so agreement is evidence, not a copy.
//! 2. **Specific assertions.** Exact values, exact error variants, exact
//!    violation-code sets. "Some error occurred" hides the defect it is meant to
//!    catch, so where a negative case is the point, the test also proves that
//!    the *unrelated* rules did not fire.

use qinfer_core::collect::{macos, CollectError, Runner};
use qinfer_core::predict::{
    fit, mape, predict, Calibration, KvGeometry, Observation, ShapeClass, CALIBRATED_SPREAD,
    MIN_OBSERVATIONS, Q8_0_BYTES_PER_ELEMENT, UNCALIBRATED_SPREAD,
};
use qinfer_core::probe::{probe, validate_config, ProbeConfig, ProbeError, ProbeResult};
use qinfer_core::{
    Bandwidth, Cpu, Fingerprint, Gpu, Identity, Memory, Power, Sample, SpikeResults, Validated,
    Workspace,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

const MIB: usize = 1024 * 1024;
const GIB: u64 = 1 << 30;

fn spike_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes/m0")
}

// ===========================================================================
// T-002 — bandwidth probe (crates/qinfer-core/src/probe.rs)
// ===========================================================================

fn sweep_cfg() -> ProbeConfig {
    ProbeConfig { buffer_bytes: 16 * MIB, reps: 3, thread_counts: vec![1, 2, 4, 8] }
}

/// AC: the probe reports both a single-thread and a saturated figure, plus the
/// thread count that produced the saturated one.
#[test]
fn t002_reports_single_thread_saturated_and_the_winning_thread_count() {
    let cfg = sweep_cfg();
    let r = probe(&cfg);

    // Both figures are present and positive — neither is optional at the type
    // level, so a zero here would mean the measurement silently failed.
    assert!(r.single_thread_gbs > 0.0, "single-thread figure absent: {}", r.single_thread_gbs);
    assert!(r.saturated_gbs > 0.0, "saturated figure absent: {}", r.saturated_gbs);

    // Saturated is the max over the sweep, and 1 is always in the sweep, so this
    // relationship is an invariant of the implementation rather than a hope.
    assert!(
        r.saturated_gbs >= r.single_thread_gbs,
        "saturated {} < single-thread {}",
        r.saturated_gbs,
        r.single_thread_gbs
    );

    // The headline figure is attributed to a thread count that was really run.
    assert!(r.sweep.contains_key(&1), "the sweep must always contain the single-thread point: {:?}", r.sweep);
    assert!(
        r.sweep.contains_key(&r.best_threads),
        "best_threads = {} is not a measured point: {:?}",
        r.best_threads,
        r.sweep.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        r.sweep.get(&r.best_threads).copied(),
        Some(r.saturated_gbs),
        "the saturated figure must be the sample for best_threads"
    );
    for t in [1usize, 2, 4, 8] {
        assert!(r.sweep.contains_key(&t), "requested thread count {t} is missing from {:?}", r.sweep);
    }
    assert_eq!(r.sweep.len(), 4, "sweep should be exactly the requested set plus 1: {:?}", r.sweep);

    // The descriptive fields the receipt depends on travel with the numbers.
    assert_eq!(r.buffer_bytes, cfg.buffer_bytes);
    assert_eq!(r.reps, cfg.reps);
    assert!(r.median_gbs > 0.0 && r.median_gbs.is_finite(), "median = {}", r.median_gbs);
    assert!(r.spread_pct.is_finite() && r.spread_pct >= 0.0, "spread = {}", r.spread_pct);
}

/// AC: refuses below 4x LLC, accepts at exactly 4x, accepts when LLC is unknown,
/// rejects zero.
#[test]
fn t002_validate_config_enforces_the_four_x_llc_floor_exactly() {
    const LLC: u64 = 3 * MIB as u64; // a deliberately small, exact LLC
    const FLOOR: usize = LLC as usize * 4; // 12 MiB

    let base = ProbeConfig { buffer_bytes: FLOOR, reps: 1, thread_counts: vec![1] };

    // One byte below the floor is refused, and the error carries the arithmetic
    // so the caller can see *why*, not merely *that*.
    let below = ProbeConfig { buffer_bytes: FLOOR - 1, ..base.clone() };
    assert_eq!(
        validate_config(&below, Some(LLC)),
        Err(ProbeError::BufferTooSmall { buffer_bytes: FLOOR - 1, required_bytes: FLOOR }),
        "one byte under 4x LLC must be rejected with the exact required size"
    );

    // The boundary is inclusive: exactly 4x is a measurement, not a cache test.
    assert_eq!(validate_config(&base, Some(LLC)), Ok(()), "exactly 4x LLC must be accepted");

    // No upper bound: a bigger buffer is strictly better for this measurement.
    let huge = ProbeConfig { buffer_bytes: 1024 * MIB, ..base.clone() };
    assert_eq!(validate_config(&huge, Some(LLC)), Ok(()));

    // Unknown LLC: any non-zero buffer is accepted; the lost knowledge is
    // recorded downstream as measurement spread, not invented here.
    assert_eq!(validate_config(&below, None), Ok(()), "unknown LLC must not block a sub-floor buffer");

    // Zero is refused whatever we know about the cache, and it is a distinct
    // error: "measure nothing" is not the same complaint as "measure cache".
    let zero = ProbeConfig { buffer_bytes: 0, ..base.clone() };
    assert_eq!(validate_config(&zero, None), Err(ProbeError::ZeroBuffer));
    assert_eq!(
        validate_config(&zero, Some(LLC)),
        Err(ProbeError::ZeroBuffer),
        "the zero check must fire before the LLC comparison"
    );
}

/// AC: saturated >= single-thread, always — including degenerate sweeps.
#[test]
fn t002_saturated_is_never_below_single_thread_for_any_sweep_shape() {
    let shapes: [Vec<usize>; 5] = [vec![1], vec![1, 1, 1], vec![2], vec![8, 4, 2], vec![]];
    for threads in shapes {
        let cfg = ProbeConfig { buffer_bytes: 4 * MIB, reps: 2, thread_counts: threads.clone() };
        let r = probe(&cfg);
        assert!(
            r.saturated_gbs >= r.single_thread_gbs,
            "thread_counts {threads:?}: saturated {} < single {}",
            r.saturated_gbs,
            r.single_thread_gbs
        );
        assert!(
            r.sweep.contains_key(&1),
            "thread_counts {threads:?} must still measure one thread: {:?}",
            r.sweep
        );
        assert_eq!(r.single_thread_gbs, r.sweep[&1], "single-thread figure must be the 1-thread sample");
    }
}

/// AC (negative): a huge thread count with a tiny buffer must not panic. More
/// threads than elements is a plausible caller mistake — the probe must degrade
/// to one-element chunks and still report.
#[test]
fn t002_a_huge_thread_count_with_a_tiny_buffer_does_not_panic() {
    // 1 KiB = 128 elements, 100_000 requested threads: every worker gets exactly
    // one element. This used to be the classic divide-to-zero / empty-chunk crash.
    let cfg = ProbeConfig { buffer_bytes: 1024, reps: 3, thread_counts: vec![100_000] };
    let r = probe(&cfg);
    assert!(
        r.sweep.contains_key(&100_000),
        "the requested thread count must still be reported: {:?}",
        r.sweep.keys().collect::<Vec<_>>()
    );
    assert!(r.sweep.contains_key(&1), "the single-thread point must still be measured");
    assert!(r.saturated_gbs >= r.single_thread_gbs);
    assert!(r.single_thread_gbs > 0.0, "a tiny buffer must still measure something: {}", r.single_thread_gbs);
    for (t, gbs) in &r.sweep {
        assert!(gbs.is_finite() && *gbs >= 0.0, "thread count {t} reported {gbs}");
    }

    // Fewer bytes than one u64: the element count floors at 1 rather than 0.
    let sub_word = ProbeConfig { buffer_bytes: 1, reps: 2, thread_counts: vec![3] };
    let r = probe(&sub_word);
    assert!(r.single_thread_gbs > 0.0, "a sub-word buffer must still be measurable: {}", r.single_thread_gbs);
    assert!(r.saturated_gbs.is_finite() && r.saturated_gbs >= r.single_thread_gbs);
    assert_eq!(r.buffer_bytes, 1, "the probe must report the buffer it actually used");
}

// ===========================================================================
// T-003 — macOS collectors (crates/qinfer-core/src/collect.rs)
// ===========================================================================

/// The `Runner` trait is what makes this testable off-macOS: this fake is an
/// exact-match map over `"<program> <args joined>"`, so any casing or spacing
/// mistake in the collector's command line misses and reads as absent.
struct RecordedRunner {
    outputs: BTreeMap<String, String>,
}

impl RecordedRunner {
    fn new(pairs: &[(&str, &str)]) -> Self {
        RecordedRunner {
            outputs: pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(),
        }
    }

    fn without(mut self, command: &str) -> Self {
        self.outputs.remove(command);
        self
    }

    fn with(mut self, command: &str, output: &str) -> Self {
        self.outputs.insert(command.to_string(), output.to_string());
        self
    }
}

impl Runner for RecordedRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        self.outputs.get(&format!("{program} {}", args.join(" "))).cloned()
    }
}

/// Recorded macOS output for a machine *different from the implementers'*
/// fixture (M4 Max / 16 cores / 64 GiB vs M5 Pro / 15 cores / 24 GiB), so
/// agreement is evidence about the code and not about a copied constant.
///
/// Deliberately absent, exactly as on Apple silicon:
///   `hw.l3cachesize` (the trap this task exists to catch) and
///   `hw.optional.arm.FEAT_SME` / `SME2` / `SME2p1` / `FP8`.
const RECORDED_MACOS: &[(&str, &str)] = &[
    ("uname -m", "arm64"),
    ("uname -r", "24.2.0"),
    ("uname -n", "verify.local"),
    ("sw_vers -productVersion", "15.2"),
    ("sw_vers -buildVersion", "24C101"),
    ("sysctl -n machdep.cpu.brand_string", "Apple M4 Max"),
    ("sysctl -n hw.physicalcpu", "16"),
    ("sysctl -n hw.logicalcpu", "16"),
    ("sysctl -n hw.memsize", "68719476736"),
    ("sysctl -n hw.nperflevels", "2"),
    ("sysctl -n hw.perflevel0.name", "Performance"),
    ("sysctl -n hw.perflevel0.physicalcpu", "12"),
    ("sysctl -n hw.perflevel0.l2cachesize", "16777216"),
    ("sysctl -n hw.perflevel1.name", "Efficiency"),
    ("sysctl -n hw.perflevel1.physicalcpu", "4"),
    ("sysctl -n hw.perflevel1.l2cachesize", "4194304"),
    ("sysctl -n hw.optional.arm.FEAT_DotProd", "1"),
    ("sysctl -n hw.optional.arm.FEAT_I8MM", "1"),
    ("sysctl -n hw.optional.arm.FEAT_BF16", "1"),
    ("sysctl -n vm.swapusage", "total = 2048.00M  used = 132.50M  free = 1915.50M"),
    (
        "system_profiler SPDisplaysDataType -json",
        r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4 Max","sppci_cores":"40","spdisplays_mtlgpufamilysupport":"spdisplays_metal4","sppci_vendor":"sppci_vendor_Apple"}]}"#,
    ),
    ("pmset -g", "System-wide power settings:\nCurrently in use:\n lowpowermode         1\n"),
];

/// A T-002 result passed into the collector: the collector does not measure
/// bandwidth itself, it records what the probe said.
fn recorded_probe(spread_pct: f64) -> ProbeResult {
    let mut sweep = BTreeMap::new();
    sweep.insert(1usize, 250.0);
    sweep.insert(2usize, 380.0);
    sweep.insert(16usize, 400.0);
    ProbeResult {
        single_thread_gbs: 250.0,
        saturated_gbs: 400.0,
        median_gbs: 396.5,
        best_threads: 16,
        sweep,
        spread_pct,
        buffer_bytes: 512 * MIB,
        reps: 5,
    }
}

/// AC: the `Runner` trait makes the collector testable — a fake runner carrying
/// recorded macOS output must produce a fingerprint that parses and validates.
#[test]
fn t003_a_recorded_macos_machine_collects_into_a_valid_fingerprint() {
    let probe = recorded_probe(4.0);
    let fp = macos::collect(&RecordedRunner::new(RECORDED_MACOS), &probe)
        .expect("collect from recorded macOS output");

    // "Parses and validates": the document satisfies cfp@1 and is admissible as
    // the trust token the planner is supposed to take.
    if let Err(v) = fp.validate() {
        let codes: Vec<String> = v.iter().map(|x| format!("{}: {}", x.code(), x)).collect();
        panic!("collector output failed cfp@1 validation: {codes:#?}");
    }
    let trusted = Validated::try_from(fp.clone())
        .expect("a collector product that validates must also be constructible as Validated");

    // Identity, straight off the recorded commands.
    assert_eq!(trusted.identity.os, "macos");
    assert_eq!(trusted.identity.arch, "arm64", "uname -m");
    assert_eq!(trusted.identity.os_version.as_deref(), Some("15.2"));
    assert_eq!(trusted.identity.os_build.as_deref(), Some("24C101"));
    assert_eq!(trusted.identity.kernel.as_deref(), Some("24.2.0"));
    assert_eq!(trusted.identity.hostname.as_deref(), Some("verify.local"));

    // CPU, including the slug that the planner keys on.
    assert_eq!(trusted.cpu.brand, "Apple M4 Max");
    assert_eq!(trusted.cpu.arch_id.as_deref(), Some("apple-m4-max"));
    assert_eq!(trusted.cpu.physical_cores, 16);
    assert_eq!(trusted.cpu.logical_cores, 16);
    assert_eq!(trusted.cpu.max_freq_mhz, None);
    assert_eq!(trusted.cpu.perf_levels.len(), 2);
    assert_eq!(trusted.cpu.perf_levels[0].name.as_deref(), Some("Performance"));
    assert_eq!(trusted.cpu.perf_levels[0].physical_cores, Some(12));
    assert_eq!(trusted.cpu.perf_levels[0].l2_bytes, Some(16_777_216));
    assert_eq!(trusted.cpu.perf_levels[1].physical_cores, Some(4));
    assert_eq!(trusted.cpu.perf_levels[1].l2_bytes, Some(4_194_304));

    // GPU: the core count is a *string* in the plist.
    assert_eq!(trusted.gpu.model.as_deref(), Some("Apple M4 Max"));
    assert_eq!(trusted.gpu.cores, Some(40), "sppci_cores \"40\" must parse to 40");
    assert_eq!(trusted.gpu.metal_support.as_deref(), Some("spdisplays_metal4"));
    assert_eq!(trusted.gpu.vendor.as_deref(), Some("sppci_vendor_Apple"));
    assert!(trusted.gpu.unified_memory, "Apple silicon has unified memory");

    // Memory: bytes from sysctl, swap used converted from the MiB string.
    assert_eq!(trusted.memory.total_bytes, 68_719_476_736);
    assert_eq!(trusted.memory.swap_used_bytes, Some(138_936_320), "132.50 MiB");
    assert!(!trusted.memory_pressured(), "1915.50/2048 MiB free is healthy headroom");

    // Bandwidth: the probe's figures are carried, not re-measured.
    assert_eq!(trusted.bandwidth.single_thread_gbs, 250.0);
    assert_eq!(trusted.bandwidth.headline_gbs, 400.0);
    assert_eq!(trusted.bandwidth.headline_method.as_deref(), Some("copy"));
    assert_eq!(trusted.bandwidth.best_threads, Some(16), "the winning thread count must survive");
    assert_eq!(trusted.bandwidth.thread_sweep.len(), 3);
    assert_eq!(trusted.bandwidth.thread_sweep.get("1"), Some(&250.0));
    assert_eq!(trusted.bandwidth.thread_sweep.get("16"), Some(&400.0));
    assert_eq!(trusted.bandwidth.buffer_mib, Some(512));
    let copy = trusted.bandwidth.copy.as_ref().expect("a copy sample must be recorded");
    assert_eq!(copy.method, "copy");
    assert_eq!(copy.best_gbs, 400.0);
    assert_eq!(copy.median_gbs, 396.5);
    assert_eq!(copy.spread_pct, 4.0);
    assert_eq!(copy.reps, Some(5));
    assert_eq!(copy.buffer_bytes, Some(512 * MIB as u64));

    // Power and provenance.
    assert_eq!(trusted.power.low_power_mode, Some(true), "pmset lowpowermode 1");
    assert_eq!(trusted.provenance.get("bandwidth").map(String::as_str), Some("measured"));
    assert_eq!(trusted.provenance.get("cpu").map(String::as_str), Some("exact"));
    assert_eq!(trusted.provenance.get("thermal").map(String::as_str), Some("unknown"));

    // collected_at is a real ISO-8601 instant, not an empty string.
    assert_eq!(trusted.collected_at.len(), 20, "got {:?}", trusted.collected_at);
    assert!(trusted.collected_at.ends_with('Z'), "got {:?}", trusted.collected_at);
    assert_eq!(trusted.collected_at.as_bytes()[4], b'-', "got {:?}", trusted.collected_at);
}

/// AC (negative): absent `hw.l3cachesize` is `None`, never `0`.
#[test]
fn t003_absent_l3_cache_is_none_not_zero() {
    let fp = macos::collect(&RecordedRunner::new(RECORDED_MACOS), &recorded_probe(4.0))
        .expect("collect");
    assert_eq!(fp.cpu.l3_bytes, None, "Apple silicon has no hw.l3cachesize: None, not 0");
    assert_ne!(fp.cpu.l3_bytes, Some(0), "0 would claim a cache that does not exist");
    // The absence is isolated: every unrelated field is still fully collected,
    // so this is not the residue of a half-failed collection.
    assert_eq!(fp.cpu.brand, "Apple M4 Max");
    assert_eq!(fp.cpu.physical_cores, 16);
    assert_eq!(fp.memory.total_bytes, 68_719_476_736);
    assert_eq!(fp.cpu.perf_levels.len(), 2);
    assert_eq!(fp.validate(), Ok(()), "a legitimately absent cache must not invalidate the doc");

    // A machine that *does* report an L3 must carry the value through.
    let with_l3 = RecordedRunner::new(RECORDED_MACOS).with("sysctl -n hw.l3cachesize", "33554432");
    let fp = macos::collect(&with_l3, &recorded_probe(4.0)).expect("collect");
    assert_eq!(fp.cpu.l3_bytes, Some(33_554_432));
}

/// AC (negative): missing `hw.memsize` is `CollectError::MissingMemory`, and
/// nothing else — restoring the one key makes the collection succeed.
#[test]
fn t003_missing_memory_is_missing_memory_and_nothing_else() {
    let runner = RecordedRunner::new(RECORDED_MACOS).without("sysctl -n hw.memsize");
    assert_eq!(
        macos::collect(&runner, &recorded_probe(4.0)).unwrap_err(),
        CollectError::MissingMemory,
        "memory is the only failing field, so this must be MissingMemory"
    );
    // Isolate the cause: exactly one key restored, and the whole thing works.
    let restored = runner.with("sysctl -n hw.memsize", "68719476736");
    assert!(
        macos::collect(&restored, &recorded_probe(4.0)).is_ok(),
        "removing hw.memsize must be the sole cause of the failure"
    );
}

/// AC (negative): missing `uname -m` is `CollectError::MissingArchitecture`,
/// not a defaulted `arch`, and not an alias for the memory error.
#[test]
fn t003_missing_architecture_is_missing_architecture_and_nothing_else() {
    let runner = RecordedRunner::new(RECORDED_MACOS).without("uname -m");
    // Memory is present and readable in this very runner, so the error cannot be
    // a stand-in for a memory failure.
    assert_eq!(
        runner.run("sysctl", &["-n", "hw.memsize"]).as_deref(),
        Some("68719476736")
    );
    assert_eq!(
        macos::collect(&runner, &recorded_probe(4.0)).unwrap_err(),
        CollectError::MissingArchitecture
    );
    assert!(macos::collect(&RecordedRunner::new(RECORDED_MACOS), &recorded_probe(4.0)).is_ok());
}

/// AC (negative): malformed GPU JSON must not panic, and must not take the rest
/// of the fingerprint down with it.
#[test]
fn t003_malformed_gpu_json_does_not_panic_or_poison_the_rest() {
    // Straight at the parser, with the shapes a broken `system_profiler` emits.
    for bad in ["not json", "", "{", "{\"SPDisplaysDataType\":", "[]", "[1,2,3]", "null"] {
        let gpu = macos::parse_gpu(bad);
        assert_eq!(gpu.model, None, "input {bad:?}");
        assert_eq!(gpu.cores, None, "input {bad:?}");
        assert_eq!(gpu.metal_support, None, "input {bad:?}");
        assert!(!gpu.unified_memory, "input {bad:?} must not invent a GPU");
    }
    // A well-formed envelope with no display entry is also "no GPU", not a panic.
    let gpu = macos::parse_gpu(r#"{"SPDisplaysDataType":[]}"#);
    assert_eq!(gpu.model, None);
    assert!(!gpu.unified_memory);

    // Through the whole collector, case A: a well-formed entry whose core count
    // does not parse. The unreadable field degrades; the report survives.
    let runner = RecordedRunner::new(RECORDED_MACOS)
        .with("system_profiler SPDisplaysDataType -json", "{\"SPDisplaysDataType\":[{\"sppci_cores\": \"forty\"}]}");
    let fp = macos::collect(&runner, &recorded_probe(4.0))
        .expect("a bad GPU probe must not fail the whole collection");
    assert_eq!(fp.gpu.cores, None, "an unparseable core count must be None, not 0");
    assert_eq!(fp.gpu.model, None);
    // `system_profiler` did report a display, so this machine genuinely has a
    // unified-memory GPU; what must not happen is an invented core count.
    assert!(fp.gpu.unified_memory);
    // Unrelated violations absent: everything else is intact and still valid.
    assert_eq!(fp.memory.total_bytes, 68_719_476_736);
    assert_eq!(fp.cpu.brand, "Apple M4 Max");
    assert_eq!(fp.cpu.physical_cores, 16);
    assert_eq!(fp.bandwidth.headline_gbs, 400.0);
    assert_eq!(fp.validate(), Ok(()), "a missing GPU detail must not invalidate the document");

    // Case B: output that is not a report at all. Then there is no GPU evidence,
    // so the type must say "no GPU" rather than defaulting to something plausible.
    let runner = RecordedRunner::new(RECORDED_MACOS)
        .with("system_profiler SPDisplaysDataType -json", "system_profiler: spdisplays error");
    let fp = macos::collect(&runner, &recorded_probe(4.0))
        .expect("an unparseable GPU report must not fail the whole collection");
    assert_eq!(fp.gpu.model, None);
    assert_eq!(fp.gpu.cores, None);
    assert!(!fp.gpu.unified_memory, "no display entry means no fabricable GPU facts");
    assert_eq!(fp.cpu.brand, "Apple M4 Max", "the CPU half of the report is unaffected");
    assert_eq!(fp.validate(), Ok(()));
}

/// The GPU parser's documented landmine: `sppci_cores` is a string in the plist.
#[test]
fn t003_gpu_cores_are_parsed_from_a_whitespace_padded_string() {
    let gpu = macos::parse_gpu(
        r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4 Max","sppci_cores":" 40 "}]}"#,
    );
    assert_eq!(gpu.cores, Some(40), "surrounding whitespace must not defeat the parse");
    assert_eq!(gpu.model.as_deref(), Some("Apple M4 Max"));

    // An integer where the plist says string. This originally asserted the
    // defect (the whole display entry was dropped). Verification finding 2
    // raised it; the fix narrows the blast radius, so the assertion now pins
    // the *stronger* contract: the field coerces, and the model survives a
    // type the probe did not expect.
    let gpu = macos::parse_gpu(r#"{"SPDisplaysDataType":[{"sppci_model":"X","sppci_cores": 40}]}"#);
    assert_eq!(gpu.cores, Some(40), "a numeric core count must coerce, not vanish");
    assert_eq!(gpu.model.as_deref(), Some("X"), "one odd field must not discard the entry");

    // A genuinely unusable type loses only the field, never the record.
    let gpu = macos::parse_gpu(r#"{"SPDisplaysDataType":[{"sppci_model":"Y","sppci_cores":{"x":1}}]}"#);
    assert_eq!(gpu.cores, None, "an object is not a core count");
    assert_eq!(gpu.model.as_deref(), Some("Y"), "the model must still be reported");
}

/// AC: ISA casing — `FEAT_I8MM` is all caps, `FEAT_DotProd` is mixed, and both
/// must be detected. The fake runner is exact-match, so a lowercased probe
/// request would silently read as absent here.
#[test]
fn t003_isa_flags_are_detected_despite_inconsistent_casing() {
    let fp = macos::collect(&RecordedRunner::new(RECORDED_MACOS), &recorded_probe(4.0))
        .expect("collect");
    assert_eq!(fp.cpu.isa.get("FEAT_I8MM"), Some(&1), "all-caps key must be probed literally");
    assert_eq!(fp.cpu.isa.get("FEAT_DotProd"), Some(&1), "mixed-case key must be probed literally");
    assert_eq!(fp.cpu.isa.get("FEAT_BF16"), Some(&1));
    assert!(!fp.cpu.isa.contains_key("feat_i8mm"), "the literal key is kept, not a case-folded one");

    // Absent capability is a *fact* (0), not an omission.
    assert_eq!(fp.cpu.isa.get("FEAT_SME"), Some(&0));
    assert_eq!(fp.cpu.isa.get("FEAT_SME2"), Some(&0));
    assert_eq!(fp.cpu.isa.get("FEAT_SME2p1"), Some(&0));
    assert_eq!(fp.cpu.isa.get("FEAT_FP8"), Some(&0));
    assert_eq!(fp.cpu.isa.len(), 7, "every probe key must be reported: {:?}", fp.cpu.isa);

    // Downstream consumers see exactly the enabled set, in key order.
    assert_eq!(fp.isa_enabled(), vec!["FEAT_BF16", "FEAT_DotProd", "FEAT_I8MM"]);
}

// ===========================================================================
// T-005 — predictor (crates/qinfer-core/src/predict.rs)
// ===========================================================================

/// Observations taken from the M0 evidence of record, not from the implementer's
/// test constants: `spikes/m0/results.json` holds the measured throughput and
/// the artifact size for each of the three models actually run.
fn recorded_spike() -> SpikeResults {
    let ws = Workspace::load(spike_root());
    ws.results.expect("spikes/m0/results.json must parse — it is the M0 evidence of record")
}

fn artefact_observations() -> Vec<Observation> {
    let results = recorded_spike();
    assert_eq!(results.rows.len(), 3, "the spike measured exactly three artifacts");
    results
        .rows
        .iter()
        // `gib` is recorded to two decimals, so these byte counts are within
        // ~1% of the file sizes the spike used. Tolerances below allow for it.
        .map(|row| Observation {
            bytes_per_token: (row.gib * GIB as f64) as u64,
            measured_decode_tps: row.measured_tps,
            shape: ShapeClass::Dense,
        })
        .collect()
}

/// AC: `fit` on the three real observations reproduces the spike —
/// k_cal ~= 1.175, t0 ~= 1.22 ms, R^2 > 0.99.
#[test]
fn t005_fit_reproduces_the_spike_parameters() {
    let results = recorded_spike();
    let obs = artefact_observations();
    let cal = fit(&obs, results.bandwidth_gbs); // 227.99 GB/s, as measured

    assert!(cal.determined, "three clean observations must determine the fit");
    assert_eq!(cal.n, 3);
    assert_eq!(cal.shape, ShapeClass::Dense);

    assert!(
        (cal.k_cal - 1.175).abs() < 0.005,
        "k_cal = {} but the spike recorded 1.175",
        cal.k_cal
    );
    assert!(
        (cal.t0_ms - 1.22).abs() < 0.05,
        "t0 = {} ms but the spike recorded ~1.22 ms",
        cal.t0_ms
    );
    assert!(cal.r2 > 0.99, "R^2 = {} — the two-parameter model must explain the data", cal.r2);
}

/// The crate's fit must agree with the fit the spike itself recorded. This is
/// the cross-check that makes the test above evidence rather than round-tripping:
/// `results.json` was produced by an independent implementation (Python).
#[test]
fn t005_fit_agrees_with_the_committed_spike_record() {
    let results = recorded_spike();
    let cal = fit(&artefact_observations(), results.bandwidth_gbs);

    assert!(results.fit.determined);
    assert_eq!(results.fit.n_models as usize, cal.n, "recorded fit used {} models", results.fit.n_models);
    assert!(
        (cal.k_cal - results.fit.eps).abs() < 0.005,
        "k_cal {} vs recorded eps {}",
        cal.k_cal,
        results.fit.eps
    );
    assert!(
        (cal.t0_ms - results.fit.t0_ms).abs() < 0.03,
        "t0 {} vs recorded {}",
        cal.t0_ms,
        results.fit.t0_ms
    );
    assert!(
        (cal.r2 - results.fit.r2).abs() < 0.001,
        "R^2 {} vs recorded {}",
        cal.r2,
        results.fit.r2
    );

    let e = mape(&cal, &artefact_observations(), results.bandwidth_gbs);
    assert!(
        (e - results.mape.two_param_pct).abs() < 0.2,
        "MAPE {e:.3}% vs recorded {:.2}%",
        results.mape.two_param_pct
    );
}

/// AC: MAPE against the spike fixtures must be <= 10%.
#[test]
fn t005_mape_against_the_spike_fixtures_is_within_the_ten_percent_bar() {
    let results = recorded_spike();
    let obs = artefact_observations();
    let cal = fit(&obs, results.bandwidth_gbs);
    let e = mape(&cal, &obs, results.bandwidth_gbs);

    assert!(e <= 10.0, "MAPE {e:.3}% exceeds the 10% acceptance bar");
    assert!(e > 0.0, "a zero-error fit against its own points means the test proved nothing");
    assert!(e < 2.0, "expected the two-parameter fit near the spike's 1.45%, got {e:.3}%");
    // The model must earn its keep against the naive bytes/bandwidth roofline
    // it replaces; the spike recorded that roofline at 15.63%.
    assert!(
        e < results.mape.naive_pct / 1.5,
        "two-param {e:.2}% must materially beat the naive roofline ({:.2}%)",
        results.mape.naive_pct
    );
}

/// The mean can hide a bad row, so check every observation individually.
#[test]
fn t005_every_spike_observation_is_predicted_within_the_bar() {
    let results = recorded_spike();
    let obs = artefact_observations();
    let cal = fit(&obs, results.bandwidth_gbs);

    for (o, row) in obs.iter().zip(results.rows.iter()) {
        let p = predict(&cal, o.bytes_per_token, results.bandwidth_gbs);
        let err = (p.decode_tps.p50 - row.measured_tps).abs() / row.measured_tps * 100.0;
        assert!(
            err <= 10.0,
            "{}: predicted {} t/s vs measured {} t/s ({err:.2}% off)",
            row.model,
            p.decode_tps.p50,
            row.measured_tps
        );
        assert!(p.determined);
        assert!(
            p.decode_tps.p10 > 0.0 && p.decode_tps.p10 < p.decode_tps.p50 && p.decode_tps.p50 < p.decode_tps.p90,
            "{}: interval {:?} is not ordered",
            row.model,
            p.decode_tps
        );
        assert!(p.ms_per_token > 0.0 && p.ms_per_token.is_finite());
        // The two published numbers must be reciprocals of one another, not two
        // independently rounded opinions about the same token.
        assert!(
            (p.ms_per_token * p.decode_tps.p50 - 1000.0).abs() < 1.0,
            "{}: {} t/s and {} ms/token are not reciprocals",
            row.model,
            p.decode_tps.p50,
            p.ms_per_token
        );
    }
}

/// AC: k_cal > 1 is accepted and *used*, never clamped to 1.0.
#[test]
fn t005_k_cal_above_one_is_used_not_clamped_to_one() {
    let results = recorded_spike();
    let bw = results.bandwidth_gbs;
    let obs = artefact_observations();
    let cal = fit(&obs, bw);

    assert!(cal.k_cal > 1.0, "k_cal = {} — the reference machine calibrates above 1.0", cal.k_cal);

    // The largest artifact: what the planner would actually be asked about.
    let bytes = obs[2].bytes_per_token;
    let p = predict(&cal, bytes, bw);

    // The published p50 must be the model evaluated at k_cal, to the rounding.
    let expected = 1.0 / (cal.t0_ms / 1000.0 + bytes as f64 / (cal.k_cal * bw * 1e9));
    assert!(
        (p.decode_tps.p50 - expected).abs() < 0.02,
        "p50 {} is not 1/(t0 + bytes/(k_cal*BW)) = {:.4}",
        p.decode_tps.p50,
        expected
    );
    // And the clamp would have been materially visible, so the check has teeth.
    let if_clamped = 1.0 / (cal.t0_ms / 1000.0 + bytes as f64 / (bw * 1e9));
    assert!(
        (p.decode_tps.p50 - if_clamped).abs() / if_clamped > 0.10,
        "a clamped k_cal would predict {if_clamped:.2} t/s; the prediction ({:.2}) must differ materially",
        p.decode_tps.p50
    );
}

/// AC: fewer than 3 observations -> `determined == false`, with a *wider*
/// interval than a determined fit on the same artifact.
#[test]
fn t005_fewer_than_three_observations_is_not_a_fit_and_the_interval_says_so() {
    let results = recorded_spike();
    let bw = results.bandwidth_gbs;
    let obs = artefact_observations();
    let bytes = obs[1].bytes_per_token;

    let under = fit(&obs[..2], bw);
    assert!(!under.determined, "a two-point fit is exact and proves nothing");
    assert_eq!(under.n, 2, "the calibration must still report how much evidence it had");

    let full = fit(&obs, bw);
    assert!(full.determined);

    let pu = predict(&under, bytes, bw);
    let pd = predict(&full, bytes, bw);
    assert!(!pu.determined, "an undetermined calibration must not produce a determined prediction");
    assert!(pd.determined);
    assert!(
        pu.decode_tps.p10 < pu.decode_tps.p50 && pu.decode_tps.p50 < pu.decode_tps.p90,
        "the interval must still be ordered: {:?}",
        pu.decode_tps
    );

    // Wider by the stated factors, not merely "wider by a bit".
    assert_eq!(MIN_OBSERVATIONS, 3);
    assert!(
        (pu.decode_tps.half_width_pct() - UNCALIBRATED_SPREAD * 100.0).abs() < 0.1,
        "uncalibrated half-width {:?}% should be {}%",
        pu.decode_tps.half_width_pct(),
        UNCALIBRATED_SPREAD * 100.0
    );
    assert!(
        (pd.decode_tps.half_width_pct() - CALIBRATED_SPREAD * 100.0).abs() < 0.1,
        "calibrated half-width {:?}% should be {}%",
        pd.decode_tps.half_width_pct(),
        CALIBRATED_SPREAD * 100.0
    );
    assert!(
        pu.decode_tps.half_width_pct() > 2.0 * pd.decode_tps.half_width_pct(),
        "a fit from two points must look clearly less certain ({}% vs {}%)",
        pu.decode_tps.half_width_pct(),
        pd.decode_tps.half_width_pct()
    );
}

/// AC: KV geometry for Llama-3.3-70B (80 layers, 8 KV heads, head_dim 128).
#[test]
fn t005_kv_geometry_matches_the_prd_table() {
    const LLAMA_70B: KvGeometry = KvGeometry { layers: 80, kv_heads: 8, head_dim: 128 };

    let fp16 = LLAMA_70B.bytes_per_token(2.0);
    assert_eq!(fp16, 327_680, "2 (K and V) x 80 x 8 x 128 x 2 B");
    assert_eq!(fp16, 320 * 1024, "320 KiB per token, as the PRD table says");

    // Context totals, exactly: 320 KiB x 32k = 10 GiB, x 128k = 40 GiB.
    assert_eq!(LLAMA_70B.total_bytes(32_768, 2.0), 10 * GIB);
    assert_eq!(LLAMA_70B.total_bytes(131_072, 2.0), 40 * GIB);
}

/// AC: q8_0 KV is 53.125% of fp16, **not** 50%. The 6.25% relative error is the
/// whole reason this constant exists.
#[test]
fn t005_q8_0_kv_is_53_125_percent_of_fp16_not_half() {
    const LLAMA_70B: KvGeometry = KvGeometry { layers: 80, kv_heads: 8, head_dim: 128 };

    // 32 int8 + one fp16 scale = 34 B per 32 weights = 1.0625 B/weight.
    assert_eq!(Q8_0_BYTES_PER_ELEMENT, 1.0625);

    let fp16 = LLAMA_70B.bytes_per_token(2.0) as f64;
    let q8 = LLAMA_70B.bytes_per_token(Q8_0_BYTES_PER_ELEMENT) as f64;
    assert_eq!(q8, 174_080.0);
    assert!(
        (q8 / fp16 - 0.53125).abs() < 1e-9,
        "q8_0/fp16 = {} — expected exactly 0.53125",
        q8 / fp16
    );
    // The "half of fp16" heuristic is far enough off to be detectable, which is
    // what makes this assertion meaningful rather than pedantic.
    assert!((q8 / fp16 - 0.5).abs() > 0.03, "the 50% heuristic must be distinguishable");
}

/// AC (negative): zero bandwidth, all-zero artifact sizes and a NaN observation
/// must not panic and must not emit NaN throughput.
#[test]
fn t005_negative_inputs_do_not_panic_or_produce_nan_throughput() {
    let test_obs = artefact_observations();

    // 1. Zero bandwidth: no fit, and a bounded fallback rather than 0/0.
    let cal = fit(&test_obs, 0.0);
    assert!(!cal.determined, "a 0 GB/s probe cannot calibrate anything");
    let p = predict(&cal, test_obs[1].bytes_per_token, 0.0);
    assert!(!p.determined);
    // Verification finding 4: publishing a positive p50 from a 0 GB/s probe is a
    // fabricated number. The fix refuses explicitly instead — so the assertion
    // is now stricter, not weaker: no throughput may be invented, and the
    // refusal must be flagged rather than inferred from a magic value.
    assert!(p.refused, "a 0 GB/s probe must be refused, not silently substituted");
    assert_eq!(p.decode_tps.p50, 0.0, "must not publish throughput it cannot compute");
    assert!(p.decode_tps.p10.is_finite() && p.decode_tps.p90.is_finite());
    assert!(p.ms_per_token.is_finite(), "ms/token must stay finite on refusal");

    // 2. All-zero artifact sizes: x is constant, so no slope exists.
    let zeros: Vec<Observation> = (0..3)
        .map(|_| Observation { bytes_per_token: 0, measured_decode_tps: 10.0, shape: ShapeClass::Dense })
        .collect();
    let cal = fit(&zeros, 227.99);
    assert!(!cal.determined, "a constant regressor determines nothing");
    let p = predict(&cal, 0, 227.99);
    assert!(!p.decode_tps.p50.is_nan(), "p50 = {}", p.decode_tps.p50);
    assert!(p.decode_tps.p50.is_finite());
    assert!(p.decode_tps.p10.is_finite() && p.decode_tps.p90.is_finite());
    assert!(p.ms_per_token.is_finite(), "ms/token = {}", p.ms_per_token);

    // 3. A NaN observation is discarded, not propagated into the parameters.
    let mut poisoned = test_obs.clone();
    poisoned[1].measured_decode_tps = f64::NAN;
    let cal = fit(&poisoned, 227.99);
    assert!(!cal.determined, "two usable points plus a NaN is still not three");
    assert!(!cal.k_cal.is_nan(), "k_cal = {}", cal.k_cal);
    assert!(!cal.t0_ms.is_nan(), "t0 = {}", cal.t0_ms);
    let p = predict(&cal, test_obs[1].bytes_per_token, 227.99);
    assert!(p.decode_tps.p50.is_finite() && !p.decode_tps.p50.is_nan());
    assert!(mape(&cal, &poisoned, 227.99).is_finite(), "MAPE must survive a NaN row");

    // 4. A hand-built calibration claiming determined-but-NaN must not leak NaN
    //    into a throughput number (the fields are public, so this is reachable).
    let bogus = Calibration {
        shape: ShapeClass::Dense,
        t0_ms: 1.0,
        k_cal: f64::NAN,
        r2: 1.0,
        determined: true,
        n: 3,
    };
    let p = predict(&bogus, test_obs[0].bytes_per_token, 227.99);
    assert!(!p.decode_tps.p50.is_nan() && p.decode_tps.p50 > 0.0, "p50 = {}", p.decode_tps.p50);

    // 5. NaN / negative bandwidth are the same class of input as zero.
    for bad_bw in [f64::NAN, f64::INFINITY, -227.99] {
        let cal = fit(&test_obs, bad_bw);
        assert!(!cal.determined, "bw = {bad_bw} cannot calibrate");
        let p = predict(&cal, test_obs[1].bytes_per_token, bad_bw);
        assert!(p.refused, "bw = {bad_bw} must be refused");
        assert!(p.decode_tps.p50.is_finite(), "bw = {bad_bw} produced p50 = {}", p.decode_tps.p50);
        assert_eq!(p.decode_tps.p50, 0.0, "bw = {bad_bw} fabricated a throughput");
        assert!(p.ms_per_token.is_finite(), "bw = {bad_bw} produced ms = {}", p.ms_per_token);
    }
}

// ===========================================================================
// Cross-cutting: the trust token and the discovery path
// ===========================================================================

fn healthy_fingerprint() -> Fingerprint {
    let mut provenance = BTreeMap::new();
    provenance.insert("bandwidth".to_string(), "measured".to_string());
    provenance.insert("cpu".to_string(), "exact".to_string());
    provenance.insert("gpu".to_string(), "exact".to_string());
    provenance.insert("thermal".to_string(), "unknown".to_string());

    Fingerprint {
        schema: "qinfer/cfp@1".into(),
        collected_at: "2026-02-01T00:00:00Z".into(),
        identity: Identity {
            os: "macos".into(),
            os_version: Some("15.2".into()),
            os_build: Some("24C101".into()),
            arch: "arm64".into(),
            kernel: Some("24.2.0".into()),
            hostname: Some("verify.local".into()),
        },
        cpu: Cpu {
            brand: "Apple M4 Max".into(),
            physical_cores: 16,
            logical_cores: 16,
            arch_id: Some("apple-m4-max".into()),
            perf_levels: vec![],
            isa: BTreeMap::new(),
            l3_bytes: None,
            max_freq_mhz: None,
        },
        gpu: Gpu {
            model: Some("Apple M4 Max".into()),
            cores: Some(40),
            metal_support: Some("spdisplays_metal4".into()),
            vendor: None,
            unified_memory: true,
        },
        memory: Memory {
            total_bytes: 64 * GIB,
            // Field added by T-008 so fit arithmetic has an honest denominator;
            // this fixture predates it and states no availability.
            available_bytes: None,
            swap_used_bytes: Some(138_936_320),
            swap_raw: Some("total = 2048.00M  used = 132.50M  free = 1915.50M".into()),
        },
        bandwidth: Bandwidth {
            copy: Some(Sample {
                method: "copy".into(),
                best_gbs: 400.0,
                median_gbs: 396.5,
                spread_pct: 4.0,
                reps: Some(5),
                buffer_bytes: Some(512 * MIB as u64),
            }),
            triad: None,
            single_thread_gbs: 250.0,
            thread_sweep: BTreeMap::from([
                ("1".to_string(), 250.0),
                ("2".to_string(), 380.0),
                ("16".to_string(), 400.0),
            ]),
            best_threads: Some(16),
            headline_gbs: 400.0,
            headline_method: Some("copy".into()),
            buffer_mib: Some(512),
        },
        power: Power::default(),
        provenance,
    }
}

/// AC: `Validated<Fingerprint>` cannot be constructed from an invalid
/// fingerprint — `TryFrom` returns Err, and it returns the *actual* violations.
#[test]
fn x_validated_cannot_wrap_an_invalid_fingerprint() {
    let good = healthy_fingerprint();
    assert_eq!(good.validate(), Ok(()), "the baseline fixture must be genuinely valid");
    assert!(Validated::try_from(good.clone()).is_ok(), "a valid fingerprint must be admissible");

    // One planted defect: an unstable spread while provenance still claims a
    // measurement. The code list is asserted exactly, so this also proves no
    // unrelated rule fired on an otherwise healthy document.
    let mut unstable = good;
    unstable.bandwidth.copy = Some(Sample {
        method: "copy".into(),
        best_gbs: 400.0,
        median_gbs: 380.0,
        spread_pct: 22.0,
        reps: Some(5),
        buffer_bytes: Some(512 * MIB as u64),
    });
    let violations = Validated::try_from(unstable.clone())
        .err()
        .expect("an unstable 'measured' fingerprint must not become trusted");
    assert_eq!(
        violations.iter().map(|v| v.code()).collect::<Vec<_>>(),
        vec!["E-DET-002"],
        "exactly one, and exactly the right one: {violations:?}"
    );

    // A second, independent defect class: saturated below single-thread.
    let mut inverted = unstable;
    inverted.bandwidth.copy = Some(Sample {
        method: "copy".into(),
        best_gbs: 500.0,
        median_gbs: 499.0,
        spread_pct: 4.0,
        reps: Some(5),
        buffer_bytes: Some(512 * MIB as u64),
    });
    inverted.bandwidth.headline_gbs = 100.0;
    inverted.bandwidth.thread_sweep = BTreeMap::from([("1".to_string(), 250.0), ("16".to_string(), 100.0)]);
    let codes: Vec<&'static str> = Validated::try_from(inverted)
        .err()
        .expect("100 GB/s saturated against 250 GB/s single-thread cannot be trusted")
        .iter()
        .map(|v| v.code())
        .collect();
    assert!(codes.contains(&"E-DET-012"), "expected saturated-below-single, got {codes:?}");
    assert!(!codes.contains(&"E-DET-002"), "the spread is healthy here, so it must not fire: {codes:?}");
}

/// AC: `Workspace::load` on `spikes/m0` yields `trusted == None` with non-empty
/// violations — the committed fingerprint is known-bad and the type says so.
#[test]
fn x_workspace_load_refuses_the_committed_spike_fingerprint() {
    let ws = Workspace::load(spike_root());

    assert!(ws.error.is_none(), "the artefacts must parse; error = {:?}", ws.error);
    assert!(ws.fingerprint.is_some(), "the raw document is kept for display");
    assert!(ws.results.is_some(), "results.json is part of the evidence of record");

    // The whole point: a document that violates the invariants is exposed as an
    // untrusted display value, never as a `Validated`.
    assert!(ws.trusted.is_none(), "the committed fingerprint must never be exposed as trusted");
    assert!(!ws.is_trusted());
    assert!(!ws.violations.is_empty(), "an untrusted fingerprint must say why");

    let mut codes: Vec<&'static str> = ws.violations.iter().map(|v| v.code()).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        vec!["E-DET-002", "E-DET-002", "E-DET-003", "E-DET-009", "E-DET-017"],
        "unexpected violation set: {:#?}",
        ws.violations
    );

    // And the unrelated rules did *not* fire: the headline IS corroborated by the
    // thread sweep, the ordering is sane, and provenance is present.
    for code in [
        "E-DET-004", "E-DET-005", "E-DET-006", "E-DET-007", "E-DET-010", "E-DET-011", "E-DET-012",
        "E-DET-013", "E-DET-014", "E-DET-015", "E-DET-016", "E-DET-018",
    ] {
        assert!(!codes.contains(&code), "{code} must not fire on the committed fingerprint: {codes:?}");
    }

    // Trust is a property of the type, not of this function: the same bytes must
    // be rejected on every path in.
    let raw = std::fs::read_to_string(spike_root().join("fingerprint.json")).expect("read fixture");
    assert!(
        serde_json::from_str::<Fingerprint>(&raw).is_ok(),
        "the document itself parses — rejection is about invariants, not syntax"
    );
    assert!(
        serde_json::from_str::<Validated<Fingerprint>>(&raw).is_err(),
        "the Deserialize path must refuse what TryFrom refuses"
    );
}

/// The collector's JSON shape is the same document the loader accepts — the two
/// halves of M0 must not drift apart.
#[test]
fn x_a_collector_shaped_document_parses_and_validates() {
    const DOC: &str = r#"{
      "schema": "qinfer/cfp@1",
      "collected_at": "2026-02-01T00:00:00Z",
      "identity": {"os": "macos", "arch": "arm64", "os_version": "15.2"},
      "cpu": {"brand": "Apple M4 Max", "physical_cores": 16, "logical_cores": 16,
              "arch_id": "apple-m4-max", "l3_bytes": null, "isa": {"FEAT_I8MM": 1}},
      "gpu": {"model": "Apple M4 Max", "cores": 40, "unified_memory": true},
      "memory": {"total_bytes": 68719476736, "swap_used_bytes": 138936320,
                 "swap_raw": "total = 2048.00M  used = 132.50M  free = 1915.50M"},
      "bandwidth": {
        "copy": {"method": "copy", "best_gbs": 400.0, "median_gbs": 396.5,
                 "spread_pct": 4.0, "reps": 5, "buffer_bytes": 536870912},
        "single_thread_gbs": 250.0,
        "thread_sweep": {"1": 250.0, "2": 380.0, "16": 400.0},
        "best_threads": 16,
        "headline_gbs": 400.0,
        "headline_method": "copy",
        "buffer_mib": 512
      },
      "provenance": {"bandwidth": "measured", "cpu": "exact"}
    }"#;

    let fp: Validated<Fingerprint> =
        serde_json::from_str(DOC).expect("a collector-shaped document must parse and validate");
    assert_eq!(fp.bandwidth.headline_gbs, 400.0);
    assert_eq!(fp.bandwidth.best_threads, Some(16));
    assert_eq!(fp.bandwidth.thread_sweep.len(), 3);
    assert_eq!(fp.cpu.l3_bytes, None);
    assert_eq!(fp.cpu.isa.get("FEAT_I8MM"), Some(&1));
    assert_eq!(fp.gpu.cores, Some(40));

    // Same document, one BAD field: rejected, and with exactly one complaint.
    let bad = DOC.replace("\"spread_pct\": 4.0", "\"spread_pct\": 40.0");
    assert_ne!(bad, DOC, "the mutation must actually apply");
    let err = serde_json::from_str::<Validated<Fingerprint>>(&bad)
        .err()
        .expect("a 40% spread claimed as measured must be refused");
    let msg = err.to_string();
    assert!(msg.contains("E-DET-002"), "message: {msg}");
    assert_eq!(
        msg.matches("E-DET-").count(),
        1,
        "exactly one violation expected, got: {msg}"
    );
}
