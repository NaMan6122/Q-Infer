//! A plannable session: one fingerprint, one catalogue, one calibration.
//!
//! This exists so the CLI and the dashboard cannot drift apart. Both need to
//! answer "what should run on this machine?", and both must derive the answer
//! the same way — from the catalogue's own measurements, never from a separate
//! copy of the numbers (ADR-0002).

use crate::catalog::{Catalog, CatalogViolation};
use crate::planner::{Constraints, PlanRequest, PlanSet, Planner};
use crate::predict::{fit, Calibration, Observation};
use crate::space::{self, DeviceInfo, Space};
use crate::Fingerprint;

/// Fraction of RAM we assume usable when the fingerprint does not say.
///
/// Re-exported rather than redefined: the fallback now lives with the rest of
/// the memory-budget logic in [`crate::space`], so there is exactly one place
/// that decides what an assumption looks like (T-012).
pub use crate::space::ASSUMED_AVAILABLE_FRACTION;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    InvalidCatalog(Vec<CatalogViolation>),
    /// A zero or non-finite bandwidth means the probe failed; planning on it
    /// would publish numbers that look like predictions and are not.
    UnusableBandwidth { value: f64 },
    /// Fewer than three on-device measurements cannot calibrate anything.
    InsufficientMeasurements { found: usize },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::InvalidCatalog(v) => write!(
                f,
                "catalogue invalid: {}",
                v.first().map(|x| x.to_string()).unwrap_or_default()
            ),
            SessionError::UnusableBandwidth { value } => {
                write!(f, "bandwidth {value} is unusable — re-run the probe")
            }
            SessionError::InsufficientMeasurements { found } => write!(
                f,
                "{found} on-device measurements; three are needed before a prediction means anything"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    fingerprint: Fingerprint,
    catalog: Catalog,
    calibration: Calibration,
    /// How much memory this machine will actually give a workload. Resolved once,
    /// so every surface quotes the same budget (T-012).
    space: Space,
}

impl Session {
    pub fn new(fingerprint: Fingerprint, catalog: Catalog) -> Result<Self, SessionError> {
        Self::with_device(fingerprint, catalog, None)
    }

    /// Build a session with the GPU's own report of its working-set ceiling.
    ///
    /// `device` is `None` on a machine with no GPU probe or on CI, which is a
    /// normal input and degrades through `space`'s documented chain. Passing a
    /// report here is what lets a 24 GiB machine plan a 17 GiB model instead of
    /// refusing it at half of RAM.
    pub fn with_device(
        fingerprint: Fingerprint,
        catalog: Catalog,
        device: Option<&DeviceInfo>,
    ) -> Result<Self, SessionError> {
        if let Err(v) = catalog.validate() {
            return Err(SessionError::InvalidCatalog(v));
        }

        let bw = fingerprint.saturated_gbs();
        if !(bw > 0.0) || !bw.is_finite() {
            return Err(SessionError::UnusableBandwidth { value: bw });
        }

        let observations = observations_from(&catalog, bw);
        if observations.len() < crate::predict::MIN_OBSERVATIONS {
            return Err(SessionError::InsufficientMeasurements { found: observations.len() });
        }

        let space = space::resolve(&fingerprint, device);
        let calibration = fit(&observations, bw);
        Ok(Session { fingerprint, catalog, calibration, space })
    }

