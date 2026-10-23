mod compensation;
mod data;
mod error;
mod header;
mod layout;
mod text;
mod types;
mod writer;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use log::debug;

use crate::data::{decode_data, read_raw_segment};
use crate::error::FcsError;
use crate::header::read_header;
use crate::layout::resolve_layout;
use crate::text::{build_text, parse_segment_range, read_text_segment};
use crate::types::{ByteRange, FcsText, Header};

pub use crate::compensation::{Spillover, compensate, find_parameter_index, invert_matrix};
pub use crate::error::FcsError as Error;
pub use crate::types::{
    ColumnData, EventColumns, FcsDataset, FcsText as Text, FcsVersion, Parameter,
    ParameterDataType, ParameterWidth, TextSegment,
};
pub use crate::writer::{
    FcsWriteOptions, FcsWriteParameter, carried_keywords, write_fcs3_1_f32,
    write_fcs3_1_f32_to_writer, write_fcs3_1_f32_to_writer_with_options,
    write_fcs3_1_f32_with_options, write_fcs3_1_f64_to_writer_with_options,
    write_fcs3_1_f64_with_options,
};

#[derive(Debug)]
pub struct FcsReader<R> {
    reader: R,
}

impl FcsReader<File> {
    /// Open an FCS file from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be opened for reading.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FcsError> {
        debug!("opening FCS file {}", path.as_ref().display());
        let reader = File::open(path).map_err(FcsError::Io)?;
        Ok(Self::new(reader))
    }
}

impl<R: Read + Seek> FcsReader<R> {
    pub const fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Read the FCS dataset from the wrapped reader.
    ///
    /// # Errors
    ///
    /// Returns an error when the file does not conform to the supported FCS
    /// 3.0, 3.1, or 3.2 layouts, when required metadata is missing, or when the
    /// underlying reader fails.
    pub fn read(mut self) -> Result<FcsDataset, FcsError> {
        let file_len = file_length(&mut self.reader)?;
        self.reader.seek(SeekFrom::Start(0)).map_err(FcsError::Io)?;

        let header = read_header(&mut self.reader)?;
        let primary_segment = read_text_segment(&mut self.reader, header.text, "TEXT")?;
        let primary_text = build_text(primary_segment.clone(), None)?;
        let supplemental_range = parse_segment_range(
            primary_text.primary(),
            "$BEGINSTEXT",
            "$ENDSTEXT",
            "supplemental TEXT",
            false,
        )?;
        let supplemental = supplemental_range
            .map(|range| read_text_segment(&mut self.reader, range, "supplemental TEXT"))
            .transpose()?;
        let text = build_text(primary_segment, supplemental)?;

        let data_range = resolve_required_range(&header, &text, "$BEGINDATA", "$ENDDATA", "DATA")?;
        let analysis_range =
            resolve_optional_range(&header, &text, "$BEGINANALYSIS", "$ENDANALYSIS", "ANALYSIS")?;
        validate_segments(
            file_len,
            header.text,
            supplemental_range,
            data_range,
            analysis_range,
        )?;

        let resolved_layout = resolve_layout(header.version, &text)?;
        let parameters = resolved_layout.public_parameters();
        let data = decode_data(&mut self.reader, data_range, &resolved_layout)?;
        let analysis = analysis_range
            .map(|range| read_raw_segment(&mut self.reader, range))
            .transpose()?;

        let event_count = data.columns.first().map_or(0, ColumnData::len);
        debug!(
            "parsed FCS dataset: version={:?}, parameters={}, events={}, analysis_bytes={}",
            header.version,
            parameters.len(),
            event_count,
            analysis.as_ref().map_or(0, Vec::len)
        );

        Ok(FcsDataset {
            version: header.version,
            header,
            text,
            parameters,
            data,
            analysis,
        })
    }
}

fn file_length<R: Read + Seek>(reader: &mut R) -> Result<u64, FcsError> {
    let current = reader.stream_position().map_err(FcsError::Io)?;
    let end = reader.seek(SeekFrom::End(0)).map_err(FcsError::Io)?;
    reader
        .seek(SeekFrom::Start(current))
        .map_err(FcsError::Io)?;
    Ok(end)
}

fn resolve_required_range(
    header: &Header,
    text: &FcsText,
    begin_key: &str,
    end_key: &str,
    name: &'static str,
) -> Result<ByteRange, FcsError> {
    let text_range = parse_segment_range(text.primary(), begin_key, end_key, name, true)?
        .ok_or_else(|| {
            FcsError::MissingKeyword(match name {
                "DATA" => begin_key.to_owned(),
                _ => end_key.to_owned(),
            })
        })?;
    let header_range = match name {
        "DATA" => header.data,
        "ANALYSIS" => header.analysis,
        _ => None,
    };

    if let Some(range) = header_range {
        ensure_matching_ranges(name, range, text_range)?;
    }

    Ok(text_range)
}

fn resolve_optional_range(
    header: &Header,
    text: &FcsText,
    begin_key: &str,
    end_key: &str,
    name: &'static str,
) -> Result<Option<ByteRange>, FcsError> {
    let text_range = parse_segment_range(text.primary(), begin_key, end_key, name, false)?;
    let header_range = match name {
        "DATA" => header.data,
        "ANALYSIS" => header.analysis,
        _ => None,
    };

    match (header_range, text_range) {
        (Some(header_value), Some(text_value)) => {
            ensure_matching_ranges(name, header_value, text_value)?;
            Ok(Some(text_value))
        }
        (Some(header_value), None) => Ok(Some(header_value)),
        (None, value) => Ok(value),
    }
}

fn ensure_matching_ranges(
    name: &'static str,
    header_value: ByteRange,
    text_value: ByteRange,
) -> Result<(), FcsError> {
    if header_value == text_value {
        Ok(())
    } else {
        Err(FcsError::OffsetMismatch {
            name,
            header_start: header_value.start,
            header_end: header_value.end,
            text_start: text_value.start,
            text_end: text_value.end,
        })
    }
}

fn validate_segments(
    file_len: u64,
    text_range: ByteRange,
    supplemental_text_range: Option<ByteRange>,
    data_range: ByteRange,
    analysis_range: Option<ByteRange>,
) -> Result<(), FcsError> {
    let mut segments = vec![
        ("HEADER", ByteRange::new(0, 57)),
        ("TEXT", text_range),
        ("DATA", data_range),
    ];

    if let Some(range) = supplemental_text_range {
        segments.push(("supplemental TEXT", range));
    }
    if let Some(range) = analysis_range {
        segments.push(("ANALYSIS", range));
    }

    for (name, range) in &segments {
        if range.end >= file_len {
            return Err(FcsError::SegmentOutOfBounds {
                name,
                start: range.start,
                end: range.end,
                file_len,
            });
        }
    }

    segments.sort_by_key(|(_, range)| range.start);
    for window in segments.windows(2) {
        let (first_name, first_range) = window[0];
        let (second_name, second_range) = window[1];
        if second_range.start <= first_range.end {
            return Err(FcsError::SegmentOverlap {
                first: first_name,
                second: second_name,
            });
        }
    }

    Ok(())
}
