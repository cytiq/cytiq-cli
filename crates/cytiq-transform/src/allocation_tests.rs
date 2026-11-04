// Frozen pre-optimisation implementation from main 1485c1d. Keep independent
// of the new buffer/iterator implementation so rounding and reductions are checked.
use super::*;

fn assert_same_column(actual: &TransformedColumn, expected: &TransformedColumn) {
    assert_eq!(
        actual
            .values
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        expected
            .values
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_string(&actual.metrics).unwrap(),
        serde_json::to_string(&expected.metrics).unwrap()
    );
}

#[test]
fn matches_baseline_bits_on_100_seeded_columns_and_both_thread_counts() {
    let pools = [1, 4].map(|threads| {
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
    });
    for seed in 0..100_u64 {
        let len = match seed {
            0 => 0,
            1 => 1,
            2 => 8191,
            3 => 8192,
            4 => 8193,
            5 => 16_385,
            _ => (seed as usize * 37) % 257 + 2,
        };
        let mut state = seed + 1;
        let mut values = (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                (state >> 32) as f64 / 8192.0 - 1000.0
            })
            .collect::<Vec<_>>();
        for (slot, edge) in values.iter_mut().zip([
            0.0,
            -0.0,
            f64::EPSILON,
            -f64::EPSILON,
            16_777_217.0,
            -123.456_789_123,
        ]) {
            *slot = edge;
        }
        let columns = [
            ColumnData::F64(values.clone()),
            ColumnData::F32(values.iter().map(|x| *x as f32).collect()),
            ColumnData::UInt(values.iter().map(|x| x.abs() as u64 + 16_777_217).collect()),
        ];
        for method in [
            TransformMethod::Arcsinh,
            TransformMethod::Logicle,
            TransformMethod::Hyperlog,
            TransformMethod::Biexponential,
            TransformMethod::None,
        ] {
            let mut config = TransformConfig::new(Vec::new(), "output.fcs".into());
            config.method = method;
            let operator = TransformOperator::from_config(&config).unwrap();
            for column in &columns {
                for selection in [Some("FL1-A"), None] {
                    let expected = baseline_transform_column(
                        column,
                        selection,
                        &operator,
                        Path::new("input.fcs"),
                        "FL1-A",
                    )
                    .unwrap();
                    for pool in &pools {
                        let actual = pool
                            .install(|| {
                                super::transform_column(
                                    column,
                                    selection,
                                    &operator,
                                    Path::new("input.fcs"),
                                    "FL1-A",
                                )
                            })
                            .unwrap();
                        assert_same_column(&actual, &expected);
                    }
                }
            }
        }
    }
}

