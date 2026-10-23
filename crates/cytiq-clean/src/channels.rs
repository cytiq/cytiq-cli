//! Mapping of FCS parameters to the roles the cleaning filters need.

use std::path::Path;

use cytiq_fcs::{FcsDataset, Parameter};
use serde::Serialize;

use crate::error::CleanError;

/// Substrings (of the lower-cased `$PnN`/`$PnS`) that mark a viability dye.
const VIABILITY_PATTERNS: &[&str] = &[
    "live",
    "dead",
    "l/d",
    "zombie",
    "viab",
    "fixable",
    "dapi",
    "7-aad",
    "7aad",
    "propidium",
    "sytox",
];

/// Whole tokens (split on non-alphanumerics) that mark a viability dye.
/// Kept separate because e.g. `pi` would match far too much as a substring.
const VIABILITY_TOKENS: &[&str] = &["pi", "ld", "lvd"];

/// User-supplied channel names; `None` means "use the default / detect".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelOverrides {
    pub fsc_a: String,
    pub fsc_h: String,
    pub fsc_w: String,
    pub ssc_a: String,
    /// Explicit viability channel; `None` auto-detects.
    pub viability: Option<String>,
    /// Skip the viability filter entirely.
    pub no_viability: bool,
}

/// Resolved channel names, as written into the `GatingML` dimensions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChannelRoles {
    pub fsc_a: String,
    pub fsc_h: Option<String>,
    pub fsc_w: Option<String>,
    pub ssc_a: Option<String>,
    pub viability: Option<String>,
    /// How the viability channel was chosen: `explicit`, `detected`,
    /// `disabled` or `none`.
    pub viability_source: &'static str,
}

impl ChannelRoles {
    /// Resolve roles against a dataset.
    ///
    /// # Errors
    ///
    /// Fails when FSC-A or an explicitly requested viability channel is
    /// missing, or when several viability candidates are found.
    pub fn resolve(
        dataset: &FcsDataset,
        overrides: &ChannelOverrides,
        path: &Path,
    ) -> Result<Self, CleanError> {
        let optional = |name: &str| {
            dataset
                .find_parameter(name)
                .map(|index| dataset.parameters[index].name.clone())
        };
        let fsc_a = optional(&overrides.fsc_a).ok_or_else(|| CleanError::MissingChannel {
            name: overrides.fsc_a.clone(),
            path: path.to_path_buf(),
        })?;

        let (viability, viability_source) = if overrides.no_viability {
            (None, "disabled")
        } else if let Some(name) = &overrides.viability {
            let channel = optional(name).ok_or_else(|| CleanError::MissingChannel {
                name: name.clone(),
                path: path.to_path_buf(),
            })?;
            (Some(channel), "explicit")
        } else {
            let candidates = dataset
                .parameters
                .iter()
                .filter(|parameter| is_viability_parameter(parameter))
                .map(|parameter| parameter.name.clone())
                .collect::<Vec<_>>();
            match candidates.as_slice() {
                [] => (None, "none"),
                [single] => (Some(single.clone()), "detected"),
                _ => {
                    return Err(CleanError::AmbiguousViabilityChannel {
                        candidates: candidates.join(", "),
                    });
                }
            }
        };

        Ok(Self {
            fsc_a,
            fsc_h: optional(&overrides.fsc_h),
            fsc_w: optional(&overrides.fsc_w),
            ssc_a: optional(&overrides.ssc_a),
            viability,
            viability_source,
        })
    }
}

/// Whether a parameter looks like a viability dye by its `$PnN` or `$PnS`.
#[must_use]
pub fn is_viability_parameter(parameter: &Parameter) -> bool {
    if is_acquisition_channel(&parameter.name) {
        return false;
    }
    std::iter::once(parameter.name.as_str())
        .chain(parameter.label.as_deref())
        .any(is_viability_name)
}

fn is_viability_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    VIABILITY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
        || lower
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| VIABILITY_TOKENS.contains(&token))
}

fn is_acquisition_channel(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("fsc") || lower.starts_with("ssc") || lower == "time"
}

#[cfg(test)]
mod tests {
    use cytiq_fcs::{ParameterDataType, ParameterWidth};

    use super::*;

    fn parameter(name: &str, label: Option<&str>) -> Parameter {
        Parameter {
            index: 1,
            name: name.to_owned(),
            label: label.map(str::to_owned),
            width: ParameterWidth::Bits(32),
            data_type: ParameterDataType::Float,
            keywords: Vec::new(),
        }
    }

    #[test]
    fn detects_common_viability_dyes() {
        for (name, label) in [
            ("BV510-A", Some("Live/Dead")),
            ("APC-Cy7-A", Some("Zombie NIR")),
            ("FL3-A", Some("7-AAD")),
            ("PE-Texas Red-A", Some("PI")),
            ("V450-A", Some("Fixable Viability Dye eF450")),
            ("DAPI-A", None),
            ("BUV496-A", Some("L/D")),
        ] {
            assert!(
                is_viability_parameter(&parameter(name, label)),
                "{name} {label:?}"
            );
        }
    }

    #[test]
    fn ignores_markers_and_scatter() {
        for (name, label) in [
            ("FITC-A", Some("CD3")),
            ("PE-A", Some("PIGR")),
            ("APC-A", Some("CD161")),
            ("FSC-A", None),
            ("Time", None),
        ] {
            assert!(
                !is_viability_parameter(&parameter(name, label)),
                "{name} {label:?}"
            );
        }
    }
}
