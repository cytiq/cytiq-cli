#![allow(clippy::cast_precision_loss)]
use cytiq_fcs::{FcsWriteOptions, FcsWriteParameter};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn write_sample(
    path: &Path,
    batch_shift: f64,
    biological_shift: f64,
    phase: f64,
    only_first: bool,
) {
    let n = 2000;
    let marker=(0..n).map(|i| if only_first||i<n/2 {0.0}else{10.0} + f64::from(i % (n/2))/1000.0+batch_shift+phase).collect::<Vec<_>>();
    let functional = (0..n)
        .map(|i| 2.0 + f64::from(i % (n / 2)) / 1000.0 + batch_shift + biological_shift + phase)
        .collect::<Vec<_>>();
    let channels = vec!["A", "B"];
    let options=FcsWriteOptions::new()
        .with_text_keyword("$CYT","synthetic")
        .with_text_keyword("CYTIQ_COMPENSATION",json!({"source":"fcs","channels":channels}).to_string())
        .with_text_keyword("CYTIQ_TRANSFORM",json!({"tool":"cytiq transform","version":"0.1.0","method":"arcsinh","parameters":{"cofactor":150.0},"files":[{"channels":channels}]}).to_string())
        .with_text_keyword("CYTIQ_MARGINS",json!({"A":{"lower":0.0,"upper":20.0},"B":{"lower":0.0,"upper":20.0}}).to_string());
    let params = vec![
        FcsWriteParameter::with_label("A", "CD3"),
        FcsWriteParameter::with_label("B", "CD4"),
        FcsWriteParameter::new("Time"),
        FcsWriteParameter::new("Original_ID"),
    ];
    cytiq_fcs::write_fcs3_1_exact_with_options(
        path,
        &params,
        &[
            marker,
            functional,
            (0..n).map(f64::from).collect(),
            (0..n).map(|i| f64::from(i + 1)).collect(),
        ],
        &options,
    )
    .unwrap();
}

pub fn design(directory: &Path, mode: &str) -> (PathBuf, Value) {
    write_sample(&directory.join("a.fcs"), 0.0, 0.0, 0.0, false);
    write_sample(&directory.join("b.fcs"), 1.0, 0.0, 0.0, false);
    write_sample(&directory.join("va.fcs"), 0.0, 3.0, 0.001, false);
    write_sample(&directory.join("vb.fcs"), 1.0, 3.0, 0.001, false);
    let manifest = json!({"schema_version":1,"panel":{"id":"P1","markers":[{"marker":"CD3","channel":"A"},{"marker":"CD4","channel":"B"}]},
        "mode":mode,"clustering_markers":["CD3"],"normalization_markers":["CD3","CD4"],
        "som":{"xdim":4,"ydim":2,"metaclusters":2},"training_events":4000,
        "samples":[{"id":"a","batch":"a","file":"a.fcs"},{"id":"b","batch":"b","file":"b.fcs"},
            {"id":"va","batch":"a","file":"va.fcs","role":"validation"},{"id":"vb","batch":"b","file":"vb.fcs","role":"validation"}]});
    let path = directory.join("batches.json");
    save(&path, &manifest);
    (path, manifest)
}
pub fn save(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
