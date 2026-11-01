//! `classify` reads its input in batches: features, lineage and mask decisions
//! computed batch by batch must equal those of the whole file.

use std::fs::{self, File};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use arrow_array::{Array, BooleanArray, Float32Array, UInt64Array};
use cytiq_classify::{
    Bundle, ClassifyConfig, ClassifyRunner, NormalizationContract,
    feature_columns_with_normalization,
};
use cytiq_fcs::{
    FcsDataset, FcsReadLimits, FcsReader, FcsStream, FcsWriteOptions, FcsWriteParameter,
};
use cytiq_mask::EventMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// (bundle directory, input FCS), relative to the classify fixtures.
const CASES: &[(&str, &str)] = &[
    ("catboost", "raw.fcs"),
    ("v2/binary", "raw.fcs"),
    ("v2/lightgbm", "v2/input.fcs"),
    ("cytof/lightgbm", "cytof/input.fcs"),
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}

fn read(path: &Path) -> FcsDataset {
    FcsReader::open(path).unwrap().read().unwrap()
}

fn bits(columns: &[Vec<f32>]) -> Vec<Vec<u32>> {
    columns
        .iter()
        .map(|column| column.iter().map(|value| value.to_bits()).collect())
        .collect()
}

#[test]
fn batch_features_equal_whole_file_features() {
    for &(bundle, input) in CASES {
        let bundle = Bundle::load(&fixtures().join(bundle), 0.0).unwrap();
        let normalization = bundle
            .normalization
            .clone()
            .unwrap_or(NormalizationContract::None);
        let path = fixtures().join(input);
        let whole =
            feature_columns_with_normalization(&read(&path), &bundle.profile, &normalization)
                .unwrap();
        for size in [1, 7, 65_536] {
            let FcsStream { metadata, batches } = FcsReader::open(&path)
                .unwrap()
                .stream(NonZeroUsize::new(size).unwrap(), FcsReadLimits::default())
                .unwrap();
            let mut window = FcsDataset {
                version: metadata.version,
                header: metadata.header,
                text: metadata.text,
                parameters: metadata.parameters,
                data: batches.empty_batch(),
                analysis: None,
            };
            let mut joined = vec![Vec::new(); whole.len()];
            for batch in batches {
                window.data = batch.unwrap();
                let part =
                    feature_columns_with_normalization(&window, &bundle.profile, &normalization)
                        .unwrap();
                for (column, values) in joined.iter_mut().zip(part) {
                    column.extend(values);
                }
            }
            assert_eq!(bits(&joined), bits(&whole), "{input} in batches of {size}");
        }
    }
}

struct Predictions {
    source_event_id: Vec<u64>,
    assessed: Vec<bool>,
    probabilities: Vec<Vec<Option<f32>>>,
}

fn predictions(output: &Path) -> Predictions {
    let file = File::open(output.join("predictions.parquet")).unwrap();
    let mut predictions = Predictions {
        source_event_id: Vec::new(),
        assessed: Vec::new(),
        probabilities: Vec::new(),
    };
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap()
    {
        let batch = batch.unwrap();
        let column = |name: &str| batch.column(batch.schema().index_of(name).unwrap()).clone();
        let source = column("source_event_id");
        let source = source.as_any().downcast_ref::<UInt64Array>().unwrap();
        predictions.source_event_id.extend(source.values());
        let assessed = column("assessed");
        let assessed = assessed.as_any().downcast_ref::<BooleanArray>().unwrap();
        predictions
            .assessed
            .extend(assessed.iter().map(Option::unwrap));
        let schema = batch.schema();
        let probability_columns = (0..schema.fields().len())
            .filter(|index| schema.field(*index).name().starts_with("p_"))
            .collect::<Vec<_>>();
        predictions
            .probabilities
            .resize(probability_columns.len(), Vec::new());
        for (slot, index) in probability_columns.into_iter().enumerate() {
            let values = batch.column(index);
            let values = values.as_any().downcast_ref::<Float32Array>().unwrap();
            predictions.probabilities[slot].extend(values.iter());
        }
    }
    predictions
}

fn classify(input: &Path, model: &Path, output: &Path, masks: Vec<PathBuf>, batch_size: usize) {
    ClassifyRunner::run(&ClassifyConfig {
        input: input.to_path_buf(),
        model: model.to_path_buf(),
        output: output.to_path_buf(),
        masks,
        threads: Some(1),
        batch_size,
        abstain_margin: 0.0,
        overwrite: false,
    })
    .unwrap();
}

