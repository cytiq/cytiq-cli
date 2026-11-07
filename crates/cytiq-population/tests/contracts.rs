use cytiq_population::{
    PopulationObservation,
    artifact::{verify_package, write_package},
    knowledge::KnowledgeArtifact,
    serialization, table,
};
use cytiq_provenance::{RunRecorder, outcome::OutcomeReason};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/specimen")
        .join(name)
}

fn count(value: u64) -> PopulationObservation {
    let sha = "a".repeat(64);
    let mut observation: PopulationObservation = serde_json::from_value(json!({
        "schema_version":1,"observation_id":"","specimen_id":"S01","panel_id":"P1","acquisition_id":"A01","source_id":"gates",
        "target":{"kind":"population","id":"t_cells","scope":"cells","state":null},
        "measure":"observed_count","unit":"events","status":"measured","value_u64":value,"value_f64":null,
        "denominator":{"target":null,"scope":"cells","observed_count":value},
        "method":{"name":"gating","version":"1","definition_sha256":sha,"experimental":false,"source_observation_ids":[]},
        "lineage":{"acquisition_sha256":sha,"tree_sha256":sha,"panel_profile_sha256":sha,"projection_sha256":sha,"annotations_sha256":sha,
            "producer_provenance_sha256":sha,"analysis_mask_sha256":sha},"n_events":value,"supporting_count":null,"reason":null
    })).unwrap();
    observation.seal().unwrap();
    observation
}

#[test]
fn observation_fields_are_validated_independently_of_their_digest() {
    let base = serde_json::to_value(count(10)).unwrap();
    for (pointer, replacement) in [
        ("/specimen_id", json!(" ")),
        ("/panel_id", json!("")),
        ("/source_id", json!("")),
        ("/method/definition_sha256", json!("g".repeat(64))),
        ("/method/name", json!("")),
        ("/method/version", json!("")),
        ("/value_f64", json!(-0.1)),
        ("/value_f64", json!(1.0)),
        ("/status", json!("estimated")),
        ("/denominator/observed_count", json!(9)),
    ] {
        let mut value = base.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let mut row: PopulationObservation = serde_json::from_value(value).unwrap();
        assert!(row.seal().is_err(), "accepted invalid {pointer}");
    }
}

#[test]
fn numeric_observations_obey_their_measure_contract() {
    use cytiq_population::{Measure, ObservationStatus as S};
    for (measure, unit, status, integer, float, denominator, valid) in [
        (
            Measure::EstimatedCount,
            "events",
            S::Estimated,
            None,
            Some(0.0),
            10,
            true,
        ),
        (
            Measure::EstimatedCount,
            "events",
            S::Measured,
            None,
            Some(0.0),
            10,
            false,
        ),
        (
            Measure::EstimatedCount,
            "events",
            S::Estimated,
            Some(1),
            None,
            10,
            false,
        ),
        (
            Measure::Fraction,
            "fraction",
            S::Measured,
            None,
            Some(1.0),
            10,
            true,
        ),
        (
            Measure::Fraction,
            "fraction",
            S::Measured,
            None,
            Some(1.1),
            10,
            false,
        ),
        (
            Measure::Fraction,
            "fraction",
            S::Measured,
            None,
            Some(0.0),
            0,
            false,
        ),
        (
            Measure::Concentration,
            "cells_per_ul",
            S::Measured,
            None,
            Some(0.0),
            10,
            true,
        ),
        (
            Measure::Concentration,
            "cells_per_ul",
            S::Measured,
            Some(1),
            None,
            10,
            false,
        ),
    ] {
        let mut row = count(10);
        row.measure = measure;
        row.unit = unit.into();
        row.status = status;
        row.value_u64 = integer;
        row.value_f64 = float;
        row.denominator.as_mut().unwrap().observed_count = denominator;
        assert_eq!(
            row.seal().is_ok(),
            valid,
            "{measure:?}/{status:?}/{float:?}"
        );
    }
}

