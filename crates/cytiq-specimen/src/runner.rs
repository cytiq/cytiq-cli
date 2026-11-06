//! Package workflows behind the CLI; all output layout lives in this library.

use crate::{
    Result, SpecimenLayout, SpecimenPlan,
    reconcile::{SourceObservations, reconcile},
    require,
};
use cytiq_manifest::RunManifest;
use cytiq_population::{
    EventAnnotations, ObserveConfig,
    artifact::{verify_package, write_json, write_package},
    serialization, table,
};
use cytiq_provenance::{
    RunRecorder,
    outcome::{CommandOutcome, OutcomeReason},
};
use cytiq_qc::CheckStatus;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct PlanConfig {
    pub manifest: PathBuf,
    pub layout: PathBuf,
    pub specimen: String,
    pub selections: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ObserveRequest {
    pub plan: PathBuf,
    pub acquisition: String,
    pub source: String,
    pub annotations: PathBuf,
    pub producer_provenance: PathBuf,
    pub mask: PathBuf,
    pub qc_report: PathBuf,
    pub qc_provenance: PathBuf,
}

fn selections(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut selections = BTreeMap::new();
    for value in values {
        let (panel, acquisition) = value.split_once('=').ok_or_else(|| {
            crate::SpecimenError::Invalid("selection must be PANEL=ACQUISITION".into())
        })?;
        require(
            !panel.is_empty()
                && !acquisition.is_empty()
                && selections
                    .insert(panel.to_owned(), acquisition.to_owned())
                    .is_none(),
            "empty or duplicate panel selection",
        )?;
    }
    Ok(selections)
}

fn build(config: &PlanConfig, recorder: &mut RunRecorder) -> Result<SpecimenPlan> {
    let manifest = RunManifest::load(&config.manifest)?;
    let layout = SpecimenLayout::load(&config.layout)?;
    let mut files = vec![config.manifest.clone(), config.layout.clone()];
    files.extend(layout.input_files());
    files.extend(
        manifest
            .samples
            .iter()
            .filter(|s| s.specimen.as_deref() == Some(&config.specimen))
            .map(|s| s.file.clone()),
    );
    recorder.snapshot_inputs("input", &files)?;
    SpecimenPlan::build(
        &config.manifest,
        &config.layout,
        &config.specimen,
        &selections(&config.selections)?,
    )
}

/// # Errors
/// Returns configuration, definition or acquisition validation failures.
pub fn validate_plan(config: &PlanConfig, mut recorder: RunRecorder) -> Result<CommandOutcome> {
    let plan = build(config, &mut recorder)?;
    recorder.prepare_staged(&[])?;
    Ok(CommandOutcome::new(
        "cytiq specimen validate",
        true,
        if plan.missing_required() {
            OutcomeReason::Incomplete
        } else {
            OutcomeReason::Success
        },
        String::new(),
    ))
}

/// # Errors
/// Rejects invalid input and output conflicts without replacing prior results.
pub fn write_plan(
    config: &PlanConfig,
    output: &Path,
    overwrite: bool,
    mut recorder: RunRecorder,
) -> Result<CommandOutcome> {
    let plan = build(config, &mut recorder)?;
    let reason = if plan.missing_required() {
        OutcomeReason::Incomplete
    } else {
        OutcomeReason::Success
    };
    Ok(write_package(
        output,
        overwrite,
        recorder,
        "cytiq specimen plan",
        reason,
        |staged| {
            write_json(&staged.join("plan.json"), &plan)?;
            Ok(vec!["plan.json".into()])
        },
    )?)
}

fn prepare_observe(request: &ObserveRequest, recorder: &mut RunRecorder) -> Result<ObserveConfig> {
    recorder.snapshot_inputs("plan", &plan_files(&request.plan))?;
    let plan = SpecimenPlan::load(&request.plan)?;
    let context = plan.context(&request.acquisition, &request.source)?;
    let config = ObserveConfig {
        context,
        annotations: request.annotations.clone(),
        producer_provenance: request.producer_provenance.clone(),
        mask: request.mask.clone(),
        qc_report: request.qc_report.clone(),
        qc_provenance: request.qc_provenance.clone(),
    };
    recorder.snapshot_inputs("input", &config.input_files())?;
    Ok(config)
}

/// Validate a source using exactly the checks used by observe; publish nothing.
/// # Errors
/// Returns invalid annotation/lineage/provenance errors.
pub fn validate_population(
    request: &ObserveRequest,
    mut recorder: RunRecorder,
) -> Result<CommandOutcome> {
    let config = prepare_observe(request, &mut recorder)?;
    let (annotations, _) = cytiq_population::observe(&config)?;
    recorder.prepare_staged(&[])?;
    Ok(CommandOutcome::new(
        "cytiq population validate",
        true,
        if annotations.technical_status == "fail" {
            OutcomeReason::ChecksFailed
        } else {
            OutcomeReason::Success
        },
        String::new(),
    ))
}

/// # Errors
/// Rejects invalid sources and publication failures.
pub fn write_observations(
    request: &ObserveRequest,
    output: &Path,
    overwrite: bool,
    mut recorder: RunRecorder,
) -> Result<CommandOutcome> {
    let config = prepare_observe(request, &mut recorder)?;
    let (annotations, observations) = cytiq_population::observe(&config)?;
    let reason = if annotations.technical_status == "fail" {
        OutcomeReason::ChecksFailed
    } else {
        OutcomeReason::Success
    };
    Ok(write_package(
        output,
        overwrite,
        recorder,
        "cytiq population observe",
        reason,
        |staged| {
            write_json(&staged.join("event-annotations.json"), &annotations)?;
            table::write(&staged.join("observations.parquet"), &observations)?;
            Ok(vec![
                "event-annotations.json".into(),
                "observations.parquet".into(),
            ])
        },
    )?)
}

/// Assemble verified summaries; never opens FCS files or native annotation Parquet.
/// # Errors
/// Rejects invalid packages, conflicting sources and publication failures.
pub fn run_specimen(
    plan_path: &Path,
    packages: &[PathBuf],
    output: &Path,
    overwrite: bool,
    mut recorder: RunRecorder,
) -> Result<CommandOutcome> {
    recorder.snapshot_inputs("plan", &plan_files(plan_path))?;
    let plan = SpecimenPlan::load(plan_path)?;
    let mut sources = Vec::new();
    for package in packages {
        let names = [
            "event-annotations.json",
            "observations.parquet",
            "provenance.json",
            "outcome.json",
        ];
        recorder.snapshot_inputs("observations", &names.map(|name| package.join(name)))?;
        let outcome = verify_package(package)?;
        require(
            outcome.command == "cytiq population observe"
                && names[..2]
                    .iter()
                    .all(|name| outcome.artifacts.iter().any(|a| &a.path == name)),
            "not a population observation package",
        )?;
        let annotations: EventAnnotations = serde_json::from_value(serialization::read(
            &package.join("event-annotations.json"),
        )?)?;
        let observations = table::read(&package.join("observations.parquet"))?;
        sources.push(SourceObservations {
            annotations,
            observations,
        });
    }
    let result = reconcile(&plan, &sources)?;
    let reason = if !result.complete {
        OutcomeReason::Incomplete
    } else if result.status == CheckStatus::Fail {
        OutcomeReason::ChecksFailed
    } else {
        OutcomeReason::Success
    };
    Ok(write_package(
        output,
        overwrite,
        recorder,
        "cytiq specimen run",
        reason,
        |staged| {
            write_json(&staged.join("result.json"), &result)?;
            table::write(&staged.join("results.parquet"), &result.results)?;
            Ok(vec!["result.json".into(), "results.parquet".into()])
        },
    )?)
}

fn plan_files(path: &Path) -> Vec<PathBuf> {
    let parent = path.parent().unwrap_or(Path::new("."));
    vec![
        path.to_owned(),
        parent.join("outcome.json"),
        parent.join("provenance.json"),
    ]
}

/// Map only new-family errors; existing commands keep their exit behavior.
#[must_use]
pub fn error_reason(error: &crate::SpecimenError) -> OutcomeReason {
    use cytiq_population::PopulationError as P;
    use cytiq_publish::PublishError;
    match error {
        crate::SpecimenError::Invalid(_) | crate::SpecimenError::Json(_) => {
            OutcomeReason::InvalidConfiguration
        }
        crate::SpecimenError::Population(p) => match p {
            P::Missing(_) => OutcomeReason::Incomplete,
            P::Invalid(_) | P::Json(_) | P::Arrow(_) | P::Parquet(_) => {
                OutcomeReason::InvalidConfiguration
            }
            P::Publish(PublishError::Exists { .. }) => OutcomeReason::OutputConflict,
            P::Io(e) if e.kind() == std::io::ErrorKind::NotFound => OutcomeReason::Incomplete,
            _ => OutcomeReason::RuntimeError,
        },
        crate::SpecimenError::Manifest(cytiq_manifest::ManifestError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            OutcomeReason::Incomplete
        }
        crate::SpecimenError::Manifest(_) => OutcomeReason::InvalidConfiguration,
        crate::SpecimenError::Publish(PublishError::Exists { .. }) => OutcomeReason::OutputConflict,
        crate::SpecimenError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            OutcomeReason::Incomplete
        }
        _ => OutcomeReason::RuntimeError,
    }
}
