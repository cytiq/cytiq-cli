use thiserror::Error;

#[derive(Debug, Error)]
pub enum FcsError {
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    #[error("unsupported FCS version `{0}`")]
    UnsupportedVersion(String),
    #[error("invalid HEADER: {0}")]
    InvalidHeader(&'static str),
    #[error("invalid UTF-8 in {section}")]
    InvalidUtf8 { section: &'static str },
    #[error("invalid TEXT segment: {0}")]
    InvalidText(&'static str),
    #[error("missing required keyword `{0}`")]
    MissingKeyword(String),
    #[error("duplicate keyword `{0}`")]
    DuplicateKeyword(String),
    #[error("invalid keyword `{key}` value `{value}`")]
    InvalidKeywordValue { key: String, value: String },
    #[error("unsupported MODE `{0}`")]
    UnsupportedMode(String),
    #[error("unsupported BYTEORD `{0}`")]
    UnsupportedByteOrder(String),
    #[error("unsupported width `{value}` for parameter {parameter}")]
    UnsupportedWidth { parameter: u16, value: String },
    #[error("unsupported layout: {0}")]
    UnsupportedLayout(&'static str),
    #[error(
        "{name} offsets do not match between HEADER ({header_start}-{header_end}) and TEXT ({text_start}-{text_end})"
    )]
    OffsetMismatch {
        name: &'static str,
        header_start: u64,
        header_end: u64,
        text_start: u64,
        text_end: u64,
    },
    #[error("{name} segment {start}-{end} is outside file length {file_len}")]
    SegmentOutOfBounds {
        name: &'static str,
        start: u64,
        end: u64,
        file_len: u64,
    },
    #[error("{first} segment overlaps {second}")]
    SegmentOverlap {
        first: &'static str,
        second: &'static str,
    },
    #[error("DATA length mismatch: expected {expected} bytes, found {actual}")]
    DataLengthMismatch { expected: u64, actual: u64 },
    #[error("failed to parse {data_type} value `{value}` for parameter {parameter}")]
    EventValueParse {
        parameter: u16,
        data_type: &'static str,
        value: String,
    },
    #[error("invalid FCS writer input: {0}")]
    InvalidWriterInput(&'static str),
}
