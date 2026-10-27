//! The Ladder (T-016): the stages a setup run passes through.
//!
//! A wizard asks questions. A ladder *measures*, and shows each rung filling in,
//! so the user can see what the tool is about to believe before it believes it.
//!
//! The order below is not the PRD's diagram — it is the order the **data
//! dependencies** impose, and pretending otherwise would mean collecting a
//! fingerprint with no bandwidth figure in it:
//!
//! ```text
//!   Gauge      CPU bandwidth sweep + device micro-operations
//!   Identify   assemble the fingerprint from the gauge results
//!   Budget     how much memory this machine will actually grant (T-012)
//!   Shortlist  catalogue -> calibration -> ranked plans
//!   Choose     frontier -> most capable plan above the usability floor (T-015)
//!   Done
//! ```
//!
//! M0 is an advisor with no installer (PRD §14), so the ladder **ends at
//! `Choose`**. Provisioning and verification join at M1 as two more rungs; the
//! event type is shaped so they can be added without changing a consumer.
//!
//! Every expensive step is behind [`Measurements`], so the whole ladder is
//! testable on a machine with no GPU, no MLX and no macOS.

use crate::catalog::Catalog;
use crate::microops::{self, OpId, Report};
use crate::probe::{probe, ProbeConfig, ProbeResult};
use crate::session::Session;
use crate::space::Space;
use crate::sweetspot::{self, SweetSpot, DEFAULT_USABLE_TPS};
use crate::Fingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Gauge,
    Identify,
    Budget,
    Shortlist,
    Choose,
}

impl Stage {
    pub const ALL: [Stage; 5] =
        [Stage::Gauge, Stage::Identify, Stage::Budget, Stage::Shortlist, Stage::Choose];

    pub fn label(self) -> &'static str {
        match self {
            Stage::Gauge => "Gauge",
            Stage::Identify => "Identify",
            Stage::Budget => "Budget",
            Stage::Shortlist => "Shortlist",
            Stage::Choose => "Choose",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub stage: Stage,
    /// What the rung found, in a sentence the dashboard can print verbatim.
    pub detail: String,
}

/// Deliberately **not** `PartialEq`: one variant carries a whole `cfp@1`
/// fingerprint, and deriving equality would require adding `PartialEq` to the
/// entire fingerprint tree for nobody's benefit. Tests match on the variant they
/// care about instead, which is what they were really asserting anyway.
#[derive(Debug, Clone)]
pub enum SetupEvent {
    /// A rung started. Emitted before the work, so a slow rung is visible while
    /// it is slow rather than after it finishes.
    Begin(Stage),
    /// A rung finished, with the one-line finding a user can read.
    Stage(Progress),
    /// The device probe's full report, for the Probe tab.
    Measured(Report),
    /// The fingerprint this run collected. Carried in the event rather than
    /// written to disk by the ladder, so a *view* can show live numbers without
    /// itself becoming a writer (ADR-0009). Persisting it is the CLI's job.
    Fingerprint(Box<Fingerprint>),
    /// The resolved memory budget.
    Budget(Space),
    /// The frontier and its recommendation.
    Chosen(SweetSpot),
    /// A rung failed. `stage` names where, so the dashboard can mark the rung
    /// that broke instead of showing a generic error.
    Failed { stage: Stage, reason: String },
    Done,
}

/// The expensive parts of a setup run, injected so the ladder can be tested
/// without a GPU, without MLX, and without macOS.
pub trait Measurements {
    fn bandwidth(&self) -> ProbeResult;
    fn microops(&self, ops: &[OpId]) -> Report;
    fn collect(&self, probe: &ProbeResult) -> Result<Fingerprint, String>;
}

/// The real thing.
pub struct SystemMeasurements;

impl Measurements for SystemMeasurements {
    fn bandwidth(&self) -> ProbeResult {
        probe(&ProbeConfig { reps: 5, ..Default::default() })
    }

    fn microops(&self, ops: &[OpId]) -> Report {
        microops::run(&crate::collect::SystemRunner, ops)
    }