#[test]
fn keeps_first_error_in_event_order_across_chunks() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let operator = TransformOperator::Arcsinh {
        cofactor: f64::MIN_POSITIVE,
    };
    for selected in [Some("FL1-A"), None] {
        for first in [f64::NAN, f64::INFINITY, 1.0e308] {
            let mut values = vec![0.0; 16_385];
            values[1] = first;
            values[8193] = f64::NAN;
            let column = ColumnData::F64(values);
            let expected = baseline_transform_column(
                &column,
                selected,
                &operator,
                Path::new("input.fcs"),
                "FL1-A",
            )
            .unwrap_err()
            .to_string();
            for _ in 0..20 {
                let actual = pool
                    .install(|| {
                        super::transform_column(
                            &column,
                            selected,
                            &operator,
                            Path::new("input.fcs"),
                            "FL1-A",
                        )
                    })
                    .unwrap_err()
                    .to_string();
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn non_finite_input_publishes_neither_fcs_nor_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.fcs");
    let output = directory.path().join("output.fcs");
    let metadata = directory.path().join("metrics.json");
    cytiq_fcs::write_fcs3_1_f64_with_options(
        &input,
        &[FcsWriteParameter::new("FL1-A")],
        &[vec![1.0, 2.0]],
        &FcsWriteOptions::new(),
    )
    .unwrap();
    let dataset = FcsReader::open(&input).unwrap().read().unwrap();
    let start = dataset.header.data.unwrap().start as usize;
    let mut bytes = fs::read(&input).unwrap();
    bytes[start..start + 8].copy_from_slice(&f64::NAN.to_le_bytes());
    fs::write(&input, bytes).unwrap();
    let mut config = TransformConfig::new(vec![input], output.clone());
    config.metadata_json = Some(metadata.clone());
    assert!(matches!(
        TransformRunner::run(&config),
        Err(TransformError::NonFiniteChannel { .. })
    ));
    assert!(!output.exists());
    assert!(!metadata.exists());
}

fn baseline_transform_column(
    column: &ColumnData,
    selected_channel: Option<&str>,
    operator: &TransformOperator,
    input: &Path,
    channel: &str,
) -> Result<TransformedColumn, TransformError> {
    let values = match column {
        ColumnData::UInt(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(
                    *value as f64,
                    selected_channel.is_some(),
                    operator,
                    input,
                    channel,
                )
            })
            .collect::<Vec<_>>(),
        ColumnData::F32(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(
                    f64::from(*value),
                    selected_channel.is_some(),
                    operator,
                    input,
                    channel,
                )
            })
            .collect::<Vec<_>>(),
        ColumnData::F64(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(*value, selected_channel.is_some(), operator, input, channel)
            })
            .collect::<Vec<_>>(),
    };

    let mut output = Vec::with_capacity(values.len());
    let mut raw_values = Vec::new();
    let mut transformed_values = Vec::new();
    for value in values {
        let value = value?;
        if selected_channel.is_some() {
            raw_values.push(f64::from(value.raw));
            transformed_values.push(value.transformed);
        }
        output.push(value.transformed);
    }

    let metrics = selected_channel
        .map(|channel| build_channel_metrics(channel, &raw_values, &transformed_values));

    Ok(TransformedColumn {
        values: output,
        metrics,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TransformedValue {
    raw: f32,
    /// Transformed values are rounded to f32; values of unselected channels
    /// (such as `Original_ID`) are kept exact.
    transformed: f64,
}

fn transform_raw_value(
    value: f64,
    selected: bool,
    operator: &TransformOperator,
    input: &Path,
    channel: &str,
) -> Result<TransformedValue, TransformError> {
    if selected && !value.is_finite() {
        return Err(TransformError::NonFiniteChannel {
            path: input.to_path_buf(),
            channel: channel.to_owned(),
        });
    }
    let output = if selected {
        operator.apply(value)
    } else {
        value
    };
    if !output.is_finite() {
        return Err(TransformError::NonFiniteOutputValues {
            path: input.to_path_buf(),
        });
    }

    Ok(TransformedValue {
        raw: value as f32,
        transformed: if selected {
            f64::from(output as f32)
        } else {
            output
        },
    })
}

fn build_channel_metrics(
    channel: &str,
    raw_values: &[f64],
    transformed_values: &[f64],
) -> TransformChannelMetrics {
    TransformChannelMetrics {
        channel: channel.to_owned(),
        events: raw_values.len(),
        raw: summarize_values(raw_values),
        transformed: summarize_values(transformed_values),
        raw_negative_count: raw_values.iter().filter(|value| **value < 0.0).count(),
        raw_zero_count: raw_values
            .iter()
            .filter(|value| value.abs() <= f64::EPSILON)
            .count(),
    }
}

fn summarize_values(values: &[f64]) -> TransformValueMetrics {
    if values.is_empty() {
        return TransformValueMetrics {
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            standard_deviation: 0.0,
        };
    }

    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let standard_deviation = if values.len() < 2 {
        0.0
    } else {
        let variance = values
            .iter()
            .map(|value| {
                let delta = value - mean;
                delta * delta
            })
            .sum::<f64>()
            / (values.len() - 1) as f64;
        variance.sqrt()
    };

    TransformValueMetrics {
        min,
        max,
        mean,
        standard_deviation,
    }
}
