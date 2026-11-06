//! Standalone views and lossless table exports of native specimen summaries.

mod html;
mod tables;

use crate::{Result, SpecimenComparison, SpecimenResult, compare::PackageIdentity, require};
use cytiq_population::artifact::write_json;
use cytiq_provenance::{
    RunRecorder,
    outcome::{CommandOutcome, OutcomeReason},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub const DEFAULT_REPORT_FORMAT: &str = "all";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    #[default]
    All,
    Html,
    Csv,
    Json,
}

#[derive(Clone, Debug)]
pub struct ReportConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub format: ReportFormat,
    pub overwrite: bool,
    pub reuse: cytiq_provenance::reuse::ReusePolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ReportData {
    Specimen(SpecimenResult),
    Comparison(Box<SpecimenComparison>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenReport {
    pub schema_version: u32,
    pub source: PackageIdentity,
    pub source_outcome: CommandOutcome,
    pub content: ReportData,
}

impl SpecimenReport {
    /// Load only the supplied native package; no plan, raw files or remote assets are read.
    /// # Errors
    /// Rejects corrupt packages and inputs other than specimen run/compare packages.
    pub fn load(input: &Path) -> Result<Self> {
        let outcome = cytiq_population::verify_package(input)?;
        let content = match outcome.command.as_str() {
            "cytiq specimen run" => {
                let result = SpecimenResult::load(input)?;
                let result = crate::compare::normalize(&result)?;
                ReportData::Specimen(result)
            }
            "cytiq specimen compare" => {
                ReportData::Comparison(Box::new(SpecimenComparison::load(input)?))
            }
            _ => {
                return Err(crate::SpecimenError::Invalid(
                    "report requires a specimen result or comparison package".into(),
                ));
            }
        };
        let source = crate::runner::package_identity(input, &outcome)?;
        Ok(Self {
            schema_version: 1,
            source,
            source_outcome: outcome,
            content,
        })
    }

    /// Write the requested report files into a publication-owned staging directory.
    /// # Errors
    /// Returns formatting, serialization and filesystem failures.
    pub fn write(&self, staged: &Path, format: ReportFormat) -> Result<Vec<String>> {
        require(
            self.schema_version == 1,
            "unsupported specimen report version",
        )?;
        let mut names = Vec::new();
        if matches!(format, ReportFormat::All | ReportFormat::Json) {
            write_json(&staged.join("report.json"), self)?;
            names.push("report.json".into());
        }
        if matches!(
            format,
            ReportFormat::All | ReportFormat::Html | ReportFormat::Csv
        ) {
            let tables = tables::build(self)?;
            if matches!(format, ReportFormat::All | ReportFormat::Html) {
                html::write(File::create(staged.join("report.html"))?, self, &tables)?;
                names.push("report.html".into());
            }
            if matches!(format, ReportFormat::All | ReportFormat::Csv) {
                for table in tables {
                    table.write_csv(&staged.join(&table.filename))?;
                    names.push(table.filename);
                }
            }
        }
        Ok(names)
    }
}

/// Export one verified result/comparison, independently of its source QC/comparison verdict.
/// # Errors
/// Rejects invalid inputs and conflicts; preserves prior output when publication fails.
pub fn write_report(config: &ReportConfig, mut recorder: RunRecorder) -> Result<CommandOutcome> {
    crate::runner::snapshot_package(&config.input, "report_input", &mut recorder)?;
    let run = match cytiq_population::artifact::begin_package(
        &config.output,
        config.overwrite,
        "cytiq specimen report",
        config.reuse,
        recorder,
    )? {
        cytiq_population::artifact::PackageBegin::Reused(outcome) => return Ok(outcome),
        cytiq_population::artifact::PackageBegin::Compute(run) => run,
    };
    let report = SpecimenReport::load(&config.input)?;
    Ok(
        run.write("cytiq specimen report", OutcomeReason::Success, |staged| {
            report.write(staged, config.format).map_err(|error| {
                cytiq_population::PopulationError::Io(std::io::Error::other(error))
            })
        })?,
    )
}