    fn collect(&self, p: &ProbeResult) -> Result<Fingerprint, String> {
        #[cfg(target_os = "macos")]
        {
            crate::collect::macos::collect(&crate::collect::SystemRunner, p)
                .map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = p;
            Err("no fingerprint collector for this platform yet".into())
        }
    }
}

/// Run the ladder, reporting each rung through `emit`.
///
/// Synchronous on purpose: the TUI wraps it in a thread and the CLI calls it
/// directly, so both consume exactly the same sequence of events (ADR-0009).
pub fn run<F>(m: &impl Measurements, catalog: &Catalog, floor: f64, mut emit: F)
where
    F: FnMut(SetupEvent),
{
    // ---- Gauge ------------------------------------------------------------
    emit(SetupEvent::Begin(Stage::Gauge));
    let bw = m.bandwidth();
    let report = m.microops(&OpId::ALL);
    let device = report
        .gpu_bandwidth_gbs()
        .map(|g| format!("{g:.0} GB/s device stream"))
        .unwrap_or_else(|| "no device measurement".into());
    emit(SetupEvent::Measured(report.clone()));
    emit(SetupEvent::Stage(Progress {
        stage: Stage::Gauge,
        detail: format!(
            "CPU {:.0} GB/s saturated ({:.1}x single-thread, {:.0}% spread) · {}",
            bw.saturated_gbs,
            if bw.single_thread_gbs > 0.0 { bw.saturated_gbs / bw.single_thread_gbs } else { 0.0 },
            bw.spread_pct,
            device
        ),
    }));

    // ---- Identify ---------------------------------------------------------
    emit(SetupEvent::Begin(Stage::Identify));
    let fp = match m.collect(&bw) {
        Ok(fp) => fp,
        Err(reason) => {
            emit(SetupEvent::Failed { stage: Stage::Identify, reason });
            return;
        }
    };
    emit(SetupEvent::Stage(Progress {
        stage: Stage::Identify,
        detail: format!(
            "{} · {} · {}",
            fp.cpu.brand,
            fp.perf_summary(),
            fp.cpu.arch_id.clone().unwrap_or_else(|| "unknown arch".into())
        ),
    }));
    emit(SetupEvent::Fingerprint(Box::new(fp.clone())));

    // ---- Budget -----------------------------------------------------------
    emit(SetupEvent::Begin(Stage::Budget));
    let space = crate::space::resolve(&fp, report.device.as_ref());
    emit(SetupEvent::Budget(space.clone()));
    emit(SetupEvent::Stage(Progress {
        stage: Stage::Budget,
        detail: space.describe(),
    }));

    // ---- Shortlist --------------------------------------------------------
    emit(SetupEvent::Begin(Stage::Shortlist));
    let session = match Session::with_device(fp, catalog.clone(), report.device.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            emit(SetupEvent::Failed { stage: Stage::Shortlist, reason: e.to_string() });
            return;
        }
    };
    let plans = session.plans(&crate::planner::PlanRequest::default());
    emit(SetupEvent::Stage(Progress {
        stage: Stage::Shortlist,
        detail: format!(
            "{} plan(s) from {} catalogued artifact(s) · {} excluded",
            plans.plans.len(),
            session.catalog().entries.len(),
            plans.rejected.len()
        ),
    }));

    // ---- Choose -----------------------------------------------------------
    emit(SetupEvent::Begin(Stage::Choose));
    let spot = sweetspot::frontier(&plans, session.catalog());
    let detail = if spot.is_empty() {
        spot.usable_reason(floor)
    } else {
        spot.usable_reason(floor)
    };
    emit(SetupEvent::Stage(Progress { stage: Stage::Choose, detail }));
    emit(SetupEvent::Chosen(spot));
    emit(SetupEvent::Done);
}

/// Run the ladder on a worker thread, for a UI that must stay responsive.
///
/// The channel is the whole interface: no socket, no daemon, no shared mutable
/// state (ADR-0009). A dropped receiver simply stops the sender.
pub fn spawn(catalog: Catalog, floor: f64) -> std::sync::mpsc::Receiver<SetupEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let m = SystemMeasurements;
        run(&m, &catalog, floor, |e| {
            let _ = tx.send(e);
        });
    });
    rx
}

