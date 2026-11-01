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

/// Rows are read from DATA this many bytes at a time, so decoding holds the
/// columns and one chunk instead of the whole segment as well. A chunk holds
/// at least one row, which is at most 65,535 × 20 ASCII bytes (~1.25 MiB).
const DATA_CHUNK_BYTES: usize = 1 << 20;

/// Check DATA against `$TOT` × row width before anything reads it, so a lying
/// `$TOT` or `$PnB` never sizes a read.
pub(crate) fn ensure_data_length(
    range: ByteRange,
    layout: &ResolvedLayout,
) -> Result<(), FcsError> {
    let expected_bytes = expected_data_length(layout)?;
    let actual_bytes = range.len();
    if expected_bytes == actual_bytes {
        Ok(())
    } else {
        Err(FcsError::DataLengthMismatch {
            expected: expected_bytes,
            actual: actual_bytes,
        })
    }
}

/// Bytes the decoded columns take: `$TOT` × the decoded width of a row.
pub(crate) fn decoded_data_length(layout: &ResolvedLayout) -> u64 {
    let row_width = layout
        .parameters()
        .iter()
        .map(|parameter| parameter.encoding.decoded_bytes() as u64)
        .sum::<u64>();
    row_width.saturating_mul(layout.row_count() as u64)
}

/// Decode DATA, whose length [`ensure_data_length`] has checked.
pub(crate) fn decode_data<R: Read + Seek>(
    reader: &mut R,
    range: ByteRange,
    layout: &ResolvedLayout,
) -> Result<EventColumns, FcsError> {
    reader
        .seek(SeekFrom::Start(range.start))
        .map_err(FcsError::Io)?;
    let rows = layout.row_count();
    let mut columns = layout
        .parameters()
        .iter()
        .map(|parameter| new_column(parameter, rows))
        .collect::<Vec<_>>();
    read_rows(reader, layout, rows, &mut columns)?;
    Ok(EventColumns { columns })
}

/// Read and decode the next `rows` rows, appending them to `columns`.
pub(crate) fn read_rows<R: Read>(
    reader: &mut R,
    layout: &ResolvedLayout,
    rows: usize,
    columns: &mut [ColumnData],
) -> Result<(), FcsError> {
    let row_width = layout
        .parameters()
        .iter()
        .map(|parameter| parameter.encoding.stored_bytes())
        .sum::<usize>()
        .max(1);
    let rows_per_chunk = (DATA_CHUNK_BYTES / row_width).max(1);
    let mut buffer = Vec::new();
    let mut remaining = rows;
    while remaining > 0 {
        let chunk_rows = remaining.min(rows_per_chunk);
        buffer.resize(chunk_rows * row_width, 0);
        reader.read_exact(&mut buffer).map_err(FcsError::Io)?;
        decode_rows(&buffer, row_width, layout, columns)?;
        remaining -= chunk_rows;
    }
    Ok(())
}

fn decode_rows(
    bytes: &[u8],
    row_width: usize,
    layout: &ResolvedLayout,
    columns: &mut [ColumnData],
) -> Result<(), FcsError> {
    for row in bytes.chunks_exact(row_width) {
        let mut offset = 0_usize;
        for (column, parameter) in columns.iter_mut().zip(layout.parameters()) {
            match parameter.encoding {
                ParameterEncoding::AsciiFixed { width } => {
                    let end = offset + width;
                    let raw = &row[offset..end];
                    let value = parse_ascii_value(raw, parameter.public.index)?;
                    if let ColumnData::UInt(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Integer { width_bytes } => {
                    let end = offset + width_bytes;
                    let raw = &row[offset..end];
                    let value = decode_integer(raw, layout.byte_order())?;
                    if let ColumnData::UInt(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Float32 => {
                    let end = offset + 4;
                    let raw = &row[offset..end];
                    let value = decode_f32(raw, layout.byte_order())?;
                    if let ColumnData::F32(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
                ParameterEncoding::Float64 => {
                    let end = offset + 8;
                    let raw = &row[offset..end];
                    let value = decode_f64(raw, layout.byte_order())?;
                    if let ColumnData::F64(values) = column {
                        values.push(value);
                    }
                    offset = end;
                }
            }
        }
    }
    Ok(())
}

fn expected_data_length(layout: &ResolvedLayout) -> Result<u64, FcsError> {
    let row_width = layout
        .parameters()
        .iter()
        .map(|parameter| u64::try_from(parameter.encoding.stored_bytes()))
        .try_fold(0_u64, |accumulator, width| {
            width
                .ok()
                .and_then(|value| accumulator.checked_add(value))
                .ok_or(FcsError::UnsupportedLayout("row width overflowed u64"))
        })?;
    let rows = u64::try_from(layout.row_count())
        .map_err(|_| FcsError::UnsupportedLayout("row count overflowed u64"))?;
    row_width
        .checked_mul(rows)
        .ok_or(FcsError::UnsupportedLayout("DATA length overflowed u64"))
}

/// A column with room for `rows` values; their decoded size is already bounded.
pub(crate) fn new_column(parameter: &ResolvedParameter, rows: usize) -> ColumnData {
    match parameter.encoding {
        ParameterEncoding::AsciiFixed { .. } | ParameterEncoding::Integer { .. } => {
            ColumnData::UInt(Vec::with_capacity(rows))
        }
        ParameterEncoding::Float32 => ColumnData::F32(Vec::with_capacity(rows)),
        ParameterEncoding::Float64 => ColumnData::F64(Vec::with_capacity(rows)),
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