#[test]
fn unavailable_observations_require_a_reason_without_either_numeric_value() {
    use cytiq_population::ObservationStatus as S;
    for (integer, float, reason, valid) in [
        (None, None, None, false),
        (Some(0), Some(0.0), Some("missing"), false),
        (None, None, Some("missing"), true),
    ] {
        let mut row = count(10);
        row.status = S::NotMeasured;
        row.value_u64 = integer;
        row.value_f64 = float;
        row.reason = reason.map(str::to_owned);
        assert_eq!(row.seal().is_ok(), valid);
    }
}

#[test]
fn sha256_requires_exactly_64_lowercase_hex_digits() {
    use cytiq_population::observation::is_sha256;
    for value in [
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        "A".repeat(64),
        String::new(),
    ] {
        assert!(!is_sha256(&value), "{value}");
    }
    assert!(is_sha256(&"0123456789abcdef".repeat(4)));
}

#[test]
fn floating_observations_reject_negative_and_non_finite_values() {
    for value in [-0.1, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut row = count(10);
        row.measure = cytiq_population::Measure::EstimatedCount;
        row.status = cytiq_population::ObservationStatus::Estimated;
        row.value_u64 = None;
        row.value_f64 = Some(value);
        assert!(row.seal().is_err(), "accepted {value}");
    }
}

#[test]
fn completion_markers_cannot_hide_independently_invalid_fields() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("result");
    write_package(
        &output,
        false,
        RunRecorder::start("test", "1", vec![]),
        "test",
        OutcomeReason::Success,
        |staged| {
            std::fs::write(staged.join("data.json"), b"data")?;
            Ok(vec!["data.json".into()])
        },
    )
    .unwrap();
    let path = output.join("outcome.json");
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (pointer, replacement) in [
        ("/schema_version", json!(2)),
        ("/completed", json!(false)),
        ("/exit_code", json!(3)),
        ("/artifacts/0/bytes", json!(5)),
        ("/artifacts/0/sha256", json!("0".repeat(64))),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(verify_package(&output).is_err(), "accepted {pointer}");
    }
    let mut value = original.clone();
    value["reason"] = json!("runtime_error");
    value["exit_code"] = json!(1);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(verify_package(&output).is_err());
    let mut value = original.clone();
    value["artifacts"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["path"] == "provenance.json");
    std::fs::remove_file(output.join("data.json")).unwrap();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(verify_package(&output).is_err());
    std::fs::write(output.join("data.json"), b"data").unwrap();
    let mut value = original.clone();
    value["artifacts"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["path"] != "provenance.json");
    let provenance = std::fs::read(output.join("provenance.json")).unwrap();
    std::fs::remove_file(output.join("provenance.json")).unwrap();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(verify_package(&output).is_err());
    std::fs::write(output.join("provenance.json"), provenance).unwrap();
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    verify_package(&output).unwrap();
}

#[test]
fn observed_counts_above_float_precision_round_trip_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("counts.parquet");
    let original = vec![count(0), count((1_u64 << 53) + 1), count(u64::MAX)];
    table::write(&path, &original).unwrap();
    assert_eq!(table::read(&path).unwrap(), original);
    let encoded = serde_json::to_vec(&original).unwrap();
    assert_eq!(
        serde_json::from_slice::<Vec<PopulationObservation>>(&encoded).unwrap(),
        original
    );
}

#[test]
fn unavailable_states_cannot_carry_a_usable_numeric_value() {
    use cytiq_population::ObservationStatus as S;
    for status in [
        S::NotMeasured,
        S::QcRejected,
        S::BelowDetection,
        S::NotApplicable,
    ] {
        let mut observation = count(0);
        observation.status = status;
        assert!(observation.seal().is_err());
        observation.value_u64 = None;
        observation.reason = Some("explicit_reason".into());
        observation.seal().unwrap();
    }
    let mut observation = count(1);
    observation.value_f64 = Some(f64::NAN);
    assert!(observation.seal().is_err());
    observation = count(1);
    observation.value_u64 = Some(2);
    assert!(observation.seal().is_err());
}

