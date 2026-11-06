mod common;
use common::{package, recorder, reseal, result};
use cytiq_population::{Measure, ObservationStatus, artifact::verify_package};
use cytiq_qc::CheckStatus;
use cytiq_specimen::{
    ReportConfig, ReportFormat, SpecimenComparison, SpecimenReport,
    reconcile::SpecimenCheck,
    report::{ReportData, write_report},
    runner::{CompareConfig, write_comparison},
};
use std::{fs, path::Path};

fn config(input: &Path, output: &Path, format: ReportFormat) -> ReportConfig {
    ReportConfig {
        input: input.into(),
        output: output.into(),
        format,
        overwrite: false,
    }
}

fn csv_rows(path: &Path) -> (csv::StringRecord, Vec<csv::StringRecord>) {
    let mut reader = csv::Reader::from_path(path).unwrap();
    (
        reader.headers().unwrap().clone(),
        reader
            .records()
            .collect::<std::result::Result<_, _>>()
            .unwrap(),
    )
}

#[test]
fn exports_exact_counts_nulls_qc_and_escaped_identifiers_in_every_format() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("input");
    let output = tmp.path().join("report");
    let mut source = result((1 << 53) + 1);
    source.protocol_id = "<script>alert(1)</script>,\"protocol\"\nline".into();
    source.status = CheckStatus::Fail;
    source.checks.push(SpecimenCheck {
        category: "technical".into(),
        scope: "P1".into(),
        check: "events".into(),
        status: CheckStatus::Fail,
        message: "<img src=x onerror=alert(2)>,\"message\"\nline".into(),
    });
    let mut absent = source.observations[0].clone();
    absent.target.id = "missing".into();
    absent.value_u64 = None;
    absent.status = ObservationStatus::NotMeasured;
    absent.reason = Some("missing source".into());
    absent.seal().unwrap();
    let mut zero = source.observations[0].clone();
    zero.target.id = "zero".into();
    zero.value_u64 = Some(0);
    zero.seal().unwrap();
    source.observations.extend([absent.clone(), zero.clone()]);
    source.results.extend([absent, zero]);
    package(&input, &source);
    let outcome = write_report(
        &config(&input, &output, ReportFormat::All),
        recorder("cytiq specimen report"),
    )
    .unwrap();
    assert_eq!(outcome.exit_code, 0);
    verify_package(&output).unwrap();
    let document: SpecimenReport =
        serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(document.source_outcome.exit_code, 3);
    assert!(
        matches!(document.content, ReportData::Specimen(ref r) if r.status == CheckStatus::Fail)
    );
    let html = fs::read_to_string(output.join("report.html")).unwrap();
    assert!(html.contains("9007199254740993"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("<script src="));
    assert!(!html.contains("<link"));
    let (headers, rows) = csv_rows(&output.join("observations.csv"));
    let count = headers.iter().position(|h| h == "value_u64").unwrap();
    assert!(rows.iter().any(|r| &r[count] == "9007199254740993"));
    assert!(rows.iter().any(|r| &r[count] == "0"));
    assert!(rows.iter().any(|r| r[count].is_empty()));
    let (_, summary) = csv_rows(&output.join("summary.csv"));
    assert_eq!(&summary[0][1], source.protocol_id);
    let (_, checks) = csv_rows(&output.join("checks.csv"));
    assert_eq!(&checks[0][4], source.checks[0].message);
}

#[test]
fn reports_comparisons_after_original_packages_are_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    let diff = tmp.path().join("diff");
    package(&a, &result((1 << 53) + 1));
    package(&b, &result((1 << 53) + 2));
    write_comparison(
        &CompareConfig {
            baseline: a.clone(),
            candidate: b.clone(),
            policy: None,
            output: diff.clone(),
            overwrite: false,
        },
        recorder("cytiq specimen compare"),
    )
    .unwrap();
    fs::remove_dir_all(a).unwrap();
    fs::remove_dir_all(b).unwrap();
    let output = tmp.path().join("report");
    let outcome = write_report(
        &config(&diff, &output, ReportFormat::All),
        recorder("cytiq specimen report"),
    )
    .unwrap();
    assert_eq!(outcome.exit_code, 0);
    let (headers, rows) = csv_rows(&output.join("rows.csv"));
    for (name, expected) in [
        ("absolute_delta_u64", "1"),
        ("baseline_value_u64", "9007199254740993"),
        ("candidate_value_u64", "9007199254740994"),
    ] {
        let index = headers.iter().position(|h| h == name).unwrap();
        assert_eq!(&rows[0][index], expected);
    }
    let report = SpecimenReport::load(&diff).unwrap();
    assert!(matches!(report.content, ReportData::Comparison(ref c) if c.summary.failed_rows == 2));
    assert!(
        fs::read_to_string(output.join("report.html"))
            .unwrap()
            .contains("checks_failed")
    );
    SpecimenComparison::load(&diff).unwrap();
}

