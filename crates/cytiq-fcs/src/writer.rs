use std::fs::File;
use std::io::Write;
use std::path::Path;

use log::debug;

use crate::error::FcsError;

const HEADER_LEN: u64 = 58;
const TEXT_DELIMITER: u8 = b'|';
const OFFSET_FIELD_LIMIT: u64 = 99_999_999;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FcsWriteParameter {
    pub name: String,
    pub label: Option<String>,
}

impl FcsWriteParameter {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: None,
        }
    }

    #[must_use]
    pub fn with_label(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: Some(label.into()),
        }
    }
}

/// Write an FCS 3.1 file with little-endian 32-bit float event data.
///
/// `columns` must contain one column per parameter and all columns must have the
/// same number of events. The writer intentionally supports only the compact
/// layout needed for generated cytiq outputs.
///
/// # Errors
///
/// Returns an error when parameters/columns are inconsistent, offsets cannot be
/// represented in the fixed header, values are non-finite, or the underlying
/// writer fails.
pub fn write_fcs3_1_f32(
    path: impl AsRef<Path>,
    parameters: &[FcsWriteParameter],
    columns: &[Vec<f32>],
) -> Result<(), FcsError> {
    let path = path.as_ref();
    debug!(
        "writing FCS 3.1 float32 dataset to {} with {} parameters",
        path.display(),
        parameters.len()
    );
    let mut file = File::create(path).map_err(FcsError::Io)?;
    write_fcs3_1_f32_to_writer(&mut file, parameters, columns)
}

/// Write an FCS 3.1 float32 dataset to any byte writer.
///
/// # Errors
///
/// Returns an error when parameters/columns are inconsistent, offsets cannot be
/// represented in the fixed header, values are non-finite, or the underlying
/// writer fails.
pub fn write_fcs3_1_f32_to_writer<W: Write>(
    writer: &mut W,
    parameters: &[FcsWriteParameter],
    columns: &[Vec<f32>],
) -> Result<(), FcsError> {
    validate_input(parameters, columns)?;

    let row_count = columns.first().map_or(0, Vec::len);
    let mut pairs = common_text_pairs(parameters.len(), row_count);
    for (parameter_index, parameter) in parameters.iter().enumerate() {
        let number = parameter_index + 1;
        if parameter.name.trim().is_empty() {
            return Err(FcsError::InvalidWriterInput(
                "parameter names must not be empty",
            ));
        }
        set_pair(&mut pairs, &format!("$P{number}N"), parameter.name.trim());
        if let Some(label) = parameter
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            set_pair(&mut pairs, &format!("$P{number}S"), label);
        }
        set_pair(&mut pairs, &format!("$P{number}B"), "32");
        set_pair(&mut pairs, &format!("$P{number}E"), "0,0");
        set_pair(
            &mut pairs,
            &format!("$P{number}R"),
            &format!("{:.6}", column_range(&columns[parameter_index])),
        );
    }

    let placeholder_text = encode_text(&pairs);
    let text_start = HEADER_LEN;
    let text_end = text_start
        + u64::try_from(placeholder_text.len()).map_err(|_| {
            FcsError::InvalidWriterInput("TEXT segment length exceeds supported range")
        })?
        - 1;
    let data_start = text_end + 1;
    let data_bytes = data_length(parameters.len(), row_count)?;
    let data_end = data_start + data_bytes - 1;
    if text_end > OFFSET_FIELD_LIMIT {
        return Err(FcsError::InvalidWriterInput(
            "TEXT offsets exceed fixed FCS HEADER fields",
        ));
    }

    set_pair(&mut pairs, "$BEGINDATA", &format!("{data_start:08}"));
    set_pair(&mut pairs, "$ENDDATA", &format!("{data_end:08}"));
    let text = encode_text(&pairs);
    let final_text_end = text_start
        + u64::try_from(text.len()).map_err(|_| {
            FcsError::InvalidWriterInput("TEXT segment length exceeds supported range")
        })?
        - 1;
    if final_text_end != text_end {
        return Err(FcsError::InvalidWriterInput(
            "FCS TEXT offset width changed during serialization",
        ));
    }

    let header = format_header(text_start, text_end);
    writer.write_all(header.as_bytes()).map_err(FcsError::Io)?;
    writer.write_all(&text).map_err(FcsError::Io)?;
    write_data(writer, columns, row_count)?;
    Ok(())
}

