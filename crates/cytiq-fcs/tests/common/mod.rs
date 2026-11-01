//! Builders for synthetic FCS files shared by the integration tests.

#![allow(dead_code)]

pub const DELIMITER: u8 = b'|';

pub fn build_fcs(
    version: &str,
    primary_pairs: Vec<(&str, &str)>,
    supplemental_pairs: Option<Vec<(&str, &str)>>,
    data: Vec<u8>,
    analysis: Option<Vec<u8>>,
) -> Vec<u8> {
    build_fcs_overriding(
        version,
        primary_pairs,
        supplemental_pairs,
        data,
        analysis,
        &[],
    )
}

/// Like `build_fcs`, but `overrides` replace primary TEXT values after the
/// segment offsets are filled in, to craft files whose keywords lie. HEADER
/// offsets still describe the real layout.
pub fn build_fcs_overriding(
    version: &str,
    primary_pairs: Vec<(&str, &str)>,
    supplemental_pairs: Option<Vec<(&str, &str)>>,
    data: Vec<u8>,
    analysis: Option<Vec<u8>>,
    overrides: &[(&str, &str)],
) -> Vec<u8> {
    let mut primary = primary_pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    set_pair(&mut primary, "$BEGINSTEXT", "00000000");
    set_pair(&mut primary, "$ENDSTEXT", "00000000");
    set_pair(&mut primary, "$BEGINDATA", "00000000");
    set_pair(&mut primary, "$ENDDATA", "00000000");
    set_pair(&mut primary, "$BEGINANALYSIS", "00000000");
    set_pair(&mut primary, "$ENDANALYSIS", "00000000");
    for (key, value) in overrides {
        set_pair(&mut primary, key, value);
    }

    let placeholder_primary = encode_text(&primary);
    let text_start = 58_u64;
    let text_end =
        text_start + u64::try_from(placeholder_primary.len()).expect("text length fits") - 1;

    let supplemental = supplemental_pairs.map(|pairs| {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect::<Vec<_>>()
    });
    let supplemental_bytes = supplemental.as_ref().map(|pairs| encode_text(pairs));
    let supplemental_range = supplemental_bytes.as_ref().map(|bytes| {
        let start = text_end + 1;
        let end = start + u64::try_from(bytes.len()).expect("supplemental TEXT length fits") - 1;
        (start, end)
    });

    let data_start = supplemental_range.map_or(text_end + 1, |(_, end)| end + 1);
    let data_end = data_start + u64::try_from(data.len()).expect("data length fits") - 1;
    let analysis_range = analysis.as_ref().map(|bytes| {
        let start = data_end + 1;
        let end = start + u64::try_from(bytes.len()).expect("analysis length fits") - 1;
        (start, end)
    });

    if let Some((start, end)) = supplemental_range {
        set_pair(&mut primary, "$BEGINSTEXT", &format!("{start:08}"));
        set_pair(&mut primary, "$ENDSTEXT", &format!("{end:08}"));
    }
    set_pair(&mut primary, "$BEGINDATA", &format!("{data_start:08}"));
    set_pair(&mut primary, "$ENDDATA", &format!("{data_end:08}"));
    if let Some((start, end)) = analysis_range {
        set_pair(&mut primary, "$BEGINANALYSIS", &format!("{start:08}"));
        set_pair(&mut primary, "$ENDANALYSIS", &format!("{end:08}"));
    }
    for (key, value) in overrides {
        set_pair(&mut primary, key, value);
    }

    let primary_bytes = encode_text(&primary);
    let header = format_header(
        version,
        text_start,
        text_start + u64::try_from(primary_bytes.len()).expect("text length fits") - 1,
        data_start,
        data_end,
        analysis_range.map_or(0, |(start, _)| start),
        analysis_range.map_or(0, |(_, end)| end),
    );

    let mut output = header.into_bytes();
    output.extend(primary_bytes);
    if let Some(bytes) = supplemental_bytes {
        output.extend(bytes);
    }
    output.extend(data);
    if let Some(bytes) = analysis {
        output.extend(bytes);
    }

    output
}

pub fn format_header(
    version: &str,
    text_start: u64,
    text_end: u64,
    data_start: u64,
    data_end: u64,
    analysis_start: u64,
    analysis_end: u64,
) -> String {
    format!(
        "{version}    {text_start:>8}{text_end:>8}{data_start:>8}{data_end:>8}{analysis_start:>8}{analysis_end:>8}"
    )
}

pub fn encode_text(pairs: &[(String, String)]) -> Vec<u8> {
    let mut bytes = vec![DELIMITER];
    for (key, value) in pairs {
        push_escaped(&mut bytes, key);
        bytes.push(DELIMITER);
        push_escaped(&mut bytes, value);
        bytes.push(DELIMITER);
    }
    bytes
}

pub fn push_escaped(bytes: &mut Vec<u8>, value: &str) {
    for byte in value.as_bytes() {
        bytes.push(*byte);
        if *byte == DELIMITER {
            bytes.push(DELIMITER);
        }
    }
}

pub fn set_pair(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = pairs.iter_mut().find(|(current_key, _)| current_key == key) {
        value.clone_into(existing);
    } else {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}

pub fn encode_u16_rows(rows: &[[u16; 1]], little_endian: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for [value] in rows {
        let raw = if little_endian {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes.extend(raw);
    }
    bytes
}

pub fn overwrite_header_range(
    bytes: &mut [u8],
    start_offset: usize,
    end_offset: usize,
    start: u64,
    end: u64,
) {
    let begin = format!("{start:>8}");
    let finish = format!("{end:>8}");
    bytes[start_offset..start_offset + 8].copy_from_slice(begin.as_bytes());
    bytes[end_offset..end_offset + 8].copy_from_slice(finish.as_bytes());
}