#[test]
fn individual_formats_empty_tables_and_deterministic_exports() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("input");
    package(&input, &result(0));
    for (index, format) in [ReportFormat::Html, ReportFormat::Csv, ReportFormat::Json]
        .into_iter()
        .enumerate()
    {
        let output = tmp.path().join(index.to_string());
        write_report(&config(&input, &output, format), recorder("report")).unwrap();
        verify_package(&output).unwrap();
        assert_eq!(
            output.join("report.html").exists(),
            format == ReportFormat::Html
        );
        assert_eq!(
            output.join("report.json").exists(),
            format == ReportFormat::Json
        );
        assert_eq!(
            output.join("results.csv").exists(),
            format == ReportFormat::Csv
        );
    }
    let a = tmp.path().join("all-a");
    let b = tmp.path().join("all-b");
    write_report(&config(&input, &a, ReportFormat::All), recorder("report")).unwrap();
    write_report(&config(&input, &b, ReportFormat::All), recorder("report")).unwrap();
    for file in [
        "report.html",
        "report.json",
        "summary.csv",
        "observations.csv",
        "results.csv",
        "checks.csv",
    ] {
        assert_eq!(
            fs::read(a.join(file)).unwrap(),
            fs::read(b.join(file)).unwrap()
        );
    }
    assert_eq!(csv_rows(&a.join("checks.csv")).1.len(), 0);
    assert!(
        fs::read_to_string(a.join("report.html"))
            .unwrap()
            .contains("No rows.")
    );
}

#[test]
fn failed_or_incomplete_source_does_not_fail_export_and_errors_preserve_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("input");
    let output = tmp.path().join("output");
    let mut source = result(0);
    source.complete = false;
    package(&input, &source);
    let mut request = config(&input, &output, ReportFormat::All);
    assert_eq!(
        write_report(&request, recorder("report"))
            .unwrap()
            .exit_code,
        0
    );
    let before = fs::read(output.join("report.html")).unwrap();
    assert!(write_report(&request, recorder("report")).is_err());
    request.overwrite = true;
    assert_eq!(
        write_report(&request, recorder("report"))
            .unwrap()
            .exit_code,
        0
    );
    assert_eq!(before, fs::read(output.join("report.html")).unwrap());
    request.output = input.clone();
    assert!(write_report(&request, recorder("report")).is_err());
    cytiq_specimen::SpecimenResult::load(&input).unwrap();
    request.output = output.clone();
    fs::write(input.join("result.json"), b"{}").unwrap();
    assert!(write_report(&request, recorder("report")).is_err());
    verify_package(&output).unwrap();
}

#[test]
fn reference_estimates_remain_separate_from_both_measured_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("input");
    let output = tmp.path().join("output");
    let mut source = result(3000);
    let mut second = source.observations[0].clone();
    second.panel_id = "P2".into();
    second.value_u64 = Some(6400);
    second.seal().unwrap();
    source.observations.push(second.clone());
    for (panel, count) in [("P1", 10000), ("P2", 20000)] {
        let mut anchor = second.clone();
        anchor.panel_id = panel.into();
        anchor.target.id = "cells".into();
        anchor.value_u64 = Some(count);
        anchor.seal().unwrap();
        source.observations.push(anchor);
    }
    let mut estimate = second;
    estimate.measure = Measure::EstimatedCount;
    estimate.status = ObservationStatus::Estimated;
    estimate.value_u64 = None;
    estimate.value_f64 = Some(3200.0);
    estimate.method.name = "reference_ratio".into();
    estimate.method.experimental = true;
    estimate.method.source_observation_ids = vec![
        source.observations[1].observation_id.clone(),
        source.observations[3].observation_id.clone(),
        source.observations[2].observation_id.clone(),
    ];
    source.results = vec![estimate];
    reseal(&mut source);
    package(&input, &source);
    write_report(
        &config(&input, &output, ReportFormat::All),
        recorder("report"),
    )
    .unwrap();
    let html = fs::read_to_string(output.join("report.html")).unwrap();
    for text in ["3000", "6400", "3200", "estimated_count", "reference_ratio"] {
        assert!(html.contains(text), "missing {text}");
    }
    let (headers, rows) = csv_rows(&output.join("results.csv"));
    let count_column = headers.iter().position(|h| h == "value_u64").unwrap();
    let estimate_column = headers.iter().position(|h| h == "value_f64").unwrap();
    assert_eq!(&rows[0][count_column], "");
    assert_eq!(&rows[0][estimate_column], "3200.0");
}