fn validate_input(parameters: &[FcsWriteParameter], columns: &[Vec<f32>]) -> Result<(), FcsError> {
    if parameters.is_empty() {
        return Err(FcsError::InvalidWriterInput(
            "at least one parameter is required",
        ));
    }
    if parameters.len() != columns.len() {
        return Err(FcsError::InvalidWriterInput(
            "parameter count must match column count",
        ));
    }
    let Some(first_column) = columns.first() else {
        return Err(FcsError::InvalidWriterInput(
            "at least one column is required",
        ));
    };
    let row_count = first_column.len();
    if row_count == 0 {
        return Err(FcsError::InvalidWriterInput(
            "at least one event row is required",
        ));
    }
    if columns.iter().any(|column| column.len() != row_count) {
        return Err(FcsError::InvalidWriterInput(
            "all columns must have the same length",
        ));
    }
    if columns
        .iter()
        .flat_map(|column| column.iter())
        .any(|value| !value.is_finite())
    {
        return Err(FcsError::InvalidWriterInput(
            "all FCS event values must be finite",
        ));
    }

    Ok(())
}

fn common_text_pairs(parameter_count: usize, row_count: usize) -> Vec<(String, String)> {
    vec![
        ("$BEGINSTEXT".to_owned(), "00000000".to_owned()),
        ("$ENDSTEXT".to_owned(), "00000000".to_owned()),
        ("$BEGINDATA".to_owned(), "00000000".to_owned()),
        ("$ENDDATA".to_owned(), "00000000".to_owned()),
        ("$BEGINANALYSIS".to_owned(), "00000000".to_owned()),
        ("$ENDANALYSIS".to_owned(), "00000000".to_owned()),
        ("$BYTEORD".to_owned(), "1,2,3,4".to_owned()),
        ("$DATATYPE".to_owned(), "F".to_owned()),
        ("$MODE".to_owned(), "L".to_owned()),
        ("$NEXTDATA".to_owned(), "00000000".to_owned()),
        ("$PAR".to_owned(), parameter_count.to_string()),
        ("$TOT".to_owned(), row_count.to_string()),
    ]
}

fn column_range(column: &[f32]) -> f32 {
    let maximum = column.iter().copied().fold(0.0_f32, f32::max);
    maximum.ceil().max(1.0)
}

fn data_length(parameter_count: usize, row_count: usize) -> Result<u64, FcsError> {
    let parameter_count = u64::try_from(parameter_count)
        .map_err(|_| FcsError::InvalidWriterInput("parameter count exceeds supported range"))?;
    let row_count = u64::try_from(row_count)
        .map_err(|_| FcsError::InvalidWriterInput("row count exceeds supported range"))?;
    parameter_count
        .checked_mul(row_count)
        .and_then(|values| values.checked_mul(4))
        .ok_or(FcsError::InvalidWriterInput(
            "DATA segment length exceeds supported range",
        ))
}

fn format_header(text_start: u64, text_end: u64) -> String {
    format!(
        "FCS3.1    {text_start:>8}{text_end:>8}{data_start:>8}{data_end:>8}{analysis_start:>8}{analysis_end:>8}",
        data_start = 0,
        data_end = 0,
        analysis_start = 0,
        analysis_end = 0
    )
}

fn write_data<W: Write>(
    writer: &mut W,
    columns: &[Vec<f32>],
    row_count: usize,
) -> Result<(), FcsError> {
    for row_index in 0..row_count {
        for column in columns {
            writer
                .write_all(&column[row_index].to_le_bytes())
                .map_err(FcsError::Io)?;
        }
    }

    Ok(())
}

fn encode_text(pairs: &[(String, String)]) -> Vec<u8> {
    let mut bytes = vec![TEXT_DELIMITER];
    for (key, value) in pairs {
        push_escaped(&mut bytes, key);
        bytes.push(TEXT_DELIMITER);
        push_escaped(&mut bytes, value);
        bytes.push(TEXT_DELIMITER);
    }

    bytes
}

fn push_escaped(bytes: &mut Vec<u8>, value: &str) {
    for byte in value.as_bytes() {
        bytes.push(*byte);
        if *byte == TEXT_DELIMITER {
            bytes.push(TEXT_DELIMITER);
        }
    }
}

fn set_pair(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = pairs.iter_mut().find(|(current_key, _)| current_key == key) {
        value.clone_into(existing);
    } else {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}
