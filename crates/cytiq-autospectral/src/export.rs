use std::fs;
use std::path::Path;

use log::debug;

use crate::error::AutoSpectralError;
use crate::runner::{AutoSpectralResult, ComponentKind};

/// Write references, unmixed FCS files, diagnostics, and optional debug output.
///
/// # Errors
///
/// Returns an error if any output directory or file cannot be created.
pub fn write_outputs(
    result: &AutoSpectralResult,
    output_dir: &Path,
    debug_dir: Option<&Path>,
) -> Result<(), AutoSpectralError> {
    let references_dir = output_dir.join("references");
    let unmixed_dir = output_dir.join("unmixed");
    let diagnostics_dir = output_dir.join("diagnostics");
    create_dir(&references_dir)?;
    create_dir(&unmixed_dir)?;
    create_dir(&diagnostics_dir)?;

    write_reference_csv(&references_dir.join("fluorophore_spectra.csv"), result)?;
    write_json(
        &references_dir.join("autospectral-references.json"),
        &result.references,
    )?;
    for sample in &result.samples {
        debug!("writing unmixed FCS to {}", sample.output_path.display());
        cytiq_fcs::write_fcs3_1_f32(&sample.output_path, &sample.parameters, &sample.columns)
            .map_err(|source| AutoSpectralError::FcsWrite {
                path: sample.output_path.clone(),
                source,
            })?;
    }
    write_json(&diagnostics_dir.join("run.json"), &result.summary)?;

    if let Some(debug_dir) = debug_dir {
        create_dir(debug_dir)?;
        write_json(&debug_dir.join("summary.json"), &result.summary)?;
    }

    Ok(())
}

fn create_dir(path: &Path) -> Result<(), AutoSpectralError> {
    fs::create_dir_all(path).map_err(|source| AutoSpectralError::OutputDirectoryCreate {
        path: path.to_path_buf(),
        source,
    })
}

fn write_reference_csv(path: &Path, result: &AutoSpectralResult) -> Result<(), AutoSpectralError> {
    let mut writer =
        csv::Writer::from_path(path).map_err(|source| AutoSpectralError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    let mut header = vec!["component".to_owned(), "kind".to_owned()];
    header.extend(result.references.detectors.iter().cloned());
    writer
        .write_record(header)
        .map_err(|source| AutoSpectralError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for component in &result.references.components {
        let mut record = vec![
            component.name.clone(),
            match component.kind {
                ComponentKind::Fluorophore => "fluorophore".to_owned(),
                ComponentKind::Autofluorescence => "autofluorescence".to_owned(),
            },
        ];
        record.extend(
            component
                .spectrum
                .iter()
                .map(|value| format!("{value:.12}")),
        );
        writer
            .write_record(record)
            .map_err(|source| AutoSpectralError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer
        .flush()
        .map_err(|source| AutoSpectralError::CsvFlush {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(())
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), AutoSpectralError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|source| AutoSpectralError::JsonSerialize {
            path: path.to_path_buf(),
            source,
        })?;
    fs::write(path, bytes).map_err(|source| AutoSpectralError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })
}