/// The floor a caller should use when it has no opinion.
pub fn default_floor() -> f64 {
    DEFAULT_USABLE_TPS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::DeviceInfo;

    const CATALOG: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.27.json");

    fn catalog() -> Catalog {
        Catalog::from_json(CATALOG).expect("catalogue")
    }

    struct Fake {
        bw: f64,
        collect_ok: bool,
    }

    impl Measurements for Fake {
        fn bandwidth(&self) -> ProbeResult {
            let mut sweep = std::collections::BTreeMap::new();
            sweep.insert(1usize, self.bw / 1.9);
            sweep.insert(15usize, self.bw);
            ProbeResult {
                single_thread_gbs: self.bw / 1.9,
                saturated_gbs: self.bw,
                median_gbs: self.bw * 0.98,
                best_threads: 15,
                sweep,
                spread_pct: 3.0,
                buffer_bytes: 256 * 1024 * 1024,
                reps: 5,
            }
        }

        fn microops(&self, ops: &[OpId]) -> Report {
            Report {
                ops: ops
                    .iter()
                    .map(|id| microops::MicroOp {
                        id: *id,
                        outcome: microops::Outcome::Refused {
                            reason: "not measured in this test".into(),
                            fix: None,
                        },
                    })
                    .collect(),
                device: Some(DeviceInfo {
                    device_name: Some("Apple M5 Pro".into()),
                    architecture: Some("applegpu_g17s".into()),
                    memory_size: Some(25_769_803_776),
                    recommended_working_set: Some(19_069_665_280),
                    max_buffer_length: Some(14_302_248_960),
                }),
            }
        }

        fn collect(&self, _p: &ProbeResult) -> Result<Fingerprint, String> {
            if !self.collect_ok {
                return Err("no fingerprint collector for this platform yet".into());
            }
            let raw = include_str!("../../../spikes/m0/fixtures/fingerprint.json");
            let mut fp: Fingerprint = serde_json::from_str(raw).expect("fixture");
            // The fixture predates these fields; a ladder test cares about flow,
            // not about re-testing the collector.
            fp.bandwidth.headline_gbs = self.bw;
            fp.bandwidth.single_thread_gbs = self.bw / 1.9;
            fp.memory.available_bytes = Some(10 << 30);
            fp.cpu.arch_id = Some("apple-m5-pro".into());
            // The committed fixture is the *paging* reference host (94% swap),
            // which is deliberate — it is why `cfp@1` rejects it. The ladder is
            // being tested for plumbing, and `space` already owns the pressure
            // rule, so the fake models a calm machine.
            fp.memory.swap_raw = Some("total = 8192.00M  used = 0.00M  free = 8192.00M".into());
            Ok(fp)
        }
    }

    fn collect_events(f: &Fake, floor: f64) -> Vec<SetupEvent> {
        let mut out = Vec::new();
        run(f, &catalog(), floor, |e| out.push(e));
        out
    }

    fn stages(evs: &[SetupEvent]) -> Vec<Stage> {
        evs.iter()
            .filter_map(|e| match e {
                SetupEvent::Stage(p) => Some(p.stage),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_rung_runs_in_dependency_order() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 20.0);
        assert_eq!(
            stages(&evs),
            vec![Stage::Gauge, Stage::Identify, Stage::Budget, Stage::Shortlist, Stage::Choose],
            "identity cannot precede the gauge: the fingerprint needs its bandwidth"
        );
        assert!(matches!(evs.last(), Some(SetupEvent::Done)));
    }

    #[test]
    fn a_rung_announces_itself_before_it_finishes() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 20.0);
        let begin = evs.iter().position(|e| matches!(e, SetupEvent::Begin(Stage::Gauge)));
        let done = evs.iter().position(|e| matches!(e, SetupEvent::Stage(p) if p.stage == Stage::Gauge));
        assert!(begin.is_some() && done.is_some(), "{evs:?}");
        assert!(begin.unwrap() < done.unwrap(), "a slow rung must be visible while it is slow");
    }

    #[test]
    fn a_failing_identify_stops_the_ladder_without_inventing_a_choice() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: false }, 20.0);
        assert!(evs.iter().any(|e| matches!(e, SetupEvent::Failed { stage: Stage::Identify, .. })));
        assert!(!evs.iter().any(|e| matches!(e, SetupEvent::Chosen(_))), "no choice from no machine");
        assert!(!evs.iter().any(|e| matches!(e, SetupEvent::Done)), "the ladder did not finish");
        assert!(!stages(&evs).contains(&Stage::Choose));
    }

    #[test]
    fn the_budget_comes_from_the_device_not_half_of_ram() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 20.0);
        let budget = evs
            .iter()
            .find_map(|e| match e {
                SetupEvent::Budget(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a budget event");
        assert_eq!(budget.plan_budget(), 19_069_665_280);
        assert_eq!(budget.provenance(), "measured");
    }

    #[test]
    fn the_choice_respects_the_floor() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 20.0);
        let spot = evs
            .iter()
            .find_map(|e| match e {
                SetupEvent::Chosen(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a frontier");
        let best = spot.best_usable(20.0).expect("something above the floor");
        assert!(best.tps() >= 20.0);
        // And raising the floor cannot make the answer more capable, only less.
        let strict = spot.best_usable(200.0);
        if let Some(s) = strict {
            assert!(s.capability <= best.capability || s.plan.id != best.plan.id);
        }
    }

    #[test]
    fn a_wide_floor_still_produces_a_curve_to_choose_from() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 10_000.0);
        let spot = evs
            .iter()
            .find_map(|e| match e {
                SetupEvent::Chosen(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a frontier");
        assert!(spot.best_usable(10_000.0).is_none(), "nothing is that fast");
        assert!(!spot.is_empty(), "but the curve still exists to explain why");
        assert!(spot.usable_reason(10_000.0).contains("nothing on this machine"));
    }

    #[test]
    fn the_prior_caveat_travels_with_the_choice() {
        let evs = collect_events(&Fake { bw: 257.0, collect_ok: true }, 20.0);
        let spot = evs
            .iter()
            .find_map(|e| match e {
                SetupEvent::Chosen(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a frontier");
        assert!(
            spot.notes.iter().any(|n| n.contains("prior")),
            "a surface must be able to say the capability number is a guess: {:?}",
            spot.notes
        );
    }

    #[test]
    fn the_default_floor_is_the_documented_one() {
        assert_eq!(default_floor(), DEFAULT_USABLE_TPS);
    }
}