    /// Build a session from already-measured observations, without a catalogue
    /// that carries them. Used by `detect`-only flows and by tests.
    pub fn from_observations(
        fingerprint: Fingerprint,
        catalog: Catalog,
        observations: &[Observation],
    ) -> Result<Self, SessionError> {
        let bw = fingerprint.saturated_gbs();
        if !(bw > 0.0) || !bw.is_finite() {
            return Err(SessionError::UnusableBandwidth { value: bw });
        }
        if observations.len() < crate::predict::MIN_OBSERVATIONS {
            return Err(SessionError::InsufficientMeasurements { found: observations.len() });
        }
        let space = space::resolve(&fingerprint, None);
        let calibration = fit(observations, bw);
        Ok(Session { fingerprint, catalog, calibration, space })
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// The resolved memory budget, so the CLI, the dashboard and the planner all
    /// quote the same number for the same machine.
    pub fn space(&self) -> &Space {
        &self.space
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    pub fn bandwidth_gbs(&self) -> f64 {
        self.fingerprint.saturated_gbs()
    }

    /// The architecture the planner keys coverage on. Older fingerprints predate
    /// `arch_id`, so it is derived from the brand rather than left unknown.
    pub fn arch_id(&self) -> String {
        self.fingerprint
            .cpu
            .arch_id
            .clone()
            .unwrap_or_else(|| crate::collect::slugify(&self.fingerprint.cpu.brand))
    }

    /// Bytes the planner may use: the GPU's working-set ceiling when we have it,
    /// otherwise the documented fallback chain. Never silently half of RAM.
    pub fn available_bytes(&self) -> u64 {
        self.space.plan_budget()
    }

    /// True when the budget rests on a stated assumption rather than on
    /// something the operating system told us.
    ///
    /// Deliberately keyed on the *source*, not on pressure: "is this an
    /// assumption?" and "is this figure trustworthy right now?" are different
    /// questions, and `space.provenance()` answers the second.
    pub fn availability_is_assumed(&self) -> bool {
        !self.space.source.is_measured()
    }

    pub fn plans(&self, req: &PlanRequest) -> PlanSet {
        let mut constraints = Constraints::new(self.arch_id(), self.available_bytes());
        if self.fingerprint.bandwidth.copy.is_some() {
            constraints.kv_bytes_per_element = 2.0;
        }
        let planner = Planner::new(&self.catalog, &self.calibration, self.bandwidth_gbs());
        planner.plan(req, &constraints)
    }

    /// A one-line provenance string for the calibration, used by every surface
    /// so the same numbers are described the same way.
    pub fn calibration_summary(&self) -> String {
        format!(
            "t₀ {:.2} ms · k_cal {:.3} · R² {:.4} · n={}",
            self.calibration.t0_ms, self.calibration.k_cal, self.calibration.r2, self.calibration.n
        )
    }
}

/// Every artifact we have both a size and a measurement for becomes one
/// observation. This is what keeps the dashboard and the CLI honest: they
/// calibrate from the catalogue, not from a stored copy of the spike results.
pub fn observations_from(catalog: &Catalog, _bw: f64) -> Vec<Observation> {
    catalog
        .entries
        .iter()
        .filter(|e| e.is_plannable())
        .filter_map(|e| {
            e.measurements.first().map(|m| Observation {
                bytes_per_token: e.bytes_per_token(),
                measured_decode_tps: m.decode_tps,
                shape: e.model.shape,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predict::ShapeClass;

    const SEED: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.27.json");

    fn fingerprint(available: Option<u64>) -> Fingerprint {
        let raw = include_str!("../../../spikes/m0/fixtures/fingerprint.json");
        let mut fp: Fingerprint = serde_json::from_str(raw).expect("fingerprint");
        fp.bandwidth.headline_gbs = 227.99;
        fp.bandwidth.single_thread_gbs = 131.4;
        fp.memory.available_bytes = available;
        fp
    }

    fn catalog() -> Catalog {
        Catalog::from_json(SEED).expect("catalogue")
    }

    #[test]
    fn builds_a_session_from_the_catalogue_alone() {
        let s = Session::new(fingerprint(None), catalog()).expect("session");
        assert_eq!(s.calibration().n, 5, "five measured artifacts are in the catalogue");
        assert!(s.calibration().determined);
        assert!((s.calibration().k_cal - 1.175).abs() < 0.06, "k_cal = {}", s.calibration().k_cal);
    }

    #[test]
    fn arch_id_is_derived_when_the_fingerprint_predates_it() {
        let mut fp = fingerprint(None);
        fp.cpu.arch_id = None;
        let s = Session::new(fp, catalog()).expect("session");
        assert_eq!(s.arch_id(), "apple-m5-pro", "derived from the brand, not left unknown");
    }

    #[test]
    fn availability_falls_back_to_a_stated_fraction() {
        let s = Session::new(fingerprint(None), catalog()).expect("session");
        assert!(s.availability_is_assumed());
        assert_eq!(s.available_bytes(), s.fingerprint.memory.total_bytes / 2);

        let s = Session::new(fingerprint(Some(6 << 30)), catalog()).expect("session");
        assert!(!s.availability_is_assumed());
        assert_eq!(s.available_bytes(), 6 << 30);
    }

    #[test]
    fn an_unusable_bandwidth_is_refused_not_defaulted() {
        let mut fp = fingerprint(None);
        fp.bandwidth.headline_gbs = 0.0;
        let err = Session::new(fp, catalog()).unwrap_err();
        assert!(matches!(err, SessionError::UnusableBandwidth { .. }));
    }

    #[test]
    fn too_few_measurements_cannot_calibrate() {
        let mut c = catalog();
        for e in c.entries.iter_mut() {
            e.measurements.clear();
        }
        let err = Session::new(fingerprint(None), c).unwrap_err();
        assert_eq!(err, SessionError::InsufficientMeasurements { found: 0 });
    }

    #[test]
    fn an_invalid_catalogue_is_refused() {
        let mut c = catalog();
        c.entries[0].artifact.licence = String::new();
        let err = Session::new(fingerprint(None), c).unwrap_err();
        assert!(matches!(err, SessionError::InvalidCatalog(_)));
    }

    #[test]
    fn plans_come_back_ranked_and_explained() {
        let s = Session::new(fingerprint(Some(12 << 30)), catalog()).expect("session");
        let set = s.plans(&PlanRequest::default());
        assert!(!set.plans.is_empty());
        assert!(set.plans[0].rationale.len() >= 4);
        assert_eq!(set.plans[0].predicted.p50, set.plans[0].predicted.p50); // not NaN
    }

    #[test]
    fn observations_use_active_bytes_for_moe() {
        let c = catalog();
        let obs = observations_from(&c, 227.99);
        let moe = c.entries.iter().find(|e| e.id == "olmoe-1b-7b-q4_k_m").unwrap();
        let expected = moe.bytes_per_token();
        assert!(obs.iter().any(|o| o.bytes_per_token == expected && o.shape == ShapeClass::Moe));
    }
}
