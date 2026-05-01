#![allow(
    clippy::cast_precision_loss,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

use std::fs;
use std::path::Path;

use nalgebra::{DMatrix, SymmetricEigen};
use plotly::common::{Mode, Title};
use plotly::{Layout, Plot, Scatter};
use serde::Serialize;

use crate::error::CytoScanError;
use crate::runner::{CytoScanFeatureMatrix, CytoScanResult};

const SUMMARY_FILE_NAME: &str = "cytoscan_summary.csv";
const FEATURES_FILE_NAME: &str = "cytoscan_features.csv";
const CONFIG_FILE_NAME: &str = "cytoscan_config.json";
const REFERENCE_FEATURES_FILE_NAME: &str = "reference_features.csv";
const AGGREGATE_SAMPLING_FILE_NAME: &str = "aggregate_sampling.csv";
const MODEL_DIAGNOSTICS_FILE_NAME: &str = "model_diagnostics.json";
const PCA_COORDINATES_FILE_NAME: &str = "pca_coordinates.csv";
const PCA_LOADINGS_FILE_NAME: &str = "pca_loadings.csv";
const REPORT_FILE_NAME: &str = "report.html";

#[derive(Serialize)]
struct ConfigDocument<'a> {
    parameters: &'a crate::runner::CytoScanParameters,
    diagnostics: &'a crate::runner::CytoScanDiagnostics,
}

#[derive(Clone, Debug)]
struct PcaResult {
    coordinates: Vec<PcaCoordinate>,
    loadings: Vec<PcaLoading>,
}

#[derive(Clone, Debug)]
struct PcaCoordinate {
    source: String,
    filename: String,
    pc1: f64,
    pc2: f64,
}

#[derive(Clone, Debug)]
struct PcaLoading {
    feature: String,
    pc1: f64,
    pc2: f64,
}

pub(crate) fn write_outputs(
    config: &crate::runner::CytoScanConfig,
    result: &CytoScanResult,
) -> Result<(), CytoScanError> {
    fs::create_dir_all(&config.output_dir).map_err(|source| {
        CytoScanError::OutputDirectoryCreate {
            path: config.output_dir.clone(),
            source,
        }
    })?;
    write_summary_csv(&config.output_dir.join(SUMMARY_FILE_NAME), result)?;
    write_feature_matrix_csv(
        &config.output_dir.join(FEATURES_FILE_NAME),
        &result.test_features,
    )?;
    write_config_json(&config.output_dir.join(CONFIG_FILE_NAME), result)?;
    if let Some(debug_dir) = config.debug_dir.as_deref() {
        write_debug_artifacts(debug_dir, result)?;
    }
    Ok(())
}

