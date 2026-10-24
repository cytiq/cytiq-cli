use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cytiq_fcs::{FcsDataset, FcsReader, Parameter, Spillover};
use cytiq_manifest::{ControlEntry, Marker, MatrixKind, Panel};
use serde::Serialize;

/// Warn when the 1-norm condition number of the spillover matrix exceeds
/// this: compensation then amplifies raw-value noise by up to that factor.
pub const SPILLOVER_CONDITION_WARN: f64 = 100.0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// One preflight finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What was checked: `manifest`, `sample S01`, `control FITC`, `run`.
    pub scope: String,
    pub check: &'static str,
    pub status: CheckStatus,
    pub message: String,
}

impl Check {
    fn new(scope: &str, check: &'static str, status: CheckStatus, message: String) -> Self {
        Self {
            scope: scope.to_owned(),
            check,
            status,
            message,
        }
    }
}

/// A manifest file together with its parsed dataset, when it could be read.
pub(crate) struct LoadedFile {
    pub scope: String,
    pub dataset: Option<FcsDataset>,
}

/// Readability, event count, signal state and acquisition metadata of one
/// file.
pub(crate) fn check_file(scope: &str, path: &Path) -> (Vec<Check>, LoadedFile) {
    let mut checks = Vec::new();
    let dataset = match FcsReader::open(path).and_then(FcsReader::read) {
        Ok(dataset) => dataset,
        Err(error) => {
            checks.push(Check::new(
                scope,
                "fcs_readable",
                CheckStatus::Fail,
                format!("`{}` cannot be read: {error}", path.display()),
            ));
            return (
                checks,
                LoadedFile {
                    scope: scope.to_owned(),
                    dataset: None,
                },
            );
        }
    };
    checks.push(Check::new(
        scope,
        "fcs_readable",
        CheckStatus::Pass,
        format!(
            "`{}`: {} event(s), {} parameter(s)",
            path.display(),
            dataset.event_count(),
            dataset.parameters.len()
        ),
    ));
    if dataset.event_count() == 0 {
        checks.push(Check::new(
            scope,
            "events",
            CheckStatus::Fail,
            "the file has no events".to_owned(),
        ));
    }
    checks.push(signal_state(scope, &dataset));
    let missing = ["$CYT", "$DATE"]
        .into_iter()
        .filter(|key| {
            dataset
                .text
                .get(key)
                .is_none_or(|value| value.trim().is_empty())
        })
        .collect::<Vec<_>>();
    checks.push(if missing.is_empty() {
        Check::new(
            scope,
            "acquisition_metadata",
            CheckStatus::Pass,
            "`$CYT` and `$DATE` are present".to_owned(),
        )
    } else {
        Check::new(
            scope,
            "acquisition_metadata",
            CheckStatus::Warn,
            format!("missing {}", missing.join(", ")),
        )
    });
    (
        checks,
        LoadedFile {
            scope: scope.to_owned(),
            dataset: Some(dataset),
        },
    )
}

fn signal_state(scope: &str, dataset: &FcsDataset) -> Check {
    let text = &dataset.text;
    let processed = [
        (
            cytiq_fcs::transform_state(text).map(|state| state.is_some()),
            "cytiq transform",
        ),
        (
            cytiq_fcs::compensation_state(text).map(|state| state.is_some()),
            "cytiq compensate",
        ),
        (
            cytiq_fcs::unmixing_state(text).map(|state| state.is_some()),
            "cytiq autospectral",
        ),
    ];
    let by = processed
        .iter()
        .filter(|(state, _)| !matches!(state, Ok(false)))
        .map(|(_, tool)| *tool)
        .collect::<Vec<_>>();
    if by.is_empty() {
        Check::new(
            scope,
            "raw_signal",
            CheckStatus::Pass,
            "raw values (no cytiq transform, compensation or unmixing)".to_owned(),
        )
    } else {
        Check::new(
            scope,
            "raw_signal",
            CheckStatus::Fail,
            format!(
                "the file was already processed by {}; preflight expects raw acquisition files",
                by.join(", ")
            ),
        )
    }
}

/// Scope of panel-level checks: `panel T`, or `panel` for an unnamed one.
pub(crate) fn panel_scope(panel: &Panel) -> String {
    panel
        .name
        .as_ref()
        .map_or_else(|| "panel".to_owned(), |name| format!("panel {name}"))
}

