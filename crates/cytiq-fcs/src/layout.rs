use crate::error::FcsError;
use crate::types::{FcsText, FcsVersion, Parameter, ParameterDataType, ParameterWidth};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ByteOrder {
    None,
    Ordered([usize; 8], usize),
    LittleEndian,
    BigEndian,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParameterEncoding {
    AsciiFixed { width: usize },
    Integer { width_bytes: usize },
    Float32,
    Float64,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedParameter {
    pub public: Parameter,
    pub encoding: ParameterEncoding,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedLayout {
    byte_order: ByteOrder,
    row_count: usize,
    parameters: Vec<ResolvedParameter>,
}

impl ResolvedLayout {
    #[must_use]
    pub const fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    #[must_use]
    pub fn parameters(&self) -> &[ResolvedParameter] {
        &self.parameters
    }

    #[must_use]
    pub fn public_parameters(&self) -> Vec<Parameter> {
        self.parameters
            .iter()
            .map(|parameter| parameter.public.clone())
            .collect()
    }
}

pub(crate) fn resolve_layout(
    version: FcsVersion,
    text: &FcsText,
) -> Result<ResolvedLayout, FcsError> {
    let mode = required_value(text, "$MODE")?;
    if mode != "L" {
        return Err(FcsError::UnsupportedMode(mode.to_owned()));
    }

    let row_count = parse_required_usize(text, "$TOT")?;
    let parameter_count = parse_required_usize(text, "$PAR")?;
    let dataset_type = parse_data_type(required_value(text, "$DATATYPE")?)?;
    let byte_order_raw = required_value(text, "$BYTEORD")?;

    let mut parameters = Vec::with_capacity(parameter_count);
    let mut max_numeric_width = 0_usize;
    let mut has_numeric = false;
    let mut has_delimited_ascii = false;
    let mut fcs30_numeric_width = None;

    for index in 1..=parameter_count {
        let index_u16 = u16::try_from(index)
            .map_err(|_| FcsError::UnsupportedLayout("parameter count exceeds u16"))?;
        let name = required_parameter_value(text, index, "N")?.to_owned();
        let label = optional_parameter_value(text, index, "S").map(str::to_owned);
        let width_value = required_parameter_value(text, index, "B")?;
        let _range_value = required_parameter_value(text, index, "R")?;
        let _scale_value = required_parameter_value(text, index, "E")?;

        let data_type = if version == FcsVersion::V3_2 {
            optional_parameter_value(text, index, "DATATYPE")
                .map(parse_data_type)
                .transpose()?
                .unwrap_or(dataset_type)
        } else {
            dataset_type
        };

        let keywords = collect_parameter_keywords(text, index_u16);
        let (width, encoding) = parse_width(index_u16, width_value, data_type)?;
        match encoding {
            ParameterEncoding::AsciiFixed { .. } => {}
            ParameterEncoding::Integer { width_bytes } => {
                has_numeric = true;
                max_numeric_width = max_numeric_width.max(width_bytes);
                if version == FcsVersion::V3_0 {
                    ensure_consistent_fcs30_width(&mut fcs30_numeric_width, width_bytes)?;
                }
            }
            ParameterEncoding::Float32 => {
                has_numeric = true;
                max_numeric_width = max_numeric_width.max(4);
                if version == FcsVersion::V3_0 {
                    ensure_consistent_fcs30_width(&mut fcs30_numeric_width, 4)?;
                }
            }
            ParameterEncoding::Float64 => {
                has_numeric = true;
                max_numeric_width = max_numeric_width.max(8);
                if version == FcsVersion::V3_0 {
                    ensure_consistent_fcs30_width(&mut fcs30_numeric_width, 8)?;
                }
            }
        }
        if width == ParameterWidth::DelimitedAscii {
            has_delimited_ascii = true;
        }

        parameters.push(ResolvedParameter {
            public: Parameter {
                index: index_u16,
                name,
                label,
                width,
                data_type,
                keywords,
            },
            encoding,
        });
    }

    if has_delimited_ascii {
        return Err(FcsError::UnsupportedLayout(
            "delimited ASCII DATA is not supported yet",
        ));
    }

    let byte_order = if has_numeric {
        parse_byte_order(version, byte_order_raw, max_numeric_width)?
    } else {
        ByteOrder::None
    };

    Ok(ResolvedLayout {
        byte_order,
        row_count,
        parameters,
    })
}

fn parse_data_type(value: &str) -> Result<ParameterDataType, FcsError> {
    match value {
        "A" => Ok(ParameterDataType::Ascii),
        "I" => Ok(ParameterDataType::Integer),
        "F" => Ok(ParameterDataType::Float),
        "D" => Ok(ParameterDataType::Double),
        _ => Err(FcsError::InvalidKeywordValue {
            key: "$DATATYPE".to_owned(),
            value: value.to_owned(),
        }),
    }
}

fn parse_width(
    parameter: u16,
    width_value: &str,
    data_type: ParameterDataType,
) -> Result<(ParameterWidth, ParameterEncoding), FcsError> {
    if width_value == "*" {
        return if data_type == ParameterDataType::Ascii {
            Ok((
                ParameterWidth::DelimitedAscii,
                ParameterEncoding::AsciiFixed { width: 0 },
            ))
        } else {
            Err(FcsError::UnsupportedWidth {
                parameter,
                value: width_value.to_owned(),
            })
        };
    }

    let width_number = width_value
        .parse::<u8>()
        .map_err(|_| FcsError::UnsupportedWidth {
            parameter,
            value: width_value.to_owned(),
        })?;

    match data_type {
        ParameterDataType::Ascii => {
            if (1..=20).contains(&width_number) {
                Ok((
                    ParameterWidth::AsciiChars(width_number),
                    ParameterEncoding::AsciiFixed {
                        width: usize::from(width_number),
                    },
                ))
            } else {
                Err(FcsError::UnsupportedWidth {
                    parameter,
                    value: width_value.to_owned(),
                })
            }
        }
        ParameterDataType::Integer => {
            if width_number % 8 == 0 && (8..=64).contains(&width_number) {
                Ok((
                    ParameterWidth::Bits(width_number),
                    ParameterEncoding::Integer {
                        width_bytes: usize::from(width_number / 8),
                    },
                ))
            } else {
                Err(FcsError::UnsupportedWidth {
                    parameter,
                    value: width_value.to_owned(),
                })
            }
        }
        ParameterDataType::Float => {
            if width_number == 32 {
                Ok((
                    ParameterWidth::Bits(width_number),
                    ParameterEncoding::Float32,
                ))
            } else {
                Err(FcsError::UnsupportedWidth {
                    parameter,
                    value: width_value.to_owned(),
                })
            }
        }
        ParameterDataType::Double => {
            if width_number == 64 {
                Ok((
                    ParameterWidth::Bits(width_number),
                    ParameterEncoding::Float64,
                ))
            } else {
                Err(FcsError::UnsupportedWidth {
                    parameter,
                    value: width_value.to_owned(),
                })
            }
        }
    }
}

fn parse_byte_order(
    version: FcsVersion,
    raw_value: &str,
    numeric_width: usize,
) -> Result<ByteOrder, FcsError> {
    match version {
        FcsVersion::V3_0 => parse_ordered_byte_order(raw_value, numeric_width),
        FcsVersion::V3_1 | FcsVersion::V3_2 => match raw_value {
            "1,2,3,4" => Ok(ByteOrder::LittleEndian),
            "4,3,2,1" => Ok(ByteOrder::BigEndian),
            _ => Err(FcsError::UnsupportedByteOrder(raw_value.to_owned())),
        },
    }
}

fn required_value<'a>(text: &'a FcsText, key: &str) -> Result<&'a str, FcsError> {
    text.get(key)
        .ok_or_else(|| FcsError::MissingKeyword(key.to_owned()))
}

fn parse_required_usize(text: &FcsText, key: &str) -> Result<usize, FcsError> {
    let value = required_value(text, key)?;
    value
        .parse::<usize>()
        .map_err(|_| FcsError::InvalidKeywordValue {
            key: key.to_owned(),
            value: value.to_owned(),
        })
}

fn required_parameter_value<'a>(
    text: &'a FcsText,
    index: usize,
    suffix: &str,
) -> Result<&'a str, FcsError> {
    let key = format!("$P{index}{suffix}");
    text.get(&key).ok_or(FcsError::MissingKeyword(key))
}

