//! The catalogue (T-004).
//!
//! The planner can only choose between artifacts it can describe precisely:
//! how big, how many parameters are actually read per token, how much KV a
//! context costs, what licence it carries, and — critically — **which
//! architectures it can actually execute on**.
//!
//! Three rules are enforced here rather than documented:
//!
//! 1. **An artifact we possess is content-addressed.** If the byte count came
//!    from a file we downloaded, a SHA-256 is mandatory. Artifacts we have only
//!    estimated are allowed to lack one — but they are not plannable.
//! 2. **Coverage is per artifact, not per version** (ADR-0004). An entry lists
//!    the architectures it has been verified or declared on, and the planner
//!    may only offer it where coverage exists.
//! 3. **Provenance is explicit.** A size is `measured`, `publisher` or
//!    `estimated`; there is no default. A number without a source is exactly
//!    the plausible default ADR-0002 forbids.

use crate::predict::{KvGeometry, ShapeClass};
use serde::Deserialize;

pub const CATALOG_SCHEMA_PREFIX: &str = "qinfer/catalog@";
pub const SUPPORTED_CATALOG_MAJOR: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub schema: String,
    pub version: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub note: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub family: String,
    #[serde(default)]
    pub display: String,
    pub artifact: Artifact,
    pub model: ModelMeta,
    #[serde(default)]
    pub coverage: Vec<Coverage>,
    #[serde(default)]
    pub measurements: Vec<Measurement>,
    #[serde(default)]
    pub quality: Option<Quality>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub format: String,
    pub quant: String,
    pub bytes: u64,
    pub bytes_source: ByteSource,
    #[serde(default)]
    pub sha256: Option<String>,
    pub licence: String,
    #[serde(default)]
    pub gated: bool,
    #[serde(default)]
    pub publisher: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ByteSource {
    /// We downloaded the file and hashed it.
    Measured,
    /// Taken from the publisher's model card / manifest.
    Publisher,
    /// Derived from params × bits-per-param. Never plannable.
    Estimated,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelMeta {
    pub total_params: u64,
    pub active_params: u64,
    pub shape: ShapeClass,
    pub kv: KvGeometry,
    pub ctx_max: u32,
    #[serde(default)]
    pub hidden: u32,
    #[serde(default)]
    pub vocab: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Coverage {
    pub arch_id: String,
    pub backend: String,
    pub status: CoverageStatus,
    #[serde(default)]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverageStatus {
    /// We ran it here and have a measurement.
    Verified,
    /// The publisher or a third party claims it works.
    Declared,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Measurement {
    pub backend: String,
    pub decode_tps: f64,
    #[serde(default)]
    pub prefill_tps: Option<f64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Quality {
    pub metric: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub method: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CatalogViolation {
    UnknownSchema { found: String },
    EmptyCatalog,
    DuplicateId { id: String },
    EmptyId { index: usize },
    ZeroBytes { id: String },
    MissingHashForMeasured { id: String },
    MissingLicence { id: String },
    NoCoverage { id: String },
    DegenerateKv { id: String },
    ActiveExceedsTotal { id: String },
    MoeWithoutSparsity { id: String },
}

impl CatalogViolation {
    pub fn code(&self) -> &'static str {
        match self {
            CatalogViolation::UnknownSchema { .. } => "E-CAT-001",
            CatalogViolation::EmptyCatalog => "E-CAT-002",
            CatalogViolation::DuplicateId { .. } => "E-CAT-003",
            CatalogViolation::EmptyId { .. } => "E-CAT-004",
            CatalogViolation::ZeroBytes { .. } => "E-CAT-005",
            CatalogViolation::MissingHashForMeasured { .. } => "E-CAT-006",
            CatalogViolation::MissingLicence { .. } => "E-CAT-007",
            CatalogViolation::NoCoverage { .. } => "E-CAT-008",
            CatalogViolation::DegenerateKv { .. } => "E-CAT-009",
            CatalogViolation::ActiveExceedsTotal { .. } => "E-CAT-010",
            CatalogViolation::MoeWithoutSparsity { .. } => "E-CAT-011",
        }
    }
}

impl std::fmt::Display for CatalogViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogViolation::UnknownSchema { found } => write!(f, "catalogue schema {found} is not supported"),
            CatalogViolation::EmptyCatalog => write!(f, "catalogue has no entries"),
            CatalogViolation::DuplicateId { id } => write!(f, "duplicate entry id {id:?}"),
            CatalogViolation::EmptyId { index } => write!(f, "entry {index} has an empty id"),
            CatalogViolation::ZeroBytes { id } => write!(f, "{id}: artifact size must be non-zero"),
            CatalogViolation::MissingHashForMeasured { id } => write!(
                f,
                "{id}: size is `measured` but no SHA-256 is recorded — an artifact we possess must be content-addressed"
            ),
            CatalogViolation::MissingLicence { id } => write!(f, "{id}: no licence recorded"),
            CatalogViolation::NoCoverage { id } => write!(
                f,
                "{id}: no architecture coverage — the planner could never offer it"
            ),
            CatalogViolation::DegenerateKv { id } => write!(
                f,
                "{id}: KV geometry has a zero dimension"
            ),
            CatalogViolation::ActiveExceedsTotal { id } => {
                write!(f, "{id}: active_params exceeds total_params")
            }
            CatalogViolation::MoeWithoutSparsity { id } => write!(
                f,
                "{id}: declared `moe` but active_params == total_params (no sparsity)"
            ),
        }
    }
}

impl Catalog {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn load(path: &std::path::Path) -> std::io::Result<Result<Self, serde_json::Error>> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::from_json(&text))
    }

    /// Returns every violation, not just the first.
    pub fn validate(&self) -> Result<(), Vec<CatalogViolation>> {
        let mut v = Vec::new();

        let major = self
            .schema
            .strip_prefix(CATALOG_SCHEMA_PREFIX)
            .and_then(|rest| rest.split('.').next())
            .and_then(|m| m.parse::<u32>().ok());
        if major.map_or(true, |m| m > SUPPORTED_CATALOG_MAJOR) {
            v.push(CatalogViolation::UnknownSchema { found: self.schema.clone() });
        }

        if self.entries.is_empty() {
            v.push(CatalogViolation::EmptyCatalog);
        }

        let mut seen = std::collections::BTreeSet::new();
        for (i, e) in self.entries.iter().enumerate() {
            if e.id.trim().is_empty() {
                v.push(CatalogViolation::EmptyId { index: i });
            } else if !seen.insert(e.id.clone()) {
                v.push(CatalogViolation::DuplicateId { id: e.id.clone() });
            }

            if e.artifact.bytes == 0 {
                v.push(CatalogViolation::ZeroBytes { id: e.id.clone() });
            }
            if e.artifact.bytes_source == ByteSource::Measured && e.artifact.sha256.is_none() {
                v.push(CatalogViolation::MissingHashForMeasured { id: e.id.clone() });
            }
            if e.artifact.licence.trim().is_empty() {
                v.push(CatalogViolation::MissingLicence { id: e.id.clone() });
            }
            if e.coverage.is_empty() {
                v.push(CatalogViolation::NoCoverage { id: e.id.clone() });
            }

            let kv = e.model.kv;
            if kv.layers == 0 || kv.kv_heads == 0 || kv.head_dim == 0 {
                v.push(CatalogViolation::DegenerateKv { id: e.id.clone() });
            }
            if e.model.active_params > e.model.total_params {
                v.push(CatalogViolation::ActiveExceedsTotal { id: e.id.clone() });
            }
            if e.model.shape == ShapeClass::Moe && e.model.active_params == e.model.total_params {
                v.push(CatalogViolation::MoeWithoutSparsity { id: e.id.clone() });
            }
        }

        if v.is_empty() { Ok(()) } else { Err(v) }
    }

    /// Every entry that claims any coverage on this architecture.
    pub fn available_for(&self, arch_id: &str) -> Vec<&CatalogEntry> {
        self.entries
            .iter()
            .filter(|e| e.coverage.iter().any(|c| c.arch_id == arch_id))
            .collect()
    }

    /// Entries the planner may actually choose. An estimated size is not a
    /// fact, so an entry hanging off one is excluded rather than guessed at.
    pub fn plannable_for(&self, arch_id: &str) -> Vec<&CatalogEntry> {
        self.available_for(arch_id)
            .into_iter()
            .filter(|e| e.is_plannable())
            .collect()
    }

    /// Age in whole days, from `generated_at` (`YYYY-MM-DD[...]`).
    pub fn age_days(&self, now_unix: u64) -> Option<i64> {
        let (y, m, d) = parse_ymd(&self.generated_at)?;
        let hi = now_unix as i64 / 86_400;
        Some(hi - days_from_civil(y, m, d))
    }

    pub fn is_stale(&self, now_unix: u64, max_age_days: i64) -> bool {
        self.age_days(now_unix).map_or(true, |age| age > max_age_days)
    }
}

