use std::fs::File;
use std::path::PathBuf;

use cytiq_classify::{Bundle, feature_columns};
use cytiq_fcs::FcsReader;

#[test]
fn features_match_the_python_recipe() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify");
    let bundle: Bundle =
        serde_json::from_reader(File::open(fixtures.join("catboost/manifest.json")).unwrap())
            .unwrap();
    let expected: serde_json::Value =
        serde_json::from_reader(File::open(fixtures.join("expected.json")).unwrap()).unwrap();
    let dataset = FcsReader::open(fixtures.join("raw.fcs"))
        .unwrap()
        .read()
        .unwrap();
    let columns = feature_columns(&dataset, &bundle.profile).unwrap();
    for (row, values) in expected["features"].as_array().unwrap().iter().enumerate() {
        for (column, value) in values.as_array().unwrap().iter().enumerate() {
            assert!((f64::from(columns[column][row]) - value.as_f64().unwrap()).abs() < 1e-5);
        }
    }
}
