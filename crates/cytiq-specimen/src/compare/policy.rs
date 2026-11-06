use super::ComparisonKey;
use crate::{Result, require};
use cytiq_population::{Measure, serialization};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Tolerance {
    Count(u64),
    Float(f64),
}

impl Tolerance {
    fn validate(&self, measure: Measure) -> Result<()> {
        require(
            match self {
                Self::Count(_) => measure == Measure::ObservedCount,
                Self::Float(value) => {
                    measure != Measure::ObservedCount && value.is_finite() && *value >= 0.0
                }
            },
            "tolerance must match the measure and be finite/nonnegative",
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tolerances {
    pub observed_count: u64,
    pub estimated_count: f64,
    pub fraction: f64,
    pub concentration: f64,
}

impl Tolerances {
    fn get(&self, measure: Measure) -> Tolerance {
        match measure {
            Measure::ObservedCount => Tolerance::Count(self.observed_count),
            Measure::EstimatedCount => Tolerance::Float(self.estimated_count),
            Measure::Fraction => Tolerance::Float(self.fraction),
            Measure::Concentration => Tolerance::Float(self.concentration),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToleranceOverride {
    pub key: ComparisonKey,
    pub limit: Tolerance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPolicy {
    pub schema_version: u32,
    #[serde(default)]
    pub defaults: Tolerances,
    #[serde(default)]
    pub overrides: Vec<ToleranceOverride>,
}

impl Default for ComparisonPolicy {
    fn default() -> Self {
        Self {
            schema_version: 1,
            defaults: Tolerances::default(),
            overrides: Vec::new(),
        }
    }
}

impl ComparisonPolicy {
    /// Read strict versioned JSON/YAML policy. Keys are checked against both inputs by compare.
    /// # Errors
    /// Rejects malformed policy documents and invalid tolerance types or values.
    pub fn load(path: &Path) -> Result<Self> {
        let policy: Self = serde_json::from_value(serialization::read(path)?)?;
        policy.validate()?;
        Ok(policy)
    }

    pub(super) fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported comparison policy version",
        )?;
        for measure in [
            Measure::ObservedCount,
            Measure::EstimatedCount,
            Measure::Fraction,
            Measure::Concentration,
        ] {
            self.defaults.get(measure).validate(measure)?;
        }
        let mut keys = BTreeSet::new();
        for rule in &self.overrides {
            rule.key.target.validate()?;
            require(
                !rule.key.panel_id.trim().is_empty() && !rule.key.source_id.trim().is_empty(),
                "empty comparison override identity",
            )?;
            require(
                keys.insert(rule.key.token()?),
                "duplicate comparison override",
            )?;
            rule.limit.validate(rule.key.measure)?;
        }
        Ok(())
    }

    pub(super) fn limits(&self, keys: &BTreeSet<String>) -> Result<BTreeMap<String, &Tolerance>> {
        self.validate()?;
        self.overrides
            .iter()
            .map(|rule| {
                let key = rule.key.token()?;
                require(
                    keys.contains(&key),
                    "comparison override does not match an input row",
                )?;
                Ok((key, &rule.limit))
            })
            .collect()
    }

    pub(super) fn limit(&self, measure: Measure) -> Tolerance {
        self.defaults.get(measure)
    }
}