impl CatalogEntry {
    /// Bytes read per token. For a dense model this is the artifact; for MoE it
    /// is the active fraction scaled by the file's own bytes-per-param.
    pub fn bytes_per_token(&self) -> u64 {
        if self.model.shape == ShapeClass::Dense || self.model.total_params == 0 {
            return self.artifact.bytes;
        }
        let per_param = self.artifact.bytes as f64 / self.model.total_params as f64;
        (self.model.active_params as f64 * per_param).round() as u64
    }

    pub fn is_plannable(&self) -> bool {
        self.artifact.bytes_source != ByteSource::Estimated && self.artifact.bytes > 0
    }

    /// Whether two entries are the same artifact. Two publishers' "Q4_K_M" are
    /// different artifacts, so identity is the hash, never the quant label.
    pub fn same_artifact(&self, other: &Self) -> bool {
        match (&self.artifact.sha256, &other.artifact.sha256) {
            (Some(a), Some(b)) => a == b,
            _ => self.id == other.id,
        }
    }
}

fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let date = s.split(['T', ' ']).next()?;
    let mut it = date.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some((y, m, d))
}

/// Howard Hinnant's days_from_civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.26.json");

    fn seed() -> Catalog {
        Catalog::from_json(SEED).expect("seed catalogue must parse")
    }

    fn entry(id: &str) -> CatalogEntry {
        seed().entries.into_iter().find(|e| e.id == id).expect("entry")
    }

    #[test]
    fn the_shipped_catalogue_is_valid() {
        assert_eq!(seed().validate(), Ok(()));
    }

    #[test]
    fn the_seed_has_thirteen_entries() {
        // 12 GGUF plus the MLX artifact that already exists on the reference host.
        assert_eq!(seed().entries.len(), 13);
    }

    #[test]
    fn a_measured_artifact_without_a_hash_is_rejected() {
        let mut c = seed();
        c.entries[0].artifact.sha256 = None;
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-006"), "{err:?}");
    }

    #[test]
    fn an_estimated_artifact_without_a_hash_is_allowed() {
        let c = seed();
        let e = c.entries.iter().find(|e| e.id == "qwen2.5-7b-q4_k_m").unwrap();
        assert_eq!(e.artifact.bytes_source, ByteSource::Estimated);
        assert!(e.artifact.sha256.is_none());
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn an_entry_with_no_coverage_is_rejected() {
        let mut c = seed();
        c.entries[0].coverage.clear();
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-008"), "{err:?}");
    }

    #[test]
    fn an_entry_with_no_licence_is_rejected() {
        let mut c = seed();
        c.entries[0].artifact.licence = "  ".into();
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-007"), "{err:?}");
    }

    #[test]
    fn degenerate_kv_geometry_is_rejected() {
        let mut c = seed();
        c.entries[0].model.kv.head_dim = 0;
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-009"), "{err:?}");
    }

    #[test]
    fn active_params_above_total_is_rejected() {
        let mut c = seed();
        c.entries[0].model.active_params = c.entries[0].model.total_params + 1;
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-010"), "{err:?}");
    }

    #[test]
    fn a_moe_with_no_sparsity_is_rejected() {
        let mut c = seed();
        let i = c.entries.iter().position(|e| e.id == "olmoe-1b-7b-q4_k_m").unwrap();
        c.entries[i].model.active_params = c.entries[i].model.total_params;
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-011"), "{err:?}");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut c = seed();
        let clone = c.entries[0].clone();
        c.entries.push(clone);
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-003"), "{err:?}");
    }

    #[test]
    fn coverage_filters_by_architecture() {
        let c = seed();
        assert_eq!(c.available_for("apple-m5-pro").len(), 13);
        assert_eq!(c.available_for("gfx1151").len(), 0, "no AMD coverage yet");
        assert_eq!(c.available_for("sm_121").len(), 0, "no Blackwell coverage yet");
    }

    #[test]
    fn estimated_artifacts_are_not_plannable() {
        let c = seed();
        let plannable = c.plannable_for("apple-m5-pro");
        // The four measured GGUF artifacts plus the MLX 27B (measured size,
        // manifest hash). The eight estimated ones are excluded.
        assert_eq!(plannable.len(), 5, "only artifacts whose size we know");
        assert!(plannable.iter().all(|e| e.artifact.bytes_source == ByteSource::Measured));
    }

    #[test]
    fn two_publishers_q4_k_m_are_distinct_entries() {
        let c = seed();
        let same_quant: Vec<_> = c
            .entries
            .iter()
            .filter(|e| e.artifact.quant == "Q4_K_M")
            .collect();
        assert!(same_quant.len() > 1);
        // Identity is the hash, not the label.
        let a = entry("qwen2.5-3b-q4_k_m");
        let b = entry("olmoe-1b-7b-q4_k_m");
        assert!(!a.same_artifact(&b));
        assert!(a.same_artifact(&a.clone()));
    }

    #[test]
    fn a_dense_entry_reads_the_whole_artifact_per_token() {
        let e = entry("qwen2.5-3b-q4_k_m");
        assert_eq!(e.bytes_per_token(), e.artifact.bytes);
    }

    #[test]
    fn a_moe_entry_reads_only_its_active_fraction() {
        let e = entry("olmoe-1b-7b-q4_k_m");
        let bpt = e.bytes_per_token();
        assert!(bpt < e.artifact.bytes / 3, "MoE must read far less than the file");
        // 1.3B active at 4.87 bit/param => ~791 MB, matching the spike measurement.
        assert!((760_000_000..=820_000_000).contains(&bpt), "got {bpt}");
    }

    #[test]
    fn staleness_is_computed_from_the_generated_date() {
        let c = seed();
        let gen_days = days_from_civil(2026, 10, 26);
        let now = (gen_days as u64 + 30) * 86_400;
        assert_eq!(c.age_days(now), Some(30));
        assert!(!c.is_stale(now, 90));
        assert!(c.is_stale(now, 7));
    }

    #[test]
    fn an_unparseable_date_is_treated_as_stale() {
        let mut c = seed();
        c.generated_at = "not-a-date".into();
        assert!(c.is_stale(0, 365));
    }

    #[test]
    fn a_foreign_schema_major_is_rejected() {
        let mut c = seed();
        c.schema = "qinfer/catalog@2".into();
        let err = c.validate().unwrap_err();
        assert!(err.iter().any(|v| v.code() == "E-CAT-001"), "{err:?}");
    }
}
