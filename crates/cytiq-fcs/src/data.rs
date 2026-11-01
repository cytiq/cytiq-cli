use std::io::{Read, Seek, SeekFrom};

use crate::error::FcsError;
use crate::layout::{ByteOrder, ParameterEncoding, ResolvedLayout, ResolvedParameter};
use crate::types::{ByteRange, ColumnData, EventColumns};

pub(crate) fn read_raw_segment<R: Read + Seek>(
    reader: &mut R,
    range: ByteRange,
) -> Result<Vec<u8>, FcsError> {
    reader
        .seek(SeekFrom::Start(range.start))
        .map_err(FcsError::Io)?;
    let length =
        usize::try_from(range.len()).map_err(|_| FcsError::InvalidText("segment too large"))?;
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes).map_err(FcsError::Io)?;
    Ok(bytes)
}

pub(crate) fn decode_data<R: Read + Seek>(
    reader: &mut R,
    range: ByteRange,
    layout: &ResolvedLayout,
) -> Result<EventColumns, FcsError> {
    // Check the declared layout against the segment before reading it, so a
    // lying `$TOT` or `$PnB` never sizes a read.
    let expected_bytes = expected_data_length(layout)?;
    let actual_bytes = range.len();
    if expected_bytes != actual_bytes {
        return Err(FcsError::DataLengthMismatch {
            expected: expected_bytes,
            actual: actual_bytes,
        });
    }
    let buffer = read_raw_segment(reader, range)?;

    let mut columns = layout
        .parameters()
        .iter()
        .map(new_column)
        .collect::<Vec<_>>();
    let mut offset = 0_usize;

    for _ in 0..layout.row_count() {
        for (column, parameter) in columns.iter_mut().zip(layout.parameters()) {
            match parameter.encoding {
                ParameterEncoding::AsciiFixed { width } => {
                    let end = offset + width;
                    let raw = &buffer[offset..end];
                    let value = parse_ascii_value(raw, parameter.public.index)?;
                    if let ColumnData::UInt(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Integer { width_bytes } => {
                    let end = offset + width_bytes;
                    let raw = &buffer[offset..end];
                    let value = decode_integer(raw, layout.byte_order())?;
                    if let ColumnData::UInt(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Float32 => {
                    let end = offset + 4;
                    let raw = &buffer[offset..end];
                    let value = decode_f32(raw, layout.byte_order())?;
                    if let ColumnData::F32(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Float64 => {
                    let end = offset + 8;
                    let raw = &buffer[offset..end];
                    let value = decode_f64(raw, layout.byte_order())?;
                    if let ColumnData::F64(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
            }
        }
    }

    Ok(EventColumns { columns })
}

fn expected_data_length(layout: &ResolvedLayout) -> Result<u64, FcsError> {
    let row_width =
        layout
            .parameters()
            .iter()
            .map(parameter_width)
            .try_fold(0_u64, |accumulator, width| {
                width.and_then(|value| {
                    accumulator
                        .checked_add(value)
                        .ok_or(FcsError::UnsupportedLayout("row width overflowed u64"))
                })
            })?;
    let rows = u64::try_from(layout.row_count())
        .map_err(|_| FcsError::UnsupportedLayout("row count overflowed u64"))?;
    row_width
        .checked_mul(rows)
        .ok_or(FcsError::UnsupportedLayout("DATA length overflowed u64"))
}

fn parameter_width(parameter: &ResolvedParameter) -> Result<u64, FcsError> {
    match parameter.encoding {
        ParameterEncoding::AsciiFixed { width } => u64::try_from(width)
            .map_err(|_| FcsError::UnsupportedLayout("ASCII width overflowed u64")),
        ParameterEncoding::Integer { width_bytes } => u64::try_from(width_bytes)
            .map_err(|_| FcsError::UnsupportedLayout("integer width overflowed u64")),
        ParameterEncoding::Float32 => Ok(4),
        ParameterEncoding::Float64 => Ok(8),
    }
}

fn new_column(parameter: &ResolvedParameter) -> ColumnData {
    match parameter.encoding {
        ParameterEncoding::AsciiFixed { .. } | ParameterEncoding::Integer { .. } => {
            ColumnData::UInt(Vec::new())
        }
        ParameterEncoding::Float32 => ColumnData::F32(Vec::new()),
        ParameterEncoding::Float64 => ColumnData::F64(Vec::new()),
    }
}

fn parse_ascii_value(raw: &[u8], parameter: u16) -> Result<u64, FcsError> {
    let text = std::str::from_utf8(raw).map_err(|_| FcsError::InvalidUtf8 {
        section: "ASCII DATA",
    })?;
    text.trim()
        .parse::<u64>()
        .map_err(|_| FcsError::EventValueParse {
            parameter,
            data_type: "ASCII integer",
            value: text.to_owned(),
        })
}

fn decode_integer(bytes: &[u8], byte_order: ByteOrder) -> Result<u64, FcsError> {
    let value = match byte_order {
        ByteOrder::None => {
            return Err(FcsError::UnsupportedLayout("numeric DATA requires BYTEORD"));
        }
        ByteOrder::Ordered(order, width) => decode_ordered(bytes, &order, width),
        ByteOrder::LittleEndian => decode_little_endian(bytes),
        ByteOrder::BigEndian => decode_big_endian(bytes),
    };
    Ok(value)
}

fn decode_f32(bytes: &[u8], byte_order: ByteOrder) -> Result<f32, FcsError> {
    let array = match byte_order {
        ByteOrder::Ordered(order, 4) => reorder_four(bytes, order),
        ByteOrder::LittleEndian => copy_four(bytes),
        ByteOrder::BigEndian => {
            let array = copy_four(bytes);
            [array[3], array[2], array[1], array[0]]
        }
        _ => {
            return Err(FcsError::UnsupportedLayout(
                "F32 data requires 4-byte BYTEORD",
            ));
        }
    };
    Ok(f32::from_le_bytes(array))
}

fn decode_f64(bytes: &[u8], byte_order: ByteOrder) -> Result<f64, FcsError> {
    let array = match byte_order {
        ByteOrder::Ordered(order, 8) => reorder_eight(bytes, order),
        ByteOrder::LittleEndian => copy_eight(bytes),
        ByteOrder::BigEndian => {
            let array = copy_eight(bytes);
            [
                array[7], array[6], array[5], array[4], array[3], array[2], array[1], array[0],
            ]
        }
        _ => {
            return Err(FcsError::UnsupportedLayout(
                "F64 data requires 8-byte BYTEORD",
            ));
        }
    };
    Ok(f64::from_le_bytes(array))
}

fn decode_ordered(bytes: &[u8], order: &[usize; 8], width: usize) -> u64 {
    order
        .iter()
        .take(width)
        .enumerate()
        .fold(0_u64, |accumulator, (index, position)| {
            let shift = index * 8;
            accumulator | (u64::from(bytes[*position]) << shift)
        })
}

fn decode_little_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .enumerate()
        .fold(0_u64, |accumulator, (index, byte)| {
            accumulator | (u64::from(*byte) << (index * 8))
        })
}

fn decode_big_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .enumerate()
        .fold(0_u64, |accumulator, (index, byte)| {
            accumulator | (u64::from(*byte) << (index * 8))
        })
}

fn copy_four(bytes: &[u8]) -> [u8; 4] {
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn copy_eight(bytes: &[u8]) -> [u8; 8] {
    [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]
}

fn reorder_four(bytes: &[u8], order: [usize; 8]) -> [u8; 4] {
    [
        bytes[order[0]],
        bytes[order[1]],
        bytes[order[2]],
        bytes[order[3]],
    ]
}

fn reorder_eight(bytes: &[u8], order: [usize; 8]) -> [u8; 8] {
    [
        bytes[order[0]],
        bytes[order[1]],
        bytes[order[2]],
        bytes[order[3]],
        bytes[order[4]],
        bytes[order[5]],
        bytes[order[6]],
        bytes[order[7]],
    ]
}