#[test]
fn external_knowledge_is_validated_and_digest_pinned() {
    for name in [
        "P1.projection.json",
        "P2.projection.json",
        "facscanto2-p1.projection.json",
        "flowcyt-bm.projection.json",
    ] {
        KnowledgeArtifact::load(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["tree"]["cells"][1]["parent"] = json!("missing");
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["panel"]["cofactor"] = json!(0);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["tree"]["unknown"] = json!(true);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["populations"][0]["status"] = json!("ambiguous");
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("tree.json")).unwrap();
    let evidence = value["cells"][0]["evidence"].clone();
    value["cells"][0]["marker_signature"] = json!([
        {"marker":"CD3","state":"positive","inherited":true,"evidence":evidence}
    ]);
    value["cells"][1]["marker_signature"] = json!([
        {"marker":"CD3","state":"negative","evidence":evidence}
    ]);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("tree.json")).unwrap();
    value["ontology_edges"] =
        json!([{"child":"CL:A","parent":"CL:B"},{"child":"CL:B","parent":"CL:A"}]);
    assert!(KnowledgeArtifact::from_value(value).is_err());
}

#[test]
fn omitted_factory_defaults_have_the_same_upstream_artifact_identity() {
    let complete = serialization::read(&fixture("tree.json")).unwrap();
    let expected = KnowledgeArtifact::from_value(complete.clone()).unwrap();
    let mut compact = complete;
    for key in [
        "phenotypes",
        "ontology_edges",
        "tasks",
        "mappings",
        "decisions",
        "snapshots",
    ] {
        compact.as_object_mut().unwrap().remove(key);
    }
    for cell in compact["cells"].as_array_mut().unwrap() {
        for key in ["aliases", "ontology_ids", "marker_signature"] {
            cell.as_object_mut().unwrap().remove(key);
        }
    }
    assert_eq!(
        KnowledgeArtifact::from_value(compact).unwrap().sha256(),
        expected.sha256()
    );
}

#[test]
fn duplicate_json_keys_and_excessive_nesting_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.json");
    for text in [
        "{\"a\":1,\"a\":2}".into(),
        format!("{{\"a\":{}0{}}}", "[".repeat(66), "]".repeat(66)),
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(serialization::read(&path).is_err());
    }
}

#[test]
fn failed_replacement_preserves_the_entire_previous_package() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("result");
    let record = || RunRecorder::start("test", "1", vec![]);
    write_package(
        &output,
        false,
        record(),
        "test",
        OutcomeReason::Success,
        |staged| {
            std::fs::write(staged.join("data.json"), b"old")?;
            Ok(vec!["data.json".into()])
        },
    )
    .unwrap();
    let original = std::fs::read(output.join("outcome.json")).unwrap();
    let error = write_package(
        &output,
        true,
        record(),
        "test",
        OutcomeReason::Success,
        |staged| {
            std::fs::write(staged.join("data.json"), b"new")?;
            Err(cytiq_population::PopulationError::Invalid(
                "injected writer failure".into(),
            ))
        },
    );
    assert!(error.is_err());
    assert_eq!(std::fs::read(output.join("data.json")).unwrap(), b"old");
    assert_eq!(
        std::fs::read(output.join("outcome.json")).unwrap(),
        original
    );
    verify_package(&output).unwrap();
    std::fs::write(output.join("data.json"), b"corrupt").unwrap();
    assert!(verify_package(&output).is_err());
    let _value: Value = serde_json::from_slice(&original).unwrap();
}

