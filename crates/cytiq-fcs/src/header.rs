use std::io::Read;

use crate::error::FcsError;
use crate::types::{ByteRange, FcsVersion, Header};

const HEADER_LEN: usize = 58;

pub(crate) fn read_header<R: Read>(reader: &mut R) -> Result<Header, FcsError> {
    let mut bytes = [0_u8; HEADER_LEN];
    reader.read_exact(&mut bytes).map_err(FcsError::Io)?;

    let version = parse_version(&bytes[0..6])?;
    if bytes[6..10] != [b' '; 4] {
        return Err(FcsError::InvalidHeader(
            "version must be followed by four spaces",
        ));
    }

    let text = parse_required_range(&bytes[10..18], &bytes[18..26], "TEXT")?;
    let data = parse_optional_range(&bytes[26..34], &bytes[34..42], "DATA")?;
    let analysis = parse_optional_range(&bytes[42..50], &bytes[50..58], "ANALYSIS")?;

    Ok(Header {
        version,
        text,
        data,
        analysis,
    })
}

fn parse_version(bytes: &[u8]) -> Result<FcsVersion, FcsError> {
    let version = std::str::from_utf8(bytes).map_err(|_| FcsError::InvalidUtf8 {
        section: "HEADER version",
    })?;
    match version {
        "FCS3.0" => Ok(FcsVersion::V3_0),
        "FCS3.1" => Ok(FcsVersion::V3_1),
        "FCS3.2" => Ok(FcsVersion::V3_2),
        other => Err(FcsError::UnsupportedVersion(other.to_owned())),
    }
}

fn parse_required_range(
    begin: &[u8],
    end: &[u8],
    name: &'static str,
) -> Result<ByteRange, FcsError> {
    let begin_value = parse_offset(begin, name)?;
    let end_value = parse_offset(end, name)?;
    if begin_value > end_value {
        return Err(FcsError::InvalidHeader(
            "segment start is after segment end",
        ));
    }
    Ok(ByteRange::new(begin_value, end_value))
}

fn parse_optional_range(
    begin: &[u8],
    end: &[u8],
    name: &'static str,
) -> Result<Option<ByteRange>, FcsError> {
    let begin_str = parse_offset_string(begin, name)?;
    let end_str = parse_offset_string(end, name)?;
    if begin_str.is_empty() && end_str.is_empty() {
        return Ok(None);
    }

    let begin_value = begin_str
        .parse::<u64>()
        .map_err(|_| FcsError::InvalidHeader("invalid offset"))?;
    let end_value = end_str
        .parse::<u64>()
        .map_err(|_| FcsError::InvalidHeader("invalid offset"))?;

    if begin_value == 0 && end_value == 0 {
        Ok(None)
    } else if begin_value <= end_value {
        Ok(Some(ByteRange::new(begin_value, end_value)))
    } else {
        Err(FcsError::InvalidHeader(
            "segment start is after segment end",
        ))
    }
}

fn parse_offset(bytes: &[u8], name: &'static str) -> Result<u64, FcsError> {
    let text = parse_offset_string(bytes, name)?;
    text.parse::<u64>()
        .map_err(|_| FcsError::InvalidKeywordValue {
            key: name.to_owned(),
            value: text.to_owned(),
        })
}

fn parse_offset_string<'a>(bytes: &'a [u8], name: &'static str) -> Result<&'a str, FcsError> {
    std::str::from_utf8(bytes)
        .map_err(|_| FcsError::InvalidUtf8 { section: name })
        .map(str::trim)
}