fn write_summary_csv(path: &Path, result: &CytoScanResult) -> Result<(), CytoScanError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| CytoScanError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .write_record([
            "filename",
            "path",
            "flag_method",
            "feature_method",
            "is_outlier",
            "outlier_score",
            "is_novelty",
            "novelty_log_likelihood",
            "channels",
            "warnings",
        ])
        .map_err(|source| CytoScanError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for row in &result.summary_rows {
        writer
            .write_record([
                row.filename.clone(),
                row.path.display().to_string(),
                row.flag_method.as_str().to_owned(),
                row.feature_method.as_str().to_owned(),
                option_bool(row.is_outlier),
                option_f64(row.outlier_score),
                option_bool(row.is_novelty),
                option_f64(row.novelty_log_likelihood),
                row.channels.clone(),
                row.warnings.clone(),
            ])
            .map_err(|source| CytoScanError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| CytoScanError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_feature_matrix_csv(
    path: &Path,
    matrix: &CytoScanFeatureMatrix,
) -> Result<(), CytoScanError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| CytoScanError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    let mut header = vec!["filename".to_owned(), "path".to_owned()];
    header.extend(matrix.feature_names.clone());
    writer
        .write_record(header)
        .map_err(|source| CytoScanError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for row in &matrix.rows {
        let mut record = vec![row.filename.clone(), row.path.display().to_string()];
        record.extend(row.values.iter().map(|value| format!("{value:.12}")));
        writer
            .write_record(record)
            .map_err(|source| CytoScanError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| CytoScanError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_config_json(path: &Path, result: &CytoScanResult) -> Result<(), CytoScanError> {
    let document = ConfigDocument {
        parameters: &result.parameters,
        diagnostics: &result.diagnostics,
    };
    let bytes =
        serde_json::to_vec_pretty(&document).map_err(|source| CytoScanError::JsonSerialize {
            path: path.to_path_buf(),
            source,
        })?;
    fs::write(path, bytes).map_err(|source| CytoScanError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn write_debug_artifacts(debug_dir: &Path, result: &CytoScanResult) -> Result<(), CytoScanError> {
    fs::create_dir_all(debug_dir).map_err(|source| CytoScanError::DebugDirectoryCreate {
        path: debug_dir.to_path_buf(),
        source,
    })?;
    if let Some(reference_features) = result.reference_features.as_ref() {
        write_feature_matrix_csv(
            &debug_dir.join(REFERENCE_FEATURES_FILE_NAME),
            reference_features,
        )?;
    }
    write_aggregate_sampling_csv(&debug_dir.join(AGGREGATE_SAMPLING_FILE_NAME), result)?;
    write_model_diagnostics_json(&debug_dir.join(MODEL_DIAGNOSTICS_FILE_NAME), result)?;
    let pca = calculate_pca(result);
    write_pca_coordinates_csv(&debug_dir.join(PCA_COORDINATES_FILE_NAME), &pca)?;
    write_pca_loadings_csv(&debug_dir.join(PCA_LOADINGS_FILE_NAME), &pca)?;
    write_report_html(&debug_dir.join(REPORT_FILE_NAME), result, &pca)
}

fn write_aggregate_sampling_csv(path: &Path, result: &CytoScanResult) -> Result<(), CytoScanError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| CytoScanError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .write_record(["aggregate_source", "aggregate_events", "file"])
        .map_err(|source| CytoScanError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for file in &result.diagnostics.aggregate_files {
        writer
            .write_record([
                result.diagnostics.aggregate_source.clone(),
                result.diagnostics.aggregate_events.to_string(),
                file.display().to_string(),
            ])
            .map_err(|source| CytoScanError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| CytoScanError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_model_diagnostics_json(path: &Path, result: &CytoScanResult) -> Result<(), CytoScanError> {
    let bytes = serde_json::to_vec_pretty(&result.diagnostics).map_err(|source| {
        CytoScanError::JsonSerialize {
            path: path.to_path_buf(),
            source,
        }
    })?;
    fs::write(path, bytes).map_err(|source| CytoScanError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn write_pca_coordinates_csv(path: &Path, pca: &PcaResult) -> Result<(), CytoScanError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| CytoScanError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .write_record(["source", "filename", "pc1", "pc2"])
        .map_err(|source| CytoScanError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for row in &pca.coordinates {
        writer
            .write_record([
                row.source.clone(),
                row.filename.clone(),
                format!("{:.12}", row.pc1),
                format!("{:.12}", row.pc2),
            ])
            .map_err(|source| CytoScanError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| CytoScanError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_pca_loadings_csv(path: &Path, pca: &PcaResult) -> Result<(), CytoScanError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| CytoScanError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .write_record(["feature", "pc1", "pc2"])
        .map_err(|source| CytoScanError::CsvWrite {
            path: path.to_path_buf(),
            source,
        })?;
    for row in &pca.loadings {
        writer
            .write_record([
                row.feature.clone(),
                format!("{:.12}", row.pc1),
                format!("{:.12}", row.pc2),
            ])
            .map_err(|source| CytoScanError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| CytoScanError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_report_html(
    path: &Path,
    result: &CytoScanResult,
    pca: &PcaResult,
) -> Result<(), CytoScanError> {
    let plot = pca_plot(pca);
    let plots = plot.to_inline_html(Some("cytoscan-pca"));
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CytoScan Report</title>
{plotly_js}
<style>
body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2rem; color: #1f2933; }}
table {{ border-collapse: collapse; margin-top: 1rem; }}
td, th {{ border: 1px solid #d9e2ec; padding: 0.45rem 0.65rem; text-align: left; }}
th {{ background: #f0f4f8; }}
code {{ background: #f0f4f8; padding: 0.1rem 0.25rem; }}
.plot-panel {{ margin-top: 1.5rem; border: 1px solid #d9e2ec; padding: 1rem; }}
.js-plotly-plot, .plotly, .plotly-graph-div {{ width: 100% !important; }}
</style>
</head>
<body>
<h1>CytoScan Report</h1>
<table>
<tr><th>Metric</th><th>Value</th></tr>
<tr><td>Test files</td><td>{test_files}</td></tr>
<tr><td>Feature method</td><td>{feature_method}</td></tr>
<tr><td>Flag method</td><td>{flag_method}</td></tr>
<tr><td>Features</td><td>{feature_count}</td></tr>
<tr><td>Aggregate source</td><td>{aggregate_source}</td></tr>
</table>
<section class="plot-panel"><h2>PCA</h2>{plots}</section>
</body>
</html>
"#,
        plotly_js = Plot::online_cdn_js(),
        test_files = result.summary_rows.len(),
        feature_method = result.parameters.feature_method.as_str(),
        flag_method = result.parameters.flag_method.as_str(),
        feature_count = result.diagnostics.feature_count,
        aggregate_source = escape_html(&result.diagnostics.aggregate_source),
        plots = plots
    );
    fs::write(path, html).map_err(|source| CytoScanError::HtmlWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn calculate_pca(result: &CytoScanResult) -> PcaResult {
    let mut labels = Vec::new();
    let mut matrix = Vec::new();
    if let Some(reference) = result.reference_features.as_ref() {
        for row in &reference.rows {
            labels.push(("reference".to_owned(), row.filename.clone()));
            matrix.push(row.values.clone());
        }
    }
    for row in &result.test_features.rows {
        labels.push(("test".to_owned(), row.filename.clone()));
        matrix.push(row.values.clone());
    }
    if matrix.is_empty() {
        return empty_pca(result);
    }
    let feature_count = matrix[0].len();
    if matrix.len() < 2 || feature_count == 0 {
        return zero_pca(result, labels);
    }
    let centered = centered_matrix(&matrix);
    let covariance = (&centered.transpose() * &centered) / (matrix.len() - 1) as f64;
    let eigen = SymmetricEigen::new(covariance);
    let mut order = (0..feature_count).collect::<Vec<_>>();
    order.sort_by(|left, right| eigen.eigenvalues[*right].total_cmp(&eigen.eigenvalues[*left]));
    let pc1_index = order[0];
    let pc2_index = *order.get(1).unwrap_or(&pc1_index);
    let pc1_vector = eigen.eigenvectors.column(pc1_index);
    let pc2_vector = eigen.eigenvectors.column(pc2_index);
    let coordinates_matrix = centered * DMatrix::from_columns(&[pc1_vector, pc2_vector]);
    let coordinates = labels
        .into_iter()
        .enumerate()
        .map(|(index, (source, filename))| PcaCoordinate {
            source,
            filename,
            pc1: coordinates_matrix[(index, 0)],
            pc2: if feature_count > 1 {
                coordinates_matrix[(index, 1)]
            } else {
                0.0
            },
        })
        .collect();
    let loadings = result
        .test_features
        .feature_names
        .iter()
        .enumerate()
        .map(|(index, feature)| PcaLoading {
            feature: feature.clone(),
            pc1: pc1_vector[index],
            pc2: if feature_count > 1 {
                pc2_vector[index]
            } else {
                0.0
            },
        })
        .collect();
    PcaResult {
        coordinates,
        loadings,
    }
}

fn centered_matrix(matrix: &[Vec<f64>]) -> DMatrix<f64> {
    let rows = matrix.len();
    let columns = matrix[0].len();
    let means = (0..columns)
        .map(|column| matrix.iter().map(|row| row[column]).sum::<f64>() / rows as f64)
        .collect::<Vec<_>>();
    DMatrix::from_fn(rows, columns, |row, column| {
        matrix[row][column] - means[column]
    })
}

fn empty_pca(result: &CytoScanResult) -> PcaResult {
    zero_pca(result, Vec::new())
}

fn zero_pca(result: &CytoScanResult, labels: Vec<(String, String)>) -> PcaResult {
    let coordinates = labels
        .into_iter()
        .map(|(source, filename)| PcaCoordinate {
            source,
            filename,
            pc1: 0.0,
            pc2: 0.0,
        })
        .collect();
    let loadings = result
        .test_features
        .feature_names
        .iter()
        .map(|feature| PcaLoading {
            feature: feature.clone(),
            pc1: 0.0,
            pc2: 0.0,
        })
        .collect();
    PcaResult {
        coordinates,
        loadings,
    }
}

fn pca_plot(pca: &PcaResult) -> Plot {
    let mut plot = Plot::new();
    let x = pca
        .coordinates
        .iter()
        .map(|row| row.pc1)
        .collect::<Vec<_>>();
    let y = pca
        .coordinates
        .iter()
        .map(|row| row.pc2)
        .collect::<Vec<_>>();
    let text = pca
        .coordinates
        .iter()
        .map(|row| format!("{}: {}", row.source, row.filename))
        .collect::<Vec<_>>();
    plot.add_trace(
        Scatter::new(x, y)
            .mode(Mode::Markers)
            .text_array(text)
            .name("files"),
    );
    plot.set_layout(Layout::new().title(Title::with_text("Feature PCA")));
    plot
}

fn option_bool(value: Option<bool>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.12}"))
        .unwrap_or_default()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
