//! Output files. Column names follow `CompensAID`'s `matrixInfo` in `snake_case`;
//! `NA` is an empty cell in CSV and `null` in JSON.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::batch::BatchRow;
use crate::compensaid::CompensaidResult;
use crate::error::SpillQcError;

pub const SUMMARY_FILE: &str = "summary.json";
pub const BATCH_FILE: &str = "batch.csv";
pub const REPORT_FILE: &str = "report.html";
pub const SSI_MATRIX_FILE: &str = "ssi_matrix.csv";
pub const SSI_SEGMENTS_FILE: &str = "ssi_segments.csv";
pub const CUTOFFS_FILE: &str = "cutoffs.csv";

/// `CompensAID`'s `matrix`: rows are secondary channels, columns primary
/// channels, in the layout of cytiq's matrix CSV files.
pub(crate) fn write_ssi_matrix(path: &Path, result: &CompensaidResult) -> Result<(), SpillQcError> {
    let mut rows = Vec::with_capacity(result.channels.len() + 1);
    let mut header = vec!["secondary\\primary".to_owned()];
    header.extend(
        result
            .channels
            .iter()
            .map(|channel| channel.channel.clone()),
    );
    rows.push(header);
    for (secondary, channel) in result.channels.iter().enumerate() {
        let mut row = vec![channel.channel.clone()];
        for primary in 0..result.channels.len() {
            row.push(optional(
                result.cell(secondary, primary).and_then(|pair| pair.ssi),
            ));
        }
        rows.push(row);
    }
    write_csv(path, &rows)
}

/// `CompensAID`'s `matrixInfo`: one row per pair and segment.
pub(crate) fn write_ssi_segments(
    path: &Path,
    result: &CompensaidResult,
) -> Result<(), SpillQcError> {
    let mut rows = vec![
        [
            "primary_marker",
            "primary_channel",
            "secondary_marker",
            "secondary_channel",
            "segment",
            "secondary_cutoff",
            "primary_cutoff_neg",
            "primary_cutoff_pos",
            "segment_min",
            "segment_max",
            "event_count",
            "event_count_merge",
            "mfi_neg",
            "sd_neg",
            "mfi_pos",
            "ssi",
            "message",
            "status",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    for pair in &result.pairs {
        let primary = &result.channels[pair.primary];
        let secondary = &result.channels[pair.secondary];
        for segment in &pair.segments {
            rows.push(vec![
                primary.marker.clone(),
                primary.channel.clone(),
                secondary.marker.clone(),
                secondary.channel.clone(),
                segment.segment.to_string(),
                number(pair.secondary_cutoff),
                number(pair.primary_cutoff_neg),
                number(pair.primary_cutoff_pos),
                optional(segment.segment_min),
                optional(segment.segment_max),
                segment
                    .event_count
                    .map(|count| count.to_string())
                    .unwrap_or_default(),
                segment
                    .event_count_merge
                    .map(|count| count.to_string())
                    .unwrap_or_default(),
                optional(segment.mfi_neg),
                optional(segment.sd_neg),
                optional(segment.mfi_pos),
                optional(segment.ssi),
                segment.message.clone().unwrap_or_default(),
                crate::compensaid::SsiStatus::of(segment.ssi)
                    .as_str()
                    .to_owned(),
            ]);
        }
    }
    write_csv(path, &rows)
}

/// The density cut-off of every marker channel and all `deGate` candidates.
pub(crate) fn write_cutoffs(path: &Path, result: &CompensaidResult) -> Result<(), SpillQcError> {
    let mut rows = vec![
        ["channel", "marker", "cutoff", "candidates"]
            .map(str::to_owned)
            .to_vec(),
    ];
    for (channel, cutoff) in result.channels.iter().zip(&result.cutoffs) {
        rows.push(vec![
            channel.channel.clone(),
            channel.marker.clone(),
            number(cutoff.cutoff),
            cutoff
                .candidates
                .iter()
                .map(|value| number(*value))
                .collect::<Vec<_>>()
                .join(";"),
        ]);
    }
    write_csv(path, &rows)
}

pub(crate) fn write_batch(path: &Path, batch: &[BatchRow]) -> Result<(), SpillQcError> {
    let mut rows = vec![
        [
            "matrix_id",
            "primary_marker",
            "primary_channel",
            "secondary_marker",
            "secondary_channel",
            "files",
            "files_assessed",
            "files_over",
            "files_under",
            "median_ssi",
            "status",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    for row in batch {
        rows.push(vec![
            row.matrix_id.clone(),
            row.primary_marker.clone(),
            row.primary_channel.clone(),
            row.secondary_marker.clone(),
            row.secondary_channel.clone(),
            row.files.to_string(),
            row.files_assessed.to_string(),
            row.files_over.to_string(),
            row.files_under.to_string(),
            optional(row.median_ssi),
            row.status.as_str().to_owned(),
        ]);
    }
    write_csv(path, &rows)
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<(), SpillQcError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| SpillQcError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), SpillQcError> {
    fs::write(path, bytes).map_err(|source| SpillQcError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn write_csv(path: &Path, rows: &[Vec<String>]) -> Result<(), SpillQcError> {
    let csv_error = |source| SpillQcError::Csv {
        path: path.to_path_buf(),
        source,
    };
    let mut writer = csv::Writer::from_path(path).map_err(csv_error)?;
    for row in rows {
        writer.write_record(row).map_err(csv_error)?;
    }
    writer.flush().map_err(|source| SpillQcError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// R's spelling of a double: `NA`, `Inf` and `-Inf` stay recognisable.
fn number(value: f64) -> String {
    if value.is_nan() {
        String::new()
    } else if value == f64::INFINITY {
        "Inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        value.to_string()
    }
}

fn optional(value: Option<f64>) -> String {
    value.map(number).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_spell_na_and_infinities_like_r() {
        assert_eq!(number(f64::NAN), "");
        assert_eq!(number(f64::NEG_INFINITY), "-Inf");
        assert_eq!(number(-1.23), "-1.23");
        assert_eq!(optional(None), "");
    }
}
