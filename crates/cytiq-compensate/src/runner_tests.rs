#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
// Frozen pre-optimisation implementation from main 1485c1d.
use super::*;

fn stored_values(dataset: &FcsDataset, index: usize) -> Vec<f64> {
    dataset.data.columns[index].to_f64_vec()
}

#[test]
fn matches_baseline_values_and_metadata_on_100_seeded_datasets() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/gating/sample.fcs");
    let original = FcsReader::open(path).unwrap().read().unwrap();
    for seed in 0..100_u64 {
        let mut dataset = original.clone();
        let mut state = seed + 1;
        for (index, column) in dataset.data.columns.iter_mut().enumerate() {
            let values = (0..127)
                .map(|event| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    if event == 0 {
                        -0.0
                    } else if event == 1 {
                        0.0
                    } else {
                        (state >> 32) as f64 / 8192.0 + index as f64
                    }
                })
                .collect::<Vec<_>>();
            *column = match (seed as usize + index) % 3 {
                0 => ColumnData::F64(values),
                1 => ColumnData::F32(values.iter().map(|value| *value as f32).collect()),
                _ => ColumnData::UInt(
                    values
                        .iter()
                        .map(|value| *value as u64 + 16_777_217)
                        .collect(),
                ),
            };
        }
        let mut indices =
            ["FL1-A", "FL2-A", "FL3-A"].map(|name| dataset.find_parameter(name).unwrap());
        indices.rotate_left(seed as usize % 3);
        let values = if seed % 2 == 0 {
            vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        } else {
            vec![1.01, -0.013, 0.003, 0.07, 0.99, -0.021, 0.001, 0.017, 1.03]
        };
        let matrix = CompensationMatrix {
            channels: indices
                .iter()
                .map(|i| dataset.parameters[*i].name.clone())
                .collect(),
            values,
        };
        let (expected_parameters, expected) =
            baseline_compensated_columns(&dataset, &indices, &matrix);
        let (actual_parameters, actual) = compensated_columns(&dataset, &indices, &matrix);
        assert_eq!(actual_parameters, expected_parameters);
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(
                actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn detects_signed_zero_and_f64_changes_before_rounding() {
    assert!(differs_from_stored(&[0.0], &ColumnData::F64(vec![-0.0])));
    assert!(differs_from_stored(
        &[1.0 + f64::EPSILON],
        &ColumnData::F32(vec![1.0])
    ));
    assert!(!differs_from_stored(
        &[16_777_217.0],
        &ColumnData::UInt(vec![16_777_217])
    ));
}

fn baseline_compensated_columns(
    dataset: &FcsDataset,
    matrix_indices: &[usize],
    matrix: &CompensationMatrix,
) -> (Vec<FcsWriteParameter>, Vec<Vec<f64>>) {
    let linear = matrix_indices
        .iter()
        .map(|index| cytiq_fcs::linear_values(dataset, *index))
        .collect::<Vec<_>>();
    let compensated = cytiq_fcs::compensate(&linear, &matrix.values);

    let mut parameters = Vec::with_capacity(dataset.parameters.len());
    let mut columns = Vec::with_capacity(dataset.parameters.len());
    for (index, parameter) in dataset.parameters.iter().enumerate() {
        let stored = stored_values(dataset, index);
        let values = if let Some(position) = matrix_indices.iter().position(|i| *i == index) {
            compensated[position].clone()
        } else if parameter.name.eq_ignore_ascii_case("time") {
            // `$TIMESTEP` is carried over and still applies to stored time.
            stored.clone()
        } else {
            cytiq_fcs::linear_values(dataset, index)
        };
        let mut output = FcsWriteParameter::from_source(parameter);
        if values
            .iter()
            .zip(&stored)
            .any(|(value, raw)| value.to_bits() != raw.to_bits())
        {
            output.range = None;
            output.keywords.retain(|(suffix, _)| suffix != "G");
            parameters.push(output);
            columns.push(values.iter().map(|value| round_to_f32(*value)).collect());
        } else {
            parameters.push(output);
            columns.push(values);
        }
    }
    (parameters, columns)
}