fn optional_parameter_value<'a>(text: &'a FcsText, index: usize, suffix: &str) -> Option<&'a str> {
    let key = format!("$P{index}{suffix}");
    text.get(&key)
}

fn collect_parameter_keywords(text: &FcsText, index: u16) -> Vec<(String, String)> {
    let prefix = format!("$P{index}");
    text.iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn ensure_consistent_fcs30_width(
    expected_width: &mut Option<usize>,
    width_bytes: usize,
) -> Result<(), FcsError> {
    match expected_width {
        Some(existing) if *existing != width_bytes => Err(FcsError::UnsupportedLayout(
            "FCS 3.0 numeric parameters must share a single width",
        )),
        Some(_) => Ok(()),
        None => {
            *expected_width = Some(width_bytes);
            Ok(())
        }
    }
}

fn parse_ordered_byte_order(raw_value: &str, numeric_width: usize) -> Result<ByteOrder, FcsError> {
    if !(1..=8).contains(&numeric_width) {
        return Err(FcsError::UnsupportedByteOrder(raw_value.to_owned()));
    }

    let components = raw_value
        .split(',')
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FcsError::UnsupportedByteOrder(raw_value.to_owned()))?;
    if components.len() != numeric_width {
        return Err(FcsError::UnsupportedByteOrder(raw_value.to_owned()));
    }

    let mut order = [0_usize; 8];
    let mut seen = [false; 8];
    for (index, component) in components.into_iter().enumerate() {
        if component == 0 || component > numeric_width {
            return Err(FcsError::UnsupportedByteOrder(raw_value.to_owned()));
        }

        let position = component - 1;
        if seen[position] {
            return Err(FcsError::UnsupportedByteOrder(raw_value.to_owned()));
        }

        seen[position] = true;
        order[index] = position;
    }

    Ok(ByteOrder::Ordered(order, numeric_width))
}