/// Parameters of `dataset` that carry `marker`: its `channel`, or those whose
/// `$PnS`/`$PnN` match its name.
fn marker_candidates(dataset: &FcsDataset, marker: &Marker) -> Vec<usize> {
    match &marker.channel {
        Some(channel) => dataset.find_parameter(channel).into_iter().collect(),
        None => dataset
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, parameter)| matches_marker(parameter, marker.marker.trim()))
            .map(|(index, _)| index)
            .collect(),
    }
}

/// Every panel marker maps to exactly one channel of the sample.
pub(crate) fn check_markers(scope: &str, dataset: &FcsDataset, panel: &Panel) -> Vec<Check> {
    let mut checks = Vec::new();
    for marker in &panel.markers {
        let name = marker.marker.trim();
        let candidates = marker_candidates(dataset, marker);
        let check = match candidates.as_slice() {
            [] => Check::new(
                scope,
                "panel_marker",
                if marker.required {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Warn
                },
                match &marker.channel {
                    Some(channel) => format!("marker `{name}`: channel `{channel}` not found"),
                    None => format!("marker `{name}`: no channel has `$PnS` or `$PnN` `{name}`"),
                },
            ),
            [index] => {
                let parameter = &dataset.parameters[*index];
                let label_disagrees = marker.channel.is_some()
                    && parameter
                        .label
                        .as_deref()
                        .is_some_and(|label| !matches_marker(parameter, name) && !label.is_empty());
                if label_disagrees {
                    Check::new(
                        scope,
                        "panel_marker",
                        CheckStatus::Warn,
                        format!(
                            "marker `{name}`: channel `{}` is labelled `{}`",
                            parameter.name,
                            parameter.label.as_deref().unwrap_or_default()
                        ),
                    )
                } else {
                    Check::new(
                        scope,
                        "panel_marker",
                        CheckStatus::Pass,
                        format!("marker `{name}` → `{}`", parameter.name),
                    )
                }
            }
            several => Check::new(
                scope,
                "panel_marker",
                CheckStatus::Fail,
                format!(
                    "marker `{name}` matches several channels: {}; set `channel` in the manifest",
                    several
                        .iter()
                        .map(|index| dataset.parameters[*index].name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        };
        checks.push(check);
    }

    let mut labels = BTreeMap::<String, Vec<&str>>::new();
    for parameter in &dataset.parameters {
        if let Some(label) = parameter
            .label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
        {
            labels
                .entry(label.trim().to_ascii_lowercase())
                .or_default()
                .push(&parameter.name);
        }
    }
    for (label, channels) in labels
        .into_iter()
        .filter(|(_, channels)| channels.len() > 1)
    {
        checks.push(Check::new(
            scope,
            "unique_labels",
            CheckStatus::Warn,
            format!(
                "label `{label}` is used by several channels: {}",
                channels.join(", ")
            ),
        ));
    }
    checks
}

/// `$PnS` (or `$PnN`) equals the marker, or the label starts with it as a
/// word (`CD3 BUV395`), ignoring case. `CD3` does not match `CD38`.
fn matches_marker(parameter: &Parameter, marker: &str) -> bool {
    let equals = |value: &str| value.trim().eq_ignore_ascii_case(marker);
    let label_starts = parameter.label.as_deref().is_some_and(|label| {
        label
            .split_whitespace()
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case(marker))
    });
    equals(&parameter.name) || parameter.label.as_deref().is_some_and(equals) || label_starts
}

/// A conventional sample's spillover matrix exists, names existing channels,
/// is invertible and reasonably conditioned.
pub(crate) fn check_spillover(scope: &str, dataset: &FcsDataset) -> Check {
    let spillover = match Spillover::from_dataset(dataset) {
        Ok(Some(spillover)) => spillover,
        Ok(None) => {
            return Check::new(
                scope,
                "spillover",
                CheckStatus::Fail,
                "no `$SPILLOVER`/`$SPILL` keyword; conventional runs need one (or compensate with an autospill matrix)".to_owned(),
            );
        }
        Err(error) => {
            return Check::new(
                scope,
                "spillover",
                CheckStatus::Fail,
                format!("invalid spillover keyword: {error}"),
            );
        }
    };
    let missing = spillover
        .channels()
        .iter()
        .filter(|channel| dataset.find_parameter(channel).is_none())
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Check::new(
            scope,
            "spillover",
            CheckStatus::Fail,
            format!("spillover channels not in the file: {}", missing.join(", ")),
        );
    }
    match cytiq_fcs::condition_number_1(spillover.matrix(), spillover.size()) {
        Err(_) => Check::new(
            scope,
            "spillover",
            CheckStatus::Fail,
            "the spillover matrix is singular".to_owned(),
        ),
        Ok(condition) if condition > SPILLOVER_CONDITION_WARN => Check::new(
            scope,
            "spillover",
            CheckStatus::Warn,
            format!(
                "{} channel(s); condition number {condition:.1} exceeds {SPILLOVER_CONDITION_WARN}: compensation will amplify noise",
                spillover.size()
            ),
        ),
        Ok(condition) => Check::new(
            scope,
            "spillover",
            CheckStatus::Pass,
            format!(
                "{} channel(s); condition number {condition:.1}",
                spillover.size()
            ),
        ),
    }
}

/// Every file of a panel has the same detector channels as its first sample,
/// at the same voltages.
pub(crate) fn check_detectors(files: &[&LoadedFile]) -> Vec<Check> {
    let datasets = files
        .iter()
        .filter_map(|file| file.dataset.as_ref().map(|dataset| (file, dataset)))
        .collect::<Vec<_>>();
    let Some((reference_file, reference)) = datasets.first() else {
        return Vec::new();
    };
    let reference_channels = channel_set(reference);
    let reference_voltages = voltages(reference);
    let mut checks = Vec::new();
    for (file, dataset) in datasets.iter().skip(1) {
        let channels = channel_set(dataset);
        let missing = reference_channels.difference(&channels).collect::<Vec<_>>();
        let extra = channels.difference(&reference_channels).collect::<Vec<_>>();
        if !missing.is_empty() || !extra.is_empty() {
            checks.push(Check::new(
                &file.scope,
                "detector_configuration",
                CheckStatus::Fail,
                format!(
                    "channels differ from {}: missing [{}], extra [{}]",
                    reference_file.scope,
                    join(&missing),
                    join(&extra)
                ),
            ));
            continue;
        }
        let voltages = voltages(dataset);
        let changed = reference_voltages
            .iter()
            .filter(|(channel, voltage)| {
                voltages
                    .get(*channel)
                    .is_some_and(|other| other != *voltage)
            })
            .map(|(channel, _)| channel.as_str())
            .collect::<Vec<_>>();
        checks.push(if changed.is_empty() {
            Check::new(
                &file.scope,
                "detector_configuration",
                CheckStatus::Pass,
                format!("same channels and voltages as {}", reference_file.scope),
            )
        } else {
            Check::new(
                &file.scope,
                "detector_configuration",
                CheckStatus::Warn,
                format!(
                    "`$PnV` differs from {} on: {}",
                    reference_file.scope,
                    changed.join(", ")
                ),
            )
        });
    }
    checks
}

fn channel_set(dataset: &FcsDataset) -> BTreeSet<String> {
    dataset
        .parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .filter(|name| name != cytiq_fcs::ORIGINAL_ID_PARAMETER)
        .collect()
}

fn voltages(dataset: &FcsDataset) -> BTreeMap<String, String> {
    dataset
        .parameters
        .iter()
        .filter_map(|parameter| {
            let key = format!("$P{}V", parameter.index);
            parameter
                .keywords
                .iter()
                .find_map(|(candidate, value)| (*candidate == key).then(|| value.trim().to_owned()))
                .map(|voltage| (parameter.name.clone(), voltage))
        })
        .collect()
}

fn join(values: &[&String]) -> String {
    values
        .iter()
        .map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A single-stain control's primary channel exists in its file.
pub(crate) fn check_primary_channel(scope: &str, dataset: &FcsDataset, channel: &str) -> Check {
    if dataset.find_parameter(channel).is_some() {
        Check::new(
            scope,
            "primary_channel",
            CheckStatus::Pass,
            format!("primary channel `{channel}` is present"),
        )
    } else {
        Check::new(
            scope,
            "primary_channel",
            CheckStatus::Fail,
            format!("primary channel `{channel}` is not in the file"),
        )
    }
}

/// A panel's external compensation matrix (`source: matrix`) is readable,
/// square, invertible and reasonably conditioned, and every sample has its
/// channels. The samples' own `$SPILLOVER` is then not needed.
pub(crate) fn check_matrix(
    scope: &str,
    path: &Path,
    kind: MatrixKind,
    samples: &[&LoadedFile],
) -> Vec<Check> {
    let kind = match kind {
        MatrixKind::Spillover => cytiq_compensate::MatrixKind::Spillover,
        MatrixKind::Compensation => cytiq_compensate::MatrixKind::Compensation,
    };
    let matrix = match cytiq_compensate::read_matrix_file(path, kind) {
        Ok(matrix) => matrix,
        Err(error) => {
            return vec![Check::new(
                scope,
                "compensation_matrix",
                CheckStatus::Fail,
                error.to_string(),
            )];
        }
    };
    let size = matrix.channels.len();
    // A matrix and its inverse have the same condition number, so this holds
    // for spillover and compensation files alike.
    let mut checks = vec![match cytiq_fcs::condition_number_1(&matrix.values, size) {
        Err(_) => Check::new(
            scope,
            "compensation_matrix",
            CheckStatus::Fail,
            format!("`{}` is singular", path.display()),
        ),
        Ok(condition) if condition > SPILLOVER_CONDITION_WARN => Check::new(
            scope,
            "compensation_matrix",
            CheckStatus::Warn,
            format!(
                "`{}`: {size} channel(s); condition number {condition:.1} exceeds {SPILLOVER_CONDITION_WARN}: compensation will amplify noise",
                path.display()
            ),
        ),
        Ok(condition) => Check::new(
            scope,
            "compensation_matrix",
            CheckStatus::Pass,
            format!(
                "`{}`: {size} channel(s); condition number {condition:.1}",
                path.display()
            ),
        ),
    }];
    for sample in samples {
        let Some(dataset) = &sample.dataset else {
            continue;
        };
        let missing = matrix
            .channels
            .iter()
            .filter(|channel| dataset.find_parameter(channel).is_none())
            .map(String::as_str)
            .collect::<Vec<_>>();
        checks.push(if missing.is_empty() {
            let embedded = matches!(Spillover::from_dataset(dataset), Ok(Some(_)));
            Check::new(
                &sample.scope,
                "compensation_channels",
                CheckStatus::Pass,
                format!(
                    "every channel of `{}` is present{}",
                    path.display(),
                    if embedded {
                        "; the file's own `$SPILLOVER` is not used"
                    } else {
                        ""
                    }
                ),
            )
        } else {
            Check::new(
                &sample.scope,
                "compensation_channels",
                CheckStatus::Fail,
                format!(
                    "channels of `{}` not in the file: {}",
                    path.display(),
                    missing.join(", ")
                ),
            )
        });
    }
    checks
}

/// A panel's controls have distinct dyes.
pub(crate) fn check_control_dyes(
    scope: &str,
    controls: &[(&ControlEntry, &LoadedFile)],
) -> Vec<Check> {
    let mut seen = BTreeSet::new();
    let duplicates = controls
        .iter()
        .map(|(control, _)| control.dye.trim().to_ascii_lowercase())
        .filter(|dye| !seen.insert(dye.clone()))
        .collect::<BTreeSet<_>>();
    if duplicates.is_empty() {
        Vec::new()
    } else {
        vec![Check::new(
            scope,
            "control_dyes",
            CheckStatus::Fail,
            format!(
                "dye(s) with more than one control: {}",
                duplicates.into_iter().collect::<Vec<_>>().join(", ")
            ),
        )]
    }
}

/// Every marker channel of a panel has a single-stain control with that
/// `primary_channel`, as `cytiq autospill` (`source: controls`) and
/// `cytiq autospectral` need. Markers are resolved on the panel's first
/// readable sample; unresolved markers are reported by `panel_marker`.
pub(crate) fn check_control_coverage(
    scope: &str,
    panel: &Panel,
    controls: &[(&ControlEntry, &LoadedFile)],
    sample: Option<&FcsDataset>,
    uncovered: CheckStatus,
) -> Check {
    let single_stains = controls
        .iter()
        .map(|(control, _)| *control)
        .filter(|control| !control.is_unstained())
        .collect::<Vec<_>>();
    let same_channel = |left: &str, right: &str| match sample {
        Some(dataset) => match (dataset.find_parameter(left), dataset.find_parameter(right)) {
            (Some(left), Some(right)) => left == right,
            _ => left == right,
        },
        None => left == right,
    };
    let channels = panel
        .markers
        .iter()
        .filter_map(|marker| match (&marker.channel, sample) {
            (Some(channel), _) => Some((marker.marker.as_str(), channel.clone())),
            (None, Some(dataset)) => match marker_candidates(dataset, marker).as_slice() {
                [index] => Some((
                    marker.marker.as_str(),
                    dataset.parameters[*index].name.clone(),
                )),
                _ => None,
            },
            (None, None) => None,
        })
        .collect::<Vec<_>>();
    let missing = channels
        .iter()
        .filter(|(_, channel)| {
            !single_stains.iter().any(|control| {
                control
                    .primary_channel
                    .as_deref()
                    .is_some_and(|primary| same_channel(primary, channel))
            })
        })
        .map(|(marker, channel)| format!("{marker} ({channel})"))
        .collect::<Vec<_>>();
    let unknown = single_stains
        .iter()
        .filter(|control| control.primary_channel.is_none())
        .map(|control| control.dye.as_str())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Check::new(
            scope,
            "control_coverage",
            CheckStatus::Pass,
            format!(
                "{} marker channel(s) have a single-stain control",
                channels.len()
            ),
        )
    } else if unknown.is_empty() {
        Check::new(
            scope,
            "control_coverage",
            uncovered,
            format!("no single-stain control for {}", missing.join(", ")),
        )
    } else {
        Check::new(
            scope,
            "control_coverage",
            CheckStatus::Warn,
            format!(
                "cannot confirm controls for {}: set `primary_channel` on {}",
                missing.join(", "),
                unknown.join(", ")
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cytiq_fcs::{FcsWriteOptions, FcsWriteParameter};

    use super::*;

    fn dataset(parameters: &[FcsWriteParameter]) -> FcsDataset {
        let columns = vec![vec![1.0_f32, 2.0]; parameters.len()];
        let mut bytes = Vec::new();
        cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
            &mut bytes,
            parameters,
            &columns,
            &FcsWriteOptions::new(),
        )
        .unwrap();
        FcsReader::new(std::io::Cursor::new(bytes)).read().unwrap()
    }

    fn loaded(scope: &str, dataset: FcsDataset) -> LoadedFile {
        LoadedFile {
            scope: scope.to_owned(),
            dataset: Some(dataset),
        }
    }

    #[test]
    fn warns_on_a_label_shared_by_two_channels() {
        let sample = dataset(&[
            FcsWriteParameter::with_label("FL1-A", "CD3"),
            FcsWriteParameter::with_label("FL2-A", "cd3 "),
        ]);
        let panel: Panel =
            serde_json::from_str(r#"{"markers":[{"marker":"CD4","required":false}]}"#).unwrap();
        let checks = check_markers("sample S01", &sample, &panel);
        let shared = checks
            .iter()
            .find(|check| check.check == "unique_labels")
            .expect("unique_labels check");
        assert_eq!(shared.status, CheckStatus::Warn);
        assert!(
            shared.message.contains("FL1-A, FL2-A"),
            "{}",
            shared.message
        );
    }

    #[test]
    fn warns_on_a_voltage_change() {
        let first = loaded(
            "sample S01",
            dataset(&[FcsWriteParameter::new("FL1-A").with_keyword("V", "450")]),
        );
        let second = loaded(
            "sample S02",
            dataset(&[FcsWriteParameter::new("FL1-A").with_keyword("V", "500")]),
        );
        let checks = check_detectors(&[&first, &second]);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Warn);
        assert!(checks[0].message.contains("FL1-A"), "{}", checks[0].message);
    }

    #[test]
    fn fails_on_two_controls_of_one_dye() {
        let control = |dye: &str| ControlEntry {
            file: PathBuf::from(format!("{dye}.fcs")),
            dye: dye.to_owned(),
            primary_channel: None,
            panel: None,
        };
        let file = LoadedFile {
            scope: "control".to_owned(),
            dataset: None,
        };
        let (fitc, again, pe) = (control("FITC"), control("fitc"), control("PE"));
        let checks = check_control_dyes("panel", &[(&fitc, &file), (&again, &file), (&pe, &file)]);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Fail);
        assert!(checks[0].message.contains("fitc"), "{}", checks[0].message);
    }
}
