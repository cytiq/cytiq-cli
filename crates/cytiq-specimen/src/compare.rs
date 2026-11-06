//! Semantic comparisons of validated native summaries, retaining standalone input snapshots.

mod policy;
pub use policy::{ComparisonPolicy, Tolerance, ToleranceOverride, Tolerances};

use crate::{Result, SpecimenResult, reconcile::SpecimenCheck, require};
use cytiq_population::{Measure, PopulationObservation, Target, serialization};
use cytiq_provenance::{BuildInfo, FileRecord, outcome::OutcomeReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Observations,
    Results,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonKey {
    pub collection: Collection,
    pub panel_id: String,
    pub source_id: String,
    pub target: Target,
    pub measure: Measure,
}

impl ComparisonKey {
    pub(crate) fn token(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowVerdict {
    Equal,
    WithinTolerance,
    Unavailable,
    Added,
    Removed,
    Incompatible,
    StatusChanged,
    ExceedsTolerance,
}

impl RowVerdict {
    #[must_use]
    pub const fn failed(self) -> bool {
        matches!(
            self,
            Self::Added
                | Self::Removed
                | Self::Incompatible
                | Self::StatusChanged
                | Self::ExceedsTolerance
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delta {
    Count { absolute: u64, direction: Direction },
    Float { absolute: f64, signed: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Decreased,
    Equal,
    Increased,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonRow {
    pub key: ComparisonKey,
    /// Observation IDs resolve within the collection in the corresponding snapshot.
    pub baseline_id: Option<String>,
    pub candidate_id: Option<String>,
    pub verdict: RowVerdict,
    pub reasons: Vec<String>,
    pub tolerance: Tolerance,
    pub delta: Option<Delta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextChange {
    pub key: Option<ComparisonKey>,
    pub field: String,
    pub baseline: Value,
    pub candidate: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckComparison {
    pub category: String,
    pub scope: String,
    pub check: String,
    pub baseline: Vec<SpecimenCheck>,
    pub candidate: Vec<SpecimenCheck>,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonSummary {
    pub rows: usize,
    pub numeric_pairs: usize,
    pub failed_rows: usize,
    pub failed_checks: usize,
    pub specimen_changed: bool,
    pub context_changes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageIdentity {
    pub artifacts: Vec<FileRecord>,
    pub version: String,
    pub build: BuildInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenComparison {
    pub schema_version: u32,
    pub baseline: SpecimenResult,
    pub candidate: SpecimenResult,
    pub baseline_package: Option<PackageIdentity>,
    pub candidate_package: Option<PackageIdentity>,
    pub policy: ComparisonPolicy,
    pub rows: Vec<ComparisonRow>,
    pub checks: Vec<CheckComparison>,
    pub context: Vec<ContextChange>,
    pub summary: ComparisonSummary,
}

impl SpecimenComparison {
    #[must_use]
    pub fn outcome_reason(&self) -> OutcomeReason {
        if self.summary.failed_rows > 0
            || self.summary.failed_checks > 0
            || self.summary.specimen_changed
        {
            OutcomeReason::ChecksFailed
        } else if self.summary.numeric_pairs == 0 {
            OutcomeReason::Incomplete
        } else {
            OutcomeReason::Success
        }
    }

    /// Recompute the verdict from embedded validated snapshots, without the original packages.
    /// # Errors
    /// Rejects unsupported versions, invalid snapshots, changed calculations or package identities.
    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported specimen comparison version",
        )?;
        let mut expected = compare(&self.baseline, &self.candidate, &self.policy)?;
        for identity in [&self.baseline_package, &self.candidate_package]
            .into_iter()
            .flatten()
        {
            let mut paths = BTreeSet::new();
            require(
                !identity.version.is_empty() && !identity.artifacts.is_empty(),
                "invalid comparison package identity",
            )?;
            for artifact in &identity.artifacts {
                require(
                    cytiq_population::observation::is_sha256(&artifact.sha256)
                        && paths.insert(&artifact.path),
                    "invalid or duplicate comparison input digest",
                )?;
            }
        }
        expected.baseline_package.clone_from(&self.baseline_package);
        expected
            .candidate_package
            .clone_from(&self.candidate_package);
        require(
            expected == *self,
            "comparison snapshots, calculations or summary disagree",
        )
    }

    /// Load a completed comparison, including comparisons whose checks failed.
    /// # Errors
    /// Rejects corrupt packages, unsupported schemas and inconsistent results/outcomes.
    pub fn load(directory: &Path) -> Result<Self> {
        let outcome = cytiq_population::verify_package(directory)?;
        require(
            outcome.command == "cytiq specimen compare",
            "not a specimen comparison package",
        )?;
        require(
            outcome
                .artifacts
                .iter()
                .any(|a| a.path == "comparison.json"),
            "comparison data is not recorded",
        )?;
        let comparison: Self =
            serde_json::from_value(serialization::read(&directory.join("comparison.json"))?)?;
        require(
            comparison.baseline_package.is_some() && comparison.candidate_package.is_some(),
            "comparison package lacks input identities",
        )?;
        comparison.validate()?;
        require(
            outcome.reason == comparison.outcome_reason(),
            "comparison and outcome disagree",
        )?;
        Ok(comparison)
    }
}

pub(crate) type RowIndex<'a> = BTreeMap<String, (ComparisonKey, &'a PopulationObservation)>;

pub(crate) fn index_rows(result: &SpecimenResult) -> Result<RowIndex<'_>> {
    let mut index = BTreeMap::new();
    for (collection, rows) in [
        (Collection::Observations, &result.observations),
        (Collection::Results, &result.results),
    ] {
        for row in rows {
            let key = ComparisonKey {
                collection,
                panel_id: row.panel_id.clone(),
                source_id: row.source_id.clone(),
                target: row.target.clone(),
                measure: row.measure,
            };
            require(
                index.insert(key.token()?, (key, row)).is_none(),
                "ambiguous duplicate semantic observation key",
            )?;
        }
    }
    Ok(index)
}

pub(crate) fn normalize(result: &SpecimenResult) -> Result<SpecimenResult> {
    result.validate()?;
    let index = index_rows(result)?;
    let mut result = result.clone();
    result.observations.clear();
    result.results.clear();
    for (_, (key, row)) in index {
        match key.collection {
            Collection::Observations => result.observations.push(row.clone()),
            Collection::Results => result.results.push(row.clone()),
        }
    }
    result.checks.sort_by_cached_key(|c| {
        (
            c.category.clone(),
            c.scope.clone(),
            c.check.clone(),
            c.status,
            c.message.clone(),
        )
    });
    Ok(result)
}

/// Compare one specimen's original observations and reconciled results independently.
/// # Errors
/// Rejects invalid snapshots, differing specimen IDs, ambiguous rows or invalid policy rules.
pub fn compare(
    baseline: &SpecimenResult,
    candidate: &SpecimenResult,
    policy: &ComparisonPolicy,
) -> Result<SpecimenComparison> {
    let baseline = normalize(baseline)?;
    let candidate = normalize(candidate)?;
    require(
        baseline.specimen_id == candidate.specimen_id,
        "comparison requires the same specimen_id",
    )?;
    let left = index_rows(&baseline)?;
    let right = index_rows(&candidate)?;
    let keys: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    let limits = policy.limits(&keys)?;
    let mut context = Vec::new();
    record_change(
        &mut context,
        None,
        "protocol_id",
        &baseline.protocol_id,
        &candidate.protocol_id,
    )?;
    let mut rows = Vec::new();
    for token in keys {
        let a = left.get(&token);
        let b = right.get(&token);
        let key = a
            .or(b)
            .ok_or_else(|| crate::SpecimenError::Invalid("missing comparison key".into()))?
            .0
            .clone();
        let tolerance = limits
            .get(&token)
            .map_or_else(|| policy.limit(key.measure), |v| (*v).clone());
        rows.push(compare_row(
            key,
            a.map(|v| v.1),
            b.map(|v| v.1),
            tolerance,
            &mut context,
        )?);
    }
    let checks = compare_checks(&baseline.checks, &candidate.checks);
    let summary = ComparisonSummary {
        rows: rows.len(),
        numeric_pairs: rows.iter().filter(|r| r.delta.is_some()).count(),
        failed_rows: rows.iter().filter(|r| r.verdict.failed()).count(),
        failed_checks: checks.iter().filter(|c| !c.passed).count(),
        specimen_changed: baseline.complete != candidate.complete
            || baseline.status != candidate.status,
        context_changes: context.len(),
    };
    Ok(SpecimenComparison {
        schema_version: 1,
        baseline,
        candidate,
        baseline_package: None,
        candidate_package: None,
        policy: policy.clone(),
        rows,
        checks,
        context,
        summary,
    })
}

fn compare_row(
    key: ComparisonKey,
    a: Option<&PopulationObservation>,
    b: Option<&PopulationObservation>,
    tolerance: Tolerance,
    context: &mut Vec<ContextChange>,
) -> Result<ComparisonRow> {
    let mut row = ComparisonRow {
        key,
        baseline_id: a.map(|o| o.observation_id.clone()),
        candidate_id: b.map(|o| o.observation_id.clone()),
        verdict: RowVerdict::Equal,
        reasons: Vec::new(),
        tolerance,
        delta: None,
    };
    let (Some(a), Some(b)) = (a, b) else {
        row.verdict = if a.is_some() {
            RowVerdict::Removed
        } else {
            RowVerdict::Added
        };
        return Ok(row);
    };
    record_row_context(context, &row.key, a, b)?;
    if a.unit != b.unit {
        row.reasons.push("unit_changed".into());
    }
    if a.lineage.tree_sha256 != b.lineage.tree_sha256 {
        row.reasons.push("tree_changed".into());
    }
    let denominator = |o: &PopulationObservation| {
        o.denominator
            .as_ref()
            .map(|d| (d.target.clone(), d.scope.clone()))
    };
    if denominator(a) != denominator(b) {
        row.reasons.push("denominator_definition_changed".into());
    }
    if a.status != b.status {
        row.reasons.push("status_changed".into());
    }
    if !row.reasons.is_empty() {
        row.verdict = if row.reasons.iter().any(|r| r != "status_changed") {
            RowVerdict::Incompatible
        } else {
            RowVerdict::StatusChanged
        };
        return Ok(row);
    }
    let (equal, within) = match (
        &row.tolerance,
        a.value_u64,
        b.value_u64,
        a.value_f64,
        b.value_f64,
    ) {
        (Tolerance::Count(limit), Some(x), Some(y), _, _) => {
            let absolute = x.abs_diff(y);
            let direction = match y.cmp(&x) {
                std::cmp::Ordering::Less => Direction::Decreased,
                std::cmp::Ordering::Equal => Direction::Equal,
                std::cmp::Ordering::Greater => Direction::Increased,
            };
            row.delta = Some(Delta::Count {
                absolute,
                direction,
            });
            (absolute == 0, absolute <= *limit)
        }
        (Tolerance::Float(limit), _, _, Some(x), Some(y)) => {
            let signed = y - x;
            let absolute = signed.abs();
            row.delta = Some(Delta::Float { absolute, signed });
            (absolute == 0.0, absolute <= *limit)
        }
        _ => {
            row.verdict = RowVerdict::Unavailable;
            return Ok(row);
        }
    };
    row.verdict = if equal {
        RowVerdict::Equal
    } else if within {
        RowVerdict::WithinTolerance
    } else {
        RowVerdict::ExceedsTolerance
    };
    Ok(row)
}

fn record_change(
    context: &mut Vec<ContextChange>,
    key: Option<&ComparisonKey>,
    field: &str,
    a: &impl Serialize,
    b: &impl Serialize,
) -> Result<()> {
    let baseline = serde_json::to_value(a)?;
    let candidate = serde_json::to_value(b)?;
    if baseline != candidate {
        context.push(ContextChange {
            key: key.cloned(),
            field: field.into(),
            baseline,
            candidate,
        });
    }
    Ok(())
}

fn record_row_context(
    context: &mut Vec<ContextChange>,
    key: &ComparisonKey,
    a: &PopulationObservation,
    b: &PopulationObservation,
) -> Result<()> {
    let a = serde_json::to_value(a)?;
    let b = serde_json::to_value(b)?;
    for field in ["acquisition_id", "n_events", "supporting_count", "reason"] {
        record_change(context, Some(key), field, &a[field], &b[field])?;
    }
    for (section, fields) in [
        (
            "method",
            &[
                "name",
                "version",
                "definition_sha256",
                "experimental",
                "source_observation_ids",
            ][..],
        ),
        (
            "lineage",
            &[
                "acquisition_sha256",
                "tree_sha256",
                "panel_profile_sha256",
                "projection_sha256",
                "annotations_sha256",
                "producer_provenance_sha256",
                "analysis_mask_sha256",
            ][..],
        ),
        ("denominator", &["observed_count"][..]),
    ] {
        for field in fields {
            record_change(
                context,
                Some(key),
                &format!("{section}.{field}"),
                &a[section][*field],
                &b[section][*field],
            )?;
        }
    }
    Ok(())
}

fn compare_checks(a: &[SpecimenCheck], b: &[SpecimenCheck]) -> Vec<CheckComparison> {
    type Key = (String, String, String);
    let mut groups: BTreeMap<Key, (Vec<SpecimenCheck>, Vec<SpecimenCheck>)> = BTreeMap::new();
    for (side, checks) in [(false, a), (true, b)] {
        for check in checks {
            let pair = groups
                .entry((
                    check.category.clone(),
                    check.scope.clone(),
                    check.check.clone(),
                ))
                .or_default();
            if side {
                pair.1.push(check.clone());
            } else {
                pair.0.push(check.clone());
            }
        }
    }
    groups
        .into_iter()
        .map(|((category, scope, check), (baseline, candidate))| {
            let statuses = |checks: &[SpecimenCheck]| {
                let mut statuses: Vec<_> = checks.iter().map(|c| c.status).collect();
                statuses.sort_unstable();
                statuses
            };
            let passed = statuses(&baseline) == statuses(&candidate);
            CheckComparison {
                category,
                scope,
                check,
                baseline,
                candidate,
                passed,
            }
        })
        .collect()
}
