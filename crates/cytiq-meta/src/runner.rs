use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use cytiq_fcs::{ColumnData, FcsDataset, FcsReader, Parameter, ParameterDataType, ParameterWidth};
use log::{debug, info};
use serde::Serialize;

use crate::error::MetaError;

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaConfig {
    pub input: PathBuf,
    /// `None` writes the document to stdout.
    pub output: Option<PathBuf>,
    pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaRunResult {
    pub metadata: FcsMetadata,
    pub output: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FcsMetadata {
    pub schema_version: u32,
    pub tool: String,
    pub version: String,
    pub file: FileIdentity,
    pub fcs_version: String,
    pub segments: SegmentOffsets,
    pub event_count: usize,
    pub parameter_count: usize,
    pub parameters: Vec<ParameterMetadata>,
    /// Primary TEXT keywords as `[key, value]` pairs in file order.
    pub text: Vec<(String, String)>,
    pub supplemental_text: Option<Vec<(String, String)>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileIdentity {
    pub path: PathBuf,
    pub name: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}

/// Inclusive `[start, end]` byte offsets of each segment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SegmentOffsets {
    pub text: [u64; 2],
    pub data: Option<[u64; 2]>,
    pub analysis: Option<[u64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParameterMetadata {
    pub index: u16,
    pub name: String,
    pub label: Option<String>,
    pub data_type: &'static str,
    pub width: ParameterWidthMetadata,
    pub keywords: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterWidthMetadata {
    Bits(u8),
    AsciiChars(u8),
    DelimitedAscii,
}

#[derive(Debug, Default)]
pub struct MetaRunner;

impl MetaRunner {
    /// Read one FCS file and write its metadata as pretty JSON to the
    /// configured output file or stdout.
    ///
    /// # Errors
    ///
    /// Returns an error when the input cannot be read or parsed, the output
    /// exists without `overwrite`, or the document cannot be written.
    pub fn run(config: &MetaConfig) -> Result<MetaRunResult, MetaError> {
        if let Some(output) = config.output.as_deref() {
            prepare_output(output, config.overwrite)?;
        }

        let metadata = read_metadata(&config.input)?;
        let mut document = serde_json::to_vec_pretty(&metadata)?;
        document.push(b'\n');

        if let Some(output) = config.output.as_deref() {
            write_atomically(output, &document)?;
            info!("wrote FCS metadata to {}", output.display());
        } else {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(&document)
                .and_then(|()| stdout.flush())
                .map_err(|source| MetaError::OutputWrite {
                    path: PathBuf::from("<stdout>"),
                    source,
                })?;
        }

        Ok(MetaRunResult {
            metadata,
            output: config.output.clone(),
        })
    }
}

/// Read the metadata of one FCS file.
///
/// # Errors
///
/// Returns an error when the file cannot be read, hashed or parsed as FCS.
pub fn read_metadata(path: impl AsRef<Path>) -> Result<FcsMetadata, MetaError> {
    let path = path.as_ref();
    debug!("reading FCS metadata from {}", path.display());
    let (bytes, sha256) = hash_file(path)?;
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| MetaError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(FcsMetadata {
        schema_version: SCHEMA_VERSION,
        tool: "cytiq meta".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        file: FileIdentity {
            path: path.to_path_buf(),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            bytes,
            sha256,
        },
        fcs_version: dataset.version.as_str().to_owned(),
        segments: segment_offsets(&dataset),
        event_count: event_count(&dataset),
        parameter_count: dataset.parameters.len(),
        parameters: dataset.parameters.iter().map(parameter_metadata).collect(),
        text: dataset.text.primary.pairs.clone(),
        supplemental_text: dataset
            .text
            .supplemental
            .as_ref()
            .map(|segment| segment.pairs.clone()),
    })
}

fn hash_file(path: &Path) -> Result<(u64, String), MetaError> {
    cytiq_provenance::hash_file(path).map_err(|source| MetaError::Hash {
        path: path.to_path_buf(),
        source,
    })
}

/// HEADER offsets, falling back to the TEXT keywords that large files use
/// once offsets no longer fit the fixed HEADER fields.
fn segment_offsets(dataset: &FcsDataset) -> SegmentOffsets {
    let header = &dataset.header;
    SegmentOffsets {
        text: [header.text.start, header.text.end],
        data: header
            .data
            .map(|range| [range.start, range.end])
            .or_else(|| text_offsets(dataset, "$BEGINDATA", "$ENDDATA")),
        analysis: header
            .analysis
            .map(|range| [range.start, range.end])
            .or_else(|| text_offsets(dataset, "$BEGINANALYSIS", "$ENDANALYSIS")),
    }
}

fn text_offsets(dataset: &FcsDataset, begin_key: &str, end_key: &str) -> Option<[u64; 2]> {
    let begin = dataset.text.get(begin_key)?.trim().parse().ok()?;
    let end = dataset.text.get(end_key)?.trim().parse().ok()?;
    (begin > 0 && end >= begin).then_some([begin, end])
}

/// Decoded event count, falling back to `$TOT` for files without columns.
fn event_count(dataset: &FcsDataset) -> usize {
    dataset
        .data
        .columns
        .first()
        .map(ColumnData::len)
        .or_else(|| dataset.text.get("$TOT")?.trim().parse().ok())
        .unwrap_or(0)
}

fn parameter_metadata(parameter: &Parameter) -> ParameterMetadata {
    ParameterMetadata {
        index: parameter.index,
        name: parameter.name.clone(),
        label: parameter.label.clone(),
        data_type: match parameter.data_type {
            ParameterDataType::Ascii => "ascii",
            ParameterDataType::Integer => "integer",
            ParameterDataType::Float => "float",
            ParameterDataType::Double => "double",
        },
        width: match parameter.width {
            ParameterWidth::Bits(bits) => ParameterWidthMetadata::Bits(bits),
            ParameterWidth::AsciiChars(chars) => ParameterWidthMetadata::AsciiChars(chars),
            ParameterWidth::DelimitedAscii => ParameterWidthMetadata::DelimitedAscii,
        },
        keywords: parameter.keywords.iter().cloned().collect(),
    }
}

fn prepare_output(output: &Path, overwrite: bool) -> Result<(), MetaError> {
    if output.exists() && !overwrite {
        return Err(MetaError::OutputExists {
            path: output.to_path_buf(),
        });
    }
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|source| MetaError::OutputDirectoryCreate {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// Write through a sibling temporary file so a failed run never leaves a
/// truncated metadata record behind.
fn write_atomically(output: &Path, document: &[u8]) -> Result<(), MetaError> {
    let mut temp_name = output.as_os_str().to_owned();
    temp_name.push(".tmp");
    let temp_path = PathBuf::from(temp_name);
    let write_error = |path: &Path| {
        let path = path.to_path_buf();
        move |source| MetaError::OutputWrite { path, source }
    };

    let mut writer = BufWriter::new(File::create(&temp_path).map_err(write_error(&temp_path))?);
    writer
        .write_all(document)
        .and_then(|()| writer.flush())
        .map_err(write_error(&temp_path))?;
    drop(writer);
    fs::rename(&temp_path, output).map_err(write_error(output))
}