#[test]
fn vendored_cytiq_tree_schemas_match_lock() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock: Value = serde_json::from_slice(
        &std::fs::read(root.join("contracts/cytiq-tree.lock.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lock["repository"], "cytiq/cytiq-tree");
    let revision = lock["revision"].as_str().unwrap();
    assert!(revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let mut locked = std::collections::BTreeSet::new();
    for file in lock["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        let digest: String = Sha256::digest(std::fs::read(root.join(path)).unwrap())
            .iter()
            .fold(String::new(), |mut text, byte| {
                use std::fmt::Write as _;
                write!(text, "{byte:02x}").expect("write to a String");
                text
            });
        assert_eq!(digest, file["sha256"].as_str().unwrap(), "{path}");
        assert!(
            file["source"]
                .as_str()
                .is_some_and(|source| !source.is_empty())
        );
        assert!(locked.insert(path.to_owned()), "{path} is locked twice");
    }
    let vendored = std::fs::read_dir(root.join("contracts/v2"))
        .unwrap()
        .map(|entry| {
            format!(
                "contracts/v2/{}",
                entry.unwrap().file_name().to_str().unwrap()
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        vendored, locked,
        "every vendored schema is locked, and only those"
    );
}

#[test]
fn all_knowledge_kinds_preserve_defaults_and_integer_representation() {
    let projection = serialization::read(&fixture("P1.projection.json")).unwrap();
    let sha = "a".repeat(64);
    let vocabulary = json!({
        "artifact_type":"class_vocabulary", "version":"1", "tree_version":"1",
        "tree_sha256":sha,"panel_sha256":sha,"registry_sha256":sha,"projection_sha256":sha,
        "panel_id":"P1","task":"task","task_kind":"multiclass","scope":"cells",
        "classes":[{"id":"a","target":"a","index":0,"parent":"cells"},{"id":"b","target":"b","index":1,"parent":"cells"}],
        "hierarchy":{"cells":null,"a":"cells","b":"cells"},"partition_complete":true,"decision_rule":"argmax"
    });
    let mapping = json!({"artifact_type":"label_mapping","version":"1","dataset":"test","dataset_version":"1","task":"task","vocabulary_sha256":sha,"labels":{"A":"a"}});
    for value in [
        projection.clone(),
        projection["tree"].clone(),
        projection["tree"]["registry"].clone(),
        projection["panel"].clone(),
        vocabulary,
        mapping,
    ] {
        let expanded = KnowledgeArtifact::from_value(value.clone()).unwrap();
        let mut compact = expanded.value().clone();
        compact.as_object_mut().unwrap().remove("schema_version");
        assert_eq!(
            KnowledgeArtifact::from_value(compact).unwrap().sha256(),
            expanded.sha256()
        );
        let mut invalid = value.clone();
        invalid["schema_version"] = json!(2.0);
        assert!(KnowledgeArtifact::from_value(invalid).is_err());
        let mut invalid = value;
        invalid["unexpected"] = json!(true);
        assert!(KnowledgeArtifact::from_value(invalid).is_err());
    }
    let mut tree = projection["tree"].clone();
    tree["generated_at"] = json!("historically-unchecked-date");
    KnowledgeArtifact::from_value(tree).unwrap();
}

#[test]
fn observation_table_keeps_physical_schema_and_structured_json() {
    use arrow_array::{Array, StringArray};
    use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
    use std::{fs::File, sync::Arc};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rows.parquet");
    let mut row = count((1_u64 << 53) + 1);
    row.acquisition_id = None;
    row.denominator = None;
    row.seal().unwrap();
    table::write(&path, &[row.clone()]).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();
    let schema = builder.schema().clone();
    assert_eq!(schema.metadata()["cytiq.population.schema_version"], "1");
    assert_eq!(
        schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect::<Vec<_>>(),
        [
            "schema_version",
            "observation_id",
            "specimen_id",
            "panel_id",
            "acquisition_id",
            "source_id",
            "target_id",
            "target_kind",
            "measure",
            "unit",
            "status",
            "reason",
            "target",
            "denominator",
            "method",
            "lineage",
            "value_u64",
            "n_events",
            "supporting_count",
            "value_f64"
        ]
    );
    let batch = builder.build().unwrap().next().unwrap().unwrap();
    let value = serde_json::to_value(&row).unwrap();
    for name in ["target", "denominator", "method", "lineage"] {
        let column = batch
            .column_by_name(name)
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(
            column.value(0),
            serde_json::to_string(&value[name]).unwrap()
        );
    }
    assert!(batch.column_by_name("acquisition_id").unwrap().is_null(0));
    let indices = (0..schema.fields().len()).rev().collect::<Vec<_>>();
    let reordered = batch.project(&indices).unwrap();
    let reordered_schema = Arc::new(arrow_schema::Schema::new_with_metadata(
        reordered.schema().fields().clone(),
        schema.metadata().clone(),
    ));
    let reordered = reordered.with_schema(reordered_schema).unwrap();
    let reordered_path = directory.path().join("reordered.parquet");
    let mut writer = ArrowWriter::try_new(
        File::create(&reordered_path).unwrap(),
        reordered.schema(),
        None,
    )
    .unwrap();
    writer.write(&reordered).unwrap();
    writer.close().unwrap();
    assert_eq!(table::read(&reordered_path).unwrap(), [row]);
    let mut columns = batch.columns().to_vec();
    columns[schema.index_of("target_id").unwrap()] = Arc::new(StringArray::from(vec!["wrong"]));
    let invalid = arrow_array::RecordBatch::try_new(schema.clone(), columns).unwrap();
    let invalid_path = directory.path().join("invalid.parquet");
    let mut writer =
        ArrowWriter::try_new(File::create(&invalid_path).unwrap(), schema, None).unwrap();
    writer.write(&invalid).unwrap();
    writer.close().unwrap();
    assert!(table::read(&invalid_path).is_err());
}

#[test]
fn observation_table_handles_empty_and_multiple_batches() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rows.parquet");
    table::write(&path, &[]).unwrap();
    assert_eq!(
        table::read(&path).unwrap(),
        Vec::<PopulationObservation>::new()
    );
    let rows = vec![count(1); cytiq_population::DEFAULT_BATCH_SIZE + 1];
    table::write(&path, &rows).unwrap();
    assert_eq!(table::read(&path).unwrap(), rows);
}

#[test]
fn observation_reader_rejects_coerced_types_and_invalid_structured_fields() {
    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Schema};
    use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
    use std::{fs::File, sync::Arc};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rows.parquet");
    table::write(&path, &[count(1)]).unwrap();
    let batch = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let cases: Vec<(&str, DataType, ArrayRef)> = vec![
        (
            "value_u64",
            DataType::Int64,
            Arc::new(Int64Array::from(vec![1])),
        ),
        (
            "target",
            DataType::Utf8,
            Arc::new(StringArray::from(vec!["invalid json"])),
        ),
        (
            "denominator",
            DataType::Utf8,
            Arc::new(StringArray::from(vec![None::<&str>])),
        ),
    ];
    for (name, kind, column) in cases {
        let index = batch.schema().index_of(name).unwrap();
        let mut columns = batch.columns().to_vec();
        columns[index] = column;
        let mut fields = batch.schema().fields().to_vec();
        fields[index] = Arc::new(fields[index].as_ref().clone().with_data_type(kind));
        let schema = Arc::new(Schema::new_with_metadata(
            fields,
            batch.schema().metadata().clone(),
        ));
        let invalid = RecordBatch::try_new(schema.clone(), columns).unwrap();
        let output = directory.path().join("invalid.parquet");
        let mut writer =
            ArrowWriter::try_new(File::create(&output).unwrap(), schema, None).unwrap();
        writer.write(&invalid).unwrap();
        writer.close().unwrap();
        assert!(table::read(&output).is_err(), "accepted invalid {name}");
    }
}