/// Write the rows `retained` of `raw` as a derived file with `Original_ID`
/// and the raw acquisition, as a cleaning stage would. Parameters holding NaN
/// (the fixture's `population_id`) are dropped: the writer refuses them and
/// no feature reads them.
fn write_derived(raw: &Path, retained: &[usize], path: &Path) {
    let dataset = read(raw);
    let acquisition = cytiq_fcs::acquisition_id(raw, &dataset.text).unwrap();
    let (mut parameters, mut columns): (Vec<_>, Vec<_>) = dataset
        .parameters
        .iter()
        .zip(&dataset.data.columns)
        .map(|(parameter, column)| {
            let values = column.to_f64_vec();
            let values = retained.iter().map(|row| values[*row]).collect::<Vec<_>>();
            (FcsWriteParameter::from_source(parameter), values)
        })
        .filter(|(_, values)| values.iter().all(|value| value.is_finite()))
        .unzip();
    parameters.push(FcsWriteParameter::new(cytiq_fcs::ORIGINAL_ID_PARAMETER));
    #[allow(clippy::cast_precision_loss)]
    columns.push(retained.iter().map(|row| (row + 1) as f64).collect());
    let mut keywords = cytiq_fcs::carried_keywords(&dataset.text);
    cytiq_fcs::set_acquisition(&mut keywords, &acquisition);
    let options = FcsWriteOptions::new().with_text_keywords(keywords);
    cytiq_fcs::write_fcs3_1_exact_with_options(path, &parameters, &columns, &options).unwrap();
}

#[test]
fn derived_input_keeps_source_rows_across_batches() {
    let temp = tempfile::tempdir().unwrap();
    let raw = fixtures().join("cytof/input.fcs");
    let model = fixtures().join("cytof/lightgbm");
    let dataset = read(&raw);
    let events = dataset.event_count();
    // A derived file without every third raw event.
    let retained = (0..events).filter(|row| row % 3 != 0).collect::<Vec<_>>();
    let derived = temp.path().join("derived.fcs");
    write_derived(&raw, &retained, &derived);
    let acquisition = cytiq_fcs::acquisition_id(&raw, &dataset.text).unwrap();
    let mask = temp.path().join("raw.mask.parquet");
    EventMask::new(
        acquisition,
        (0..events).map(|row| row % 2 == 0).collect(),
        None,
    )
    .write_parquet(&mask)
    .unwrap();

    let whole_raw = temp.path().join("raw");
    classify(&raw, &model, &whole_raw, vec![mask.clone()], 65_536);
    let raw = predictions(&whole_raw);
    for size in [1, 5, 65_536] {
        let output = temp.path().join(format!("derived-{size}"));
        classify(&derived, &model, &output, vec![mask.clone()], size);
        let derived = predictions(&output);
        let expected_source = retained.iter().map(|row| *row as u64).collect::<Vec<_>>();
        assert_eq!(
            derived.source_event_id, expected_source,
            "batch size {size}"
        );
        let expected_assessed = retained.iter().map(|row| row % 2 == 0).collect::<Vec<_>>();
        assert_eq!(derived.assessed, expected_assessed, "batch size {size}");
        for (derived, raw) in derived.probabilities.iter().zip(&raw.probabilities) {
            for (row, value) in retained.iter().zip(derived) {
                match (value, raw[*row]) {
                    (Some(value), Some(expected)) => assert!((value - expected).abs() <= 1e-6),
                    (None, None) => {}
                    other => panic!("raw row {row}: {other:?}"),
                }
            }
        }
    }
}

#[test]
fn nonfinite_feature_in_a_late_batch_fails_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixtures().join("cytof/input.fcs");
    let model = fixtures().join("cytof/lightgbm");
    let bundle = Bundle::load(&model, 0.0).unwrap();
    let dataset = read(&source);
    let feature = &bundle.profile.features[0].channel;
    let column = dataset
        .parameters
        .iter()
        .position(|parameter| &parameter.name == feature)
        .unwrap();
    // $DATATYPE=F, little-endian: overwrite the last event's first feature.
    let mut bytes = fs::read(&source).unwrap();
    let data_start: usize = std::str::from_utf8(&bytes[26..34])
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let row_width = dataset.parameters.len() * 4;
    let at = data_start + (dataset.event_count() - 1) * row_width + column * 4;
    bytes[at..at + 4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    let input = temp.path().join("late-infinity.fcs");
    fs::write(&input, bytes).unwrap();

    let fresh = temp.path().join("fresh");
    let config = |output: &Path, overwrite| ClassifyConfig {
        input: input.clone(),
        model: model.clone(),
        output: output.to_path_buf(),
        masks: vec![],
        threads: Some(1),
        batch_size: 16,
        abstain_margin: 0.0,
        overwrite,
    };
    let error = ClassifyRunner::run(&config(&fresh, false)).unwrap_err();
    assert!(
        error.to_string().contains("nonfinite classifier feature"),
        "{error}"
    );
    assert!(!fresh.exists(), "a failed run left {}", fresh.display());

    // Over earlier outputs, a failed run leaves them byte-identical.
    let earlier = temp.path().join("earlier");
    classify(&source, &model, &earlier, vec![], 16);
    let before = fs::read_dir(&earlier)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    ClassifyRunner::run(&config(&earlier, true)).unwrap_err();
    let after = fs::read_dir(&earlier)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(after.len(), before.len());
    for (path, content) in before {
        assert!(
            after.contains(&(path.clone(), content)),
            "{} changed",
            path.display()
        );
    }
}
