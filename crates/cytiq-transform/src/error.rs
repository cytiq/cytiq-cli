use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("at least one `--input` value is required")]
    EmptyInputs,
    #[error("input path `{path}` does not exist")]
    MissingInput { path: PathBuf },
    #[error("input path `{path}` is not an FCS file")]
    NonFcsInput { path: PathBuf },
    #[error("glob pattern `{pattern}` matched no FCS files")]
    EmptyGlob { pattern: String },
    #[error(
        "glob pattern `{pattern}` is not supported; wildcard characters are only supported in the file name"
    )]
    UnsupportedGlob { pattern: String },
    #[error("failed to read directory `{path}`: {source}")]
    DirectoryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("directory input `{path}` contains no `.fcs` files")]
    EmptyDirectory { path: PathBuf },
    #[error("failed to resolve input file `{path}`: {source}")]
    InputCanonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("input file `{path}` was provided more than once")]
    DuplicateInput { path: PathBuf },
    #[error(
        "transform format `{format}` is not supported in the FCS transform MVP; use `--format fcs`"
    )]
    UnsupportedFormat { format: &'static str },
    #[error("{parameter} must be finite and greater than zero")]
    InvalidPositiveParameter { parameter: &'static str },
    #[error("{parameter} must be finite and greater than or equal to zero")]
    InvalidNonNegativeParameter { parameter: &'static str },
    #[error("{parameter} must be finite")]
    InvalidFiniteParameter { parameter: &'static str },
    #[error("failed to read FCS input `{path}`: {source}")]
    FcsRead {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error("failed to write FCS output `{path}`: {source}")]
    FcsWrite {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error("FCS input `{path}` has no events")]
    EmptyFcs { path: PathBuf },
    #[error("channel list must contain at least one channel")]
    EmptyChannelList,
    #[error("duplicate channel `{channel}`")]
    DuplicateChannel { channel: String },
    #[error(
        "channel `{channel}` was not found in `{path}`; available channels: {available_channels:?}"
    )]
    MissingChannel {
        path: PathBuf,
        channel: String,
        available_channels: Vec<String>,
    },
    #[error("no transformable channels were selected for `{path}`")]
    NoTransformableChannels { path: PathBuf },
    #[error("channel `{channel}` in `{path}` contains non-finite values")]
    NonFiniteChannel { path: PathBuf, channel: String },
    #[error("output FCS values for `{path}` contain non-finite values")]
    NonFiniteOutputValues { path: PathBuf },
    #[error(
        "multiple inputs require `--output` to be a directory, but `{path}` is an existing file"
    )]
    MultipleInputsOutputFile { path: PathBuf },
    #[error("multiple inputs would write the same output path `{path}`")]
    DuplicateOutput { path: PathBuf },
    #[error("output path `{path}` already exists; pass `--overwrite` to replace it")]
    OutputExists { path: PathBuf },
    #[error("output path `{path}` is not an FCS file")]
    NonFcsOutput { path: PathBuf },
    #[error("input and output both resolve to `{path}`; pass `--overwrite` to replace it")]
    SameInputOutput { path: PathBuf },
    #[error("failed to create output directory `{path}`: {source}")]
    OutputDirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create metadata parent directory `{path}`: {source}")]
    MetadataDirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("metadata output `{path}` already exists; pass `--overwrite` to replace it")]
    MetadataExists { path: PathBuf },
    #[error("failed to serialize metadata JSON for `{path}`: {source}")]
    MetadataSerialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write metadata JSON `{path}`: {source}")]
    MetadataWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create debug output directory `{path}`: {source}")]
    DebugDirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write CSV debug output `{path}`: {source}")]
    CsvWrite {
        path: PathBuf,
        #[source]
        source: csv::Error,
    },
    #[error("failed to flush CSV debug output `{path}`: {source}")]
    CsvFlush {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write JSON debug output `{path}`: {source}")]
    DebugJsonWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize JSON debug output `{path}`: {source}")]
    DebugJsonSerialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write HTML debug output `{path}`: {source}")]
    HtmlWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to replace `{path}` with transformed output `{temp_path}`: {source}")]
    OutputReplace {
        path: PathBuf,
        temp_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to build transform worker thread pool with {threads} threads: {source}")]
    ThreadPoolBuild {
        threads: usize,
        #[source]
        source: rayon::ThreadPoolBuildError,
    },
}
