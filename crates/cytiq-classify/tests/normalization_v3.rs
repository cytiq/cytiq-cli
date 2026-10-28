use std::fs::{self, File};
use std::path::{Path, PathBuf};

use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner, feature_columns};
use cytiq_fcs::{FcsReader, FcsWriteOptions, FcsWriteParameter, NormalizationState};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}

fn make_bundle(root: &Path, version: u32, normalization: Option<Value>) -> PathBuf {
    let source = fixtures().join("v2/binary");
    let destination = root.join(format!("bundle-{version}"));
    fs::create_dir_all(&destination).unwrap();
    for name in [
        "model.onnx",
        "manifest.json",
        "classes.json",
        "panel.json",
        "label-mapping.json",
    ] {
        fs::copy(source.join(name), destination.join(name)).unwrap();
    }
    let mut manifest: Value =
        serde_json::from_reader(File::open(destination.join("manifest.json")).unwrap()).unwrap();
    manifest["schema_version"] = version.into();
    if let Some(value) = normalization {
        manifest["normalization"] = value;
    }
    fs::write(
        destination.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    destination
}

fn transformed_input(root: &Path, normalized: bool) -> PathBuf {
    let raw = FcsReader::open(fixtures().join("raw.fcs"))
        .unwrap()
        .read()
        .unwrap();
    let bundle = Bundle::load(&fixtures().join("v2/binary"), 0.0).unwrap();
    let features = feature_columns(&raw, &bundle.profile).unwrap();
    let mut columns = raw
        .data
        .columns
        .iter()
        .map(cytiq_fcs::ColumnData::to_f64_vec)
        .collect::<Vec<_>>();
    for (feature, values) in bundle.profile.features.iter().zip(features) {
        let i = raw.find_parameter(&feature.channel).unwrap();
        columns[i] = values.into_iter().map(f64::from).collect();
    }
    let fluorescence = bundle
        .profile
        .features
        .iter()
        .filter(|f| f.fluorescence)
        .map(|f| f.channel.clone())
        .collect::<Vec<_>>();
    let parameters = raw
        .parameters
        .iter()
        .map(FcsWriteParameter::from_source)
        .collect::<Vec<_>>();
    let mut keywords = cytiq_fcs::carried_keywords(&raw.text)
        .into_iter()
        .filter(|(k, _)| !cytiq_fcs::is_spillover_keyword(k))
        .collect::<Vec<_>>();
    keywords.push((
        "CYTIQ_COMPENSATION".into(),
        json!({"source":"fcs","channels":fluorescence}).to_string(),
    ));
    keywords.push(("CYTIQ_TRANSFORM".into(),json!({"method":"arcsinh","parameters":{"cofactor":bundle.profile.cofactor},"files":[{"channels":fluorescence}]}).to_string()));
    if normalized {
        keywords.push((
            "CYTIQ_NORMALIZATION".into(),
            serde_json::to_string(&NormalizationState {
                schema_version: 1,
                model_sha256: "ab".repeat(32),
                panel_sha256: "cd".repeat(32),
                batch: "a".into(),
                channels: fluorescence,
            })
            .unwrap(),
        ));
    }
    let path = root.join(if normalized {
        "normalized.fcs"
    } else {
        "transformed.fcs"
    });
    cytiq_fcs::write_fcs3_1_exact_with_options(
        &path,
        &parameters,
        &columns,
        &FcsWriteOptions::new().with_text_keywords(keywords),
    )
    .unwrap();
    path
}

fn config(input: PathBuf, model: PathBuf, output: PathBuf) -> ClassifyConfig {
    ClassifyConfig {
        input,
        model,
        output,
        masks: vec![],
        threads: Some(1),
        batch_size: 32,
        abstain_margin: 0.0,
        overwrite: false,
    }
}

#[test]
fn v3_checks_exact_normalizer_before_inference_and_preserves_v2_class_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let input = transformed_input(root, true);
    let model = make_bundle(
        root,
        3,
        Some(json!({"kind":"cytonorm","model_sha256":"ab".repeat(32)})),
    );
    let cfg = config(input.clone(), model.clone(), root.join("accepted"));
    let result = ClassifyRunner::run(&cfg).unwrap();
    assert_eq!(result.schema_version, 3);
    assert_eq!(result.assessed, 128);
    assert!(result.class_counts.is_some());
    assert!(result.normalization.is_some());
    let unnormalized = transformed_input(root, false);
    let cfg = config(unnormalized, model, root.join("rejected-missing"));
    assert!(ClassifyRunner::run(&cfg).is_err());
    assert!(!cfg.output.exists());
    let model = make_bundle(
        root,
        3,
        Some(json!({"kind":"cytonorm","model_sha256":"ef".repeat(32)})),
    );
    let cfg = config(input, model, root.join("rejected-wrong-model"));
    assert!(ClassifyRunner::run(&cfg).is_err());
    assert!(!cfg.output.exists());
}

#[test]
fn legacy_and_explicit_none_contracts_reject_normalized_fcs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let normalized = transformed_input(root, true);
    for version in [1, 2, 3] {
        let contract = (version == 3).then(|| json!({"kind":"none"}));
        let model = make_bundle(root, version, contract);
        let cfg = config(
            normalized.clone(),
            model,
            root.join(format!("rejected-{version}")),
        );
        let error = ClassifyRunner::run(&cfg).unwrap_err().to_string();
        assert!(error.contains("normalization"), "{error}");
        assert!(!cfg.output.exists());
    }
    let dataset = FcsReader::open(normalized).unwrap().read().unwrap();
    let bundle = Bundle::load(&fixtures().join("v2/binary"), 0.0).unwrap();
    assert!(feature_columns(&dataset, &bundle.profile).is_err());
    let model = make_bundle(root, 3, None);
    assert!(Bundle::load(&model, 0.0).is_err());
}
