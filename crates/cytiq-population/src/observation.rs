//! Quantities retain their measurement status, denominator and source.

use crate::{Result, SCHEMA_VERSION, require, serialization};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Population,
    Phenotype,
    Residual,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub kind: TargetKind,
    pub id: String,
    pub scope: String,
    #[serde(default)]
    pub state: Option<bool>,
}

impl Target {
    /// # Errors
    /// Rejects ambiguous target identities and missing scope.
    pub fn validate(&self) -> Result<()> {
        require(
            !self.id.trim().is_empty() && !self.scope.trim().is_empty(),
            "target and scope must be explicit",
        )?;
        require(
            (self.kind == TargetKind::Phenotype) == self.state.is_some(),
            "only phenotype targets require a Boolean state",
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    ObservedCount,
    EstimatedCount,
    Fraction,
    Concentration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    Measured,
    Estimated,
    NotMeasured,
    QcRejected,
    BelowDetection,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Denominator {
    /// None means all assessed events of the named analysis scope.
    pub target: Option<Target>,
    pub scope: String,
    pub observed_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    pub name: String,
    pub version: String,
    pub definition_sha256: String,
    pub experimental: bool,
    pub source_observation_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lineage {
    pub acquisition_sha256: Option<String>,
    pub tree_sha256: String,
    pub panel_profile_sha256: String,
    pub projection_sha256: String,
    pub annotations_sha256: Option<String>,
    pub producer_provenance_sha256: Option<String>,
    pub analysis_mask_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationObservation {
    pub schema_version: u32,
    pub observation_id: String,
    pub specimen_id: String,
    pub panel_id: String,
    pub acquisition_id: Option<String>,
    pub source_id: String,
    pub target: Target,
    pub measure: Measure,
    pub unit: String,
    pub status: ObservationStatus,
    pub value_u64: Option<u64>,
    pub value_f64: Option<f64>,
    pub denominator: Option<Denominator>,
    pub method: Method,
    pub lineage: Lineage,
    pub n_events: u64,
    /// Rejected/below-detection counts are evidence, never a usable value.
    pub supporting_count: Option<u64>,
    pub reason: Option<String>,
}

impl PopulationObservation {
    /// Refresh the identity after intentionally deriving a new observation.
    /// # Errors
    /// Rejects values that cannot be serialized.
    pub fn seal(&mut self) -> Result<()> {
        self.observation_id.clear();
        self.observation_id = serialization::digest(&serde_json::to_value(&*self)?);
        self.validate()
    }

    /// # Errors
    /// Rejects invalid quantities, unavailable numeric values and changed identities.
    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == SCHEMA_VERSION,
            "unsupported observation schema",
        )?;
        self.target.validate()?;
        require(
            !self.specimen_id.trim().is_empty()
                && !self.panel_id.trim().is_empty()
                && !self.source_id.trim().is_empty(),
            "missing observation identity",
        )?;
        require(
            is_sha256(&self.method.definition_sha256)
                && !self.method.name.is_empty()
                && !self.method.version.is_empty(),
            "invalid observation method",
        )?;
        self.validate_lineage()?;
        require(
            self.value_f64.is_none_or(|v| v.is_finite() && v >= 0.0),
            "invalid numeric observation",
        )?;
        let numeric = matches!(
            self.status,
            ObservationStatus::Measured | ObservationStatus::Estimated
        );
        require(
            numeric == (self.value_u64.is_some() ^ self.value_f64.is_some()),
            "status/value mismatch",
        )?;
        if !numeric {
            require(
                self.value_u64.is_none() && self.value_f64.is_none() && self.reason.is_some(),
                "unavailable observations require a reason and null values",
            )?;
        }
        let expected_unit = match self.measure {
            Measure::ObservedCount | Measure::EstimatedCount => "events",
            Measure::Fraction => "fraction",
            Measure::Concentration => "cells_per_ul",
        };
        require(self.unit == expected_unit, "measure/unit mismatch")?;
        if numeric {
            match self.measure {
                Measure::ObservedCount => require(
                    self.value_u64.is_some_and(|n| n <= self.n_events)
                        && self
                            .denominator
                            .as_ref()
                            .is_none_or(|d| self.value_u64.is_some_and(|n| n <= d.observed_count))
                        && self.status == ObservationStatus::Measured,
                    "observed counts require exact measured integers",
                )?,
                Measure::EstimatedCount => require(
                    self.value_f64.is_some() && self.status == ObservationStatus::Estimated,
                    "estimated counts require estimated floats",
                )?,
                Measure::Fraction => {
                    require(
                        self.value_f64.is_some_and(|v| v <= 1.0)
                            && self
                                .denominator
                                .as_ref()
                                .is_some_and(|d| d.observed_count > 0),
                        "invalid fraction denominator/value",
                    )?;
                }
                Measure::Concentration => require(
                    self.value_f64.is_some(),
                    "concentration requires a floating value",
                )?,
            }
        }
        if let Some(denominator) = &self.denominator {
            require(!denominator.scope.is_empty(), "missing denominator scope")?;
            if let Some(target) = &denominator.target {
                target.validate()?;
            }
        }
        let mut unsigned = self.clone();
        unsigned.observation_id.clear();
        require(
            self.observation_id == serialization::digest(&serde_json::to_value(unsigned)?),
            "observation identity mismatch",
        )
    }
    fn validate_lineage(&self) -> Result<()> {
        for sha in [
            &self.lineage.tree_sha256,
            &self.lineage.panel_profile_sha256,
            &self.lineage.projection_sha256,
        ] {
            require(is_sha256(sha), "invalid knowledge lineage digest")?;
        }
        for sha in [
            &self.lineage.acquisition_sha256,
            &self.lineage.annotations_sha256,
            &self.lineage.producer_provenance_sha256,
            &self.lineage.analysis_mask_sha256,
        ]
        .into_iter()
        .flatten()
        {
            require(is_sha256(sha), "invalid source lineage digest")?;
        }
        require(
            self.method
                .source_observation_ids
                .iter()
                .all(|id| is_sha256(id)),
            "invalid source observation reference",
        )?;
        require(
            self.supporting_count.is_none_or(|n| n <= self.n_events),
            "invalid supporting count",
        )?;
        Ok(())
    }
}

#[must_use]
pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
