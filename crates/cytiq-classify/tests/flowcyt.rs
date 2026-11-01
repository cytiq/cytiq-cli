//! The `FlowCyt` stored-value profile on real author exports
//! (`data/fixtures/flowcyt`): Kaluza names itself in `$CYT` and keeps the
//! cytometer in `$OriginalInstrument`.

use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner, Profile, feature_columns};
use cytiq_fcs::{
    ColumnData, FcsDataset, FcsReader, FcsWriteOptions, FcsWriteParameter, find_parameter_index,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures")
}

fn profile() -> Profile {
    Bundle::load(&root().join("classify/v2/catboost"), 0.0)
        .unwrap()
        .profile
}

fn author_file(name: &str) -> FcsDataset {
    FcsReader::new(File::open(root().join("flowcyt").join(name)).unwrap())
        .read()
        .unwrap()
}

/// A Kaluza-shaped file with the profile's channels and the given keywords.
fn kaluza_like(text: &[(&str, &str)]) -> FcsDataset {
    let profile = profile();
    let parameters = profile
        .features
        .iter()
        .map(|f| {
            FcsWriteParameter::with_label(&f.channel, f.labels.first().cloned().unwrap_or_default())
        })
        .collect::<Vec<_>>();
    let columns = vec![vec![1.0_f32, 2.0]; parameters.len()];
    let options = FcsWriteOptions::new().with_text_keywords(
        text.iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
    );
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(&mut bytes, &parameters, &columns, &options)
        .unwrap();
    FcsReader::new(Cursor::new(bytes)).read().unwrap()
}

#[test]
fn kaluza_exports_give_their_stored_values() {
    let dataset = author_file("Case12_P.fcs");
    let profile = profile();
    let features = feature_columns(&dataset, &profile).unwrap();
    assert_eq!(features.len(), profile.features.len());
    for (feature, values) in profile.features.iter().zip(&features) {
        let index = find_parameter_index(&dataset.parameters, &feature.channel).unwrap();
        let Some(ColumnData::F32(stored)) = dataset.data.column(index) else {
            panic!("{} is not float32", feature.channel);
        };
        assert_eq!(values, stored, "{}", feature.channel);
    }
}

#[test]
fn kaluza_exports_must_name_the_profile_instrument_as_original() {
    let mut other = profile();
    other.instrument = "Gallios".into();
    let error = feature_columns(&author_file("Case12_P.fcs"), &other).unwrap_err();
    assert!(error.to_string().contains("instrument"), "{error}");
    for text in [
        vec![("$CYT", "Kaluza")],
        vec![("$CYT", "Kaluza 2.1"), ("$OriginalInstrument", "Navios")],
        vec![("$CYT", "Navios")],
    ] {
        let error = feature_columns(&kaluza_like(&text), &profile()).unwrap_err();
        assert!(
            error.to_string().contains("instrument"),
            "{text:?}: {error}"
        );
    }
    let accepted = kaluza_like(&[("$CYT", "Kaluza"), ("$OriginalInstrument", "NAVIOS")]);
    assert!(feature_columns(&accepted, &profile()).is_ok());
}

#[test]
fn flowcore_reexports_without_instrument_keywords_are_rejected() {
    let error = feature_columns(&author_file("Case30_P.fcs"), &profile()).unwrap_err();
    assert!(error.to_string().contains("instrument"), "{error}");
}

#[test]
fn classifies_a_real_kaluza_export() {
    let temp = tempfile::tempdir().unwrap();
    let config = ClassifyConfig {
        input: root().join("flowcyt/Case12_P.fcs"),
        model: root().join("classify/v2/catboost"),
        output: temp.path().join("out"),
        masks: vec![],
        threads: Some(1),
        batch_size: 2,
        abstain_margin: 0.0,
        overwrite: false,
    };
    let result = ClassifyRunner::run(&config).unwrap();
    assert_eq!(result.assessed, 3);
    assert_eq!(result.class_counts.unwrap().values().sum::<usize>(), 3);
}
