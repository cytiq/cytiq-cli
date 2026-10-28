use std::fs;
use std::path::{Path, PathBuf};

use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner, input_files};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}

fn model(root: &Path, kind: &str, json: bool) -> PathBuf {
    let path = root.join(format!("{kind}-{json}"));
    fs::create_dir(&path).unwrap();
    fs::copy(
        fixtures().join(format!("v2/{kind}/model.onnx")),
        path.join("model.onnx"),
    )
    .unwrap();
    for stem in ["manifest", "classes", "panel", "label-mapping"] {
        let content =
            fs::read_to_string(fixtures().join(format!("v4/{kind}/{stem}.yaml"))).unwrap();
        if json {
            let payload: Value = serde_saphyr::from_str(&content).unwrap();
            fs::write(
                path.join(format!("{stem}.json")),
                serde_json::to_string(&payload).unwrap(),
            )
            .unwrap();
        } else {
            fs::write(path.join(format!("{stem}.yaml")), content).unwrap();
        }
    }
    path
}

fn config(model: PathBuf, output: PathBuf, binary: bool) -> ClassifyConfig {
    ClassifyConfig {
        input: fixtures().join(if binary { "raw.fcs" } else { "v2/input.fcs" }),
        model,
        output,
        masks: vec![],
        threads: Some(1),
        batch_size: 17,
        abstain_margin: 0.0,
        overwrite: false,
    }
}

#[test]
fn python_exported_yaml_and_reformatted_json_produce_identical_predictions() {
    let temp = tempfile::tempdir().unwrap();
    for kind in ["binary", "catboost", "lightgbm"] {
        let mut outputs = vec![];
        for json in [false, true] {
            let cfg = config(
                model(temp.path(), kind, json),
                temp.path().join(format!("out-{kind}-{json}")),
                kind == "binary",
            );
            let result = ClassifyRunner::run(&cfg).unwrap();
            assert_eq!(result.schema_version, 4);
            outputs.push(fs::read(cfg.output.join("predictions.parquet")).unwrap());
            let files = input_files(&cfg.model).unwrap();
            assert_eq!(files.len(), 5);
            assert!(files.iter().all(|path| path.exists()));
            assert!(files.iter().any(|path| path.ends_with(if json {
                "manifest.json"
            } else {
                "manifest.yaml"
            })));
        }
        assert_eq!(outputs[0], outputs[1]);
    }
}

#[test]
fn mixed_extensions_are_supported_and_duplicate_files_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = model(temp.path(), "binary", false);
    fs::rename(path.join("classes.yaml"), path.join("classes.yml")).unwrap();
    let content = fs::read_to_string(path.join("panel.yaml")).unwrap();
    let panel: Value = serde_saphyr::from_str(&content).unwrap();
    fs::remove_file(path.join("panel.yaml")).unwrap();
    fs::write(path.join("panel.json"), serde_json::to_vec(&panel).unwrap()).unwrap();
    assert!(Bundle::load(&path, 0.0).is_ok());
    fs::copy(path.join("classes.yml"), path.join("classes.yaml")).unwrap();
    assert!(Bundle::load(&path, 0.0).is_err());
    assert!(input_files(&path).is_err());
}

#[test]
fn edited_knowledge_and_missing_normalization_fail_before_output() {
    let temp = tempfile::tempdir().unwrap();
    let path = model(temp.path(), "binary", false);
    let classes = path.join("classes.yaml");
    let content = fs::read_to_string(&classes).unwrap();
    fs::write(&classes, content.replace("non_blast", "edited")).unwrap();
    let cfg = config(path, temp.path().join("rejected"), true);
    assert!(ClassifyRunner::run(&cfg).is_err());
    assert!(!cfg.output.exists());
    let path = model(temp.path(), "catboost", false);
    let manifest = path.join("manifest.yaml");
    let mut payload: Value =
        serde_saphyr::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    payload.as_object_mut().unwrap().remove("normalization");
    fs::write(&manifest, serde_saphyr::to_string(&payload).unwrap()).unwrap();
    assert!(Bundle::load(&path, 0.0).is_err());
}
