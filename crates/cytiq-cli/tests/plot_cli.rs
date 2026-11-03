mod common;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use arrow_array::builder::StringBuilder;
use arrow_array::{ArrayRef, RecordBatch, UInt64Array};
use cytiq_mask::EventMask;
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;

const FLUORESCENCE: &str = "FL1-A,FL2-A,FL3-A,FL4-A,FL5-A";

fn fixture(area: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures")
        .join(area)
        .join(name)
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be UTF-8")
}

fn cytiq(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(args)
        .output()
        .expect("cytiq should run")
}

fn run(args: &[&str]) -> Output {
    let output = cytiq(args);
    assert!(
        output.status.success(),
        "cytiq {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fails(args: &[&str]) -> String {
    let output = cytiq(args);
    assert!(!output.status.success(), "cytiq {args:?} should fail");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Read an SVG and check the vector-only contract every plot must keep.
fn read_svg(path: &Path) -> String {
    let svg = fs::read_to_string(path).expect("SVG should exist");
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.trim_end().ends_with("</svg>"));
    for forbidden in ["<image", "<script", "foreignObject", "href=", "data:"] {
        assert!(!svg.contains(forbidden), "SVG contains {forbidden}");
    }
    svg
}

/// Tick labels of both axes, in document order.
fn tick_labels(svg: &str) -> Vec<String> {
    svg.split("<text ")
        .filter(|element| {
            element.contains(r#"text-anchor="middle">"#) && !element.contains("font-size")
                || element.contains(r#"text-anchor="end">"#)
        })
        .filter_map(|element| {
            let start = element.find('>')? + 1;
            let end = element.find("</text>")?;
            Some(element[start..end].to_owned())
        })
        .collect()
}

fn write_mask(source: &Path, path: &Path, keep: impl Fn(usize) -> bool) -> usize {
    let events = cytiq_fcs::FcsReader::open(source)
        .and_then(cytiq_fcs::FcsReader::read)
        .expect("fixture should be readable")
        .event_count();
    let keep = (0..events).map(keep).collect::<Vec<_>>();
    let removed = keep.iter().filter(|kept| !**kept).count();
    EventMask::new(common::acquisition_of(source), keep, None)
        .write_parquet(path)
        .expect("mask should be written");
    removed
}

/// Compensated and transformed pipeline sample, as `cytiq gate` and
/// `cytiq flowsom` read it.
fn transformed_pipeline_sample(directory: &Path) -> PathBuf {
    let compensated = directory.join("s.comp.fcs");
    let transformed = directory.join("s.t.fcs");
    run(&[
        "compensate",
        "--input",
        utf8(&fixture("pipeline", "sample.fcs")),
        "--output",
        utf8(&compensated),
    ]);
    run(&[
        "transform",
        "--input",
        utf8(&compensated),
        "--output",
        utf8(&transformed),
    ]);
    transformed
}

#[test]
fn plots_density_of_raw_scatter_channels() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let output = directory.path().join("fsc_ssc.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    assert!(svg.contains("<title>sample: SSC-A vs FSC-A</title>"));
    assert!(svg.contains("density · 10,000 events shown"));
    assert!(svg.contains(r#"shape-rendering="crispEdges""#));
    assert!(svg.contains(">100K</text>"), "linear scatter axis in K");
    common::assert_provenance(
        &directory.path().join("fsc_ssc.svg.provenance.json"),
        "cytiq plot",
        &[&input],
        &[&output],
    );
}

#[test]
fn overlays_events_removed_by_the_mask() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let mask = directory.path().join("analysis.parquet");
    let removed = write_mask(&input, &mask, |event| event % 3 != 0);
    let output = directory.path().join("scatter.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--mask",
        utf8(&mask),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--type",
        "scatter",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    assert_eq!(removed, 3334);
    assert!(
        svg.contains("removed by mask (3,334)"),
        "legend counts removed events"
    );
    assert!(svg.contains("6,666 events shown"));
    assert!(
        svg.contains(r##"<path fill="#E15759""##),
        "removed events are drawn"
    );
    common::assert_provenance(
        &directory.path().join("scatter.svg.provenance.json"),
        "cytiq plot",
        &[&input, &mask],
        &[&output],
    );

    let hidden = directory.path().join("hidden.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--mask",
        utf8(&mask),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--removed",
        "hide",
        "--output",
        utf8(&hidden),
    ]);
    let hidden = read_svg(&hidden);
    assert!(!hidden.contains("#E15759"));

    // Axes come from every event of the file, so masked and unmasked plots
    // of one file share them (same frame: no legend without the overlay).
    let unmasked = directory.path().join("unmasked.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&unmasked),
    ]);
    let ticks = tick_labels(&hidden);
    assert_eq!(ticks.len(), 9, "{ticks:?}");
    assert_eq!(ticks, tick_labels(&read_svg(&unmasked)));
}

#[test]
fn plots_when_the_mask_removes_every_event() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let mask = directory.path().join("failed.parquet");
    write_mask(&input, &mask, |_| false);
    let output = directory.path().join("failed.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--mask",
        utf8(&mask),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    assert!(svg.contains("no events left after the mask"));
    assert!(svg.contains("removed by mask (10,000)"));
}

#[test]
fn gate_labels_match_cytiq_gate_percent_of_parent() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let gates = fixture("gating", "gates.xml");
    let counts = directory.path().join("counts.json");
    run(&[
        "gate",
        "--input",
        utf8(&input),
        "--gating-ml",
        utf8(&gates),
        "--output",
        utf8(&counts),
        "--format",
        "json",
    ]);
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&counts).unwrap()).unwrap();
    let percent = |gate: &str| {
        report
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["gate_id"] == gate)
            .and_then(|row| row["percent_of_parent"].as_f64())
            .unwrap_or_else(|| panic!("{gate} in the counts"))
    };
    for (gate, extra) in [
        ("Cells", &["--type", "scatter"][..]),
        ("CD4_CD8_ellipse", &["--population", "CD3pos"][..]),
        ("Bowtie", &["--x", "FL2-A", "--y", "FL3-A"][..]),
        ("Singlets", &[][..]),
    ] {
        let output = directory.path().join(format!("{gate}.svg"));
        let mut args = vec![
            "plot",
            "--input",
            utf8(&input),
            "--gating-ml",
            utf8(&gates),
            "--gate",
            gate,
            "--output",
            utf8(&output),
        ];
        args.extend_from_slice(extra);
        run(&args);
        let svg = read_svg(&output);
        let label = format!("{gate} {:.1}%", percent(gate));
        assert!(svg.contains(&label), "{gate}: expected `{label}`");
    }
}

#[test]
fn removed_events_are_drawn_over_the_density() {
    // Under the density cells, removed events that share a cell with kept
    // ones would not show at all (most of them, on real samples).
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let mask = directory.path().join("analysis.parquet");
    write_mask(&input, &mask, |event| event % 3 != 0);
    let output = directory.path().join("density.svg");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--mask",
        utf8(&mask),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    let density = svg
        .find(r#"shape-rendering="crispEdges""#)
        .expect("density cells");
    let removed = svg
        .find(r##"<path fill="#E15759""##)
        .expect("removed events");
    assert!(
        removed > density,
        "removed events come after the density cells"
    );
}

/// `(x, y, text)` of every gate label, in document order.
fn gate_labels(svg: &str) -> Vec<(f64, f64, String)> {
    let attribute = |element: &str, name: &str| -> f64 {
        let start = element.find(&format!("{name}=\"")).expect("attribute") + name.len() + 2;
        let end = start + element[start..].find('"').expect("closing quote");
        element[start..end].parse().expect("number")
    };
    svg.split("<text ")
        .filter(|element| element.contains(r##"font-weight="600" fill="#1F2933" stroke"##))
        .map(|element| {
            let start = element.find('>').expect("text start") + 1;
            let end = element.find("</text>").expect("text end");
            (
                attribute(element, "x"),
                attribute(element, "y"),
                element[start..end].to_owned(),
            )
        })
        .collect()
}

#[test]
fn complementary_range_gates_are_labelled_on_their_own_side() {
    // `T` (CD3 ≥ 3) and `NonT` (CD3 < 3) draw one line; their labels used to
    // share one spot right of it, `NonT` covering `T` on the CD3⁺ side.
    let directory = tempfile::tempdir().expect("temporary directory");
    let transformed = transformed_pipeline_sample(directory.path());
    let output = directory.path().join("cd3.svg");
    run(&[
        "plot",
        "--input",
        utf8(&transformed),
        "--gating-ml",
        utf8(&fixture("pipeline", "gates.xml")),
        "--x",
        "FL1-A",
        "--y",
        "SSC-A",
        "--population",
        "Lymphocytes",
        "--gate",
        "T",
        "--gate",
        "NonT",
        "--output",
        utf8(&output),
    ]);
    let labels = gate_labels(&read_svg(&output));
    let find = |gate: &str| {
        labels
            .iter()
            .find(|(_, _, text)| text.starts_with(&format!("{gate} ")))
            .map_or_else(
                || panic!("{gate} label in {labels:?}"),
                |(x, y, _)| (*x, *y),
            )
    };
    let (t_x, t_y) = find("T");
    let (non_t_x, non_t_y) = find("NonT");
    assert!(
        non_t_x < t_x,
        "NonT on the CD3⁻ side, left of T: {labels:?}"
    );
    // Default frame top is 62 px: 11 px glyphs on a baseline at least 12 px
    // below it stay clear of the frame line.
    assert!(t_y >= 74.0 && non_t_y >= 74.0, "{labels:?}");
}

#[test]
fn rejects_gates_that_cannot_be_drawn_on_the_plot() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let gates = fixture("gating", "gates.xml");
    let output = directory.path().join("plot.svg");
    let base = [
        "plot",
        "--input",
        utf8(&input),
        "--gating-ml",
        utf8(&gates),
        "--output",
        utf8(&output),
    ];
    let boolean = fails(&[&base[..], &["--gate", "CD3neg"]].concat());
    assert!(boolean.contains("Boolean"), "{boolean}");
    let other = fails(
        &[
            &base[..],
            &["--x", "FSC-A", "--y", "SSC-A", "--gate", "CD3pos"],
        ]
        .concat(),
    );
    assert!(other.contains("not an axis of this plot"), "{other}");
    let unknown = fails(&[&base[..], &["--gate", "Nope"]].concat());
    assert!(
        unknown.contains("`Nope` is not in the Gating-ML file"),
        "{unknown}"
    );
    let range_only = fails(&[&base[..], &["--gate", "CD3pos"]].concat());
    assert!(range_only.contains("no y axis"), "{range_only}");
    assert!(!output.exists(), "a failed plot publishes nothing");
}

#[test]
fn plots_a_population_of_the_transformed_pipeline_sample() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let transformed = transformed_pipeline_sample(directory.path());
    let gates = fixture("pipeline", "gates.xml");
    let output = directory.path().join("t_cd4_cd8.svg");
    run(&[
        "plot",
        "--input",
        utf8(&transformed),
        "--gating-ml",
        utf8(&gates),
        "--population",
        "T",
        "--gate",
        "CD4T",
        "--gate",
        "CD8T",
        "--type",
        "contour",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    assert!(svg.contains("population T"));
    assert!(svg.contains("CD4T "), "gate label");
    assert!(
        svg.contains(">10³</text>"),
        "raw decades on the arcsinh axis"
    );
    assert!(
        svg.contains(r##"<path fill="none" stroke="#"##),
        "contour lines"
    );

    let logicle = fails(&[
        "plot",
        "--input",
        utf8(&transformed),
        "--x",
        "CD4",
        "--y",
        "CD8",
        "--x-scale",
        "logicle",
        "--output",
        utf8(&directory.path().join("twice.svg")),
    ]);
    assert!(
        logicle.contains("already holds transformed values"),
        "{logicle}"
    );
}

#[test]
fn colours_events_by_flowsom_metacluster() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let transformed = transformed_pipeline_sample(directory.path());
    let clusters = directory.path().join("flowsom.csv");
    run(&[
        "flowsom",
        "--input",
        utf8(&transformed),
        "--output",
        utf8(&clusters),
        "--channels",
        FLUORESCENCE,
        "--metaclusters",
        "7",
    ]);
    let output = directory.path().join("clusters.svg");
    run(&[
        "plot",
        "--input",
        utf8(&transformed),
        "--color-by",
        utf8(&clusters),
        "--x",
        "CD3",
        "--y",
        "CD19",
        "--type",
        "scatter",
        "--output",
        utf8(&output),
    ]);
    let svg = read_svg(&output);
    assert!(svg.contains("coloured by flowsom_metacluster"));
    assert!(svg.contains(">1 ("), "legend lists metacluster 1");
    common::assert_provenance(
        &directory.path().join("clusters.svg.provenance.json"),
        "cytiq plot",
        &[&transformed, &clusters],
        &[&output],
    );
    let density = fails(&[
        "plot",
        "--input",
        utf8(&transformed),
        "--color-by",
        utf8(&clusters),
        "--x",
        "CD3",
        "--y",
        "CD19",
        "--output",
        utf8(&directory.path().join("density.svg")),
    ]);
    assert!(density.contains("needs the `scatter` type"), "{density}");
}

fn write_predictions(path: &Path, acquisition: &str, events: u64) {
    let mut labels = StringBuilder::new();
    for event in 0..events {
        match event % 3 {
            0 => labels.append_value("blast"),
            1 => labels.append_value("non_blast"),
            _ => labels.append_null(),
        }
    }
    let batch = RecordBatch::try_from_iter([
        (
            "source_event_id",
            Arc::new(UInt64Array::from_iter_values(0..events)) as ArrayRef,
        ),
        ("label", Arc::new(labels.finish()) as ArrayRef),
    ])
    .expect("record batch");
    let properties = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![
            KeyValue::new(
                "cytiq.acquisition_sha256".to_owned(),
                acquisition.to_owned(),
            ),
            KeyValue::new(
                "cytiq.classify.classes".to_owned(),
                r#"["non_blast","blast"]"#.to_owned(),
            ),
        ]))
        .build();
    let mut writer = ArrowWriter::try_new(
        File::create(path).expect("predictions file"),
        batch.schema(),
        Some(properties),
    )
    .expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

#[test]
fn colours_events_by_classify_predictions_of_the_same_acquisition() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let predictions = directory.path().join("predictions.parquet");
    write_predictions(&predictions, &common::acquisition_of(&input), 10_000);
    let output = directory.path().join("classes.svg");
    let args = [
        "plot",
        "--input",
        utf8(&input),
        "--color-by",
        utf8(&predictions),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--type",
        "scatter",
        "--output",
        utf8(&output),
    ];
    run(&args);
    let svg = read_svg(&output);
    assert!(
        svg.contains("non_blast (3,333)"),
        "legend in declared class order"
    );
    assert!(svg.contains("blast (3,334)"));
    assert!(svg.contains("unlabelled (3,333)"));

    write_predictions(&predictions, &"0".repeat(64), 10_000);
    let mismatch = fails(&[&args[..], &["--overwrite"]].concat());
    assert!(mismatch.contains("acquisition"), "{mismatch}");
}

#[test]
fn batch_config_writes_every_plot_in_both_formats() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let gates = fixture("gating", "gates.xml");
    let config = directory.path().join("plots.yaml");
    fs::write(
        &config,
        "version: 1\ndefaults:\n  palette: viridis\nplots:\n  - { name: fsc_ssc, type: density, x: FSC-A, y: SSC-A }\n  - { name: cells, type: contour, gates: [Cells] }\n",
    )
    .unwrap();
    let output = directory.path().join("plots");
    run(&[
        "plot",
        "--input",
        utf8(&input),
        "--gating-ml",
        utf8(&gates),
        "--config",
        utf8(&config),
        "--format",
        "svg",
        "--format",
        "plotly",
        "--output",
        utf8(&output),
    ]);
    let outputs = [
        "fsc_ssc.svg",
        "fsc_ssc.plotly.json",
        "cells.svg",
        "cells.plotly.json",
    ]
    .map(|name| output.join(name));
    read_svg(&outputs[0]);
    assert!(read_svg(&outputs[2]).contains("Cells "));
    let figure: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&outputs[1]).unwrap()).unwrap();
    assert_eq!(figure["data"][0]["type"], "heatmap");
    assert_eq!(figure["layout"]["xaxis"]["tickmode"], "array");
    common::assert_provenance(
        &output.join("provenance.json"),
        "cytiq plot",
        &[&input, &gates, &config],
        &outputs.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
    );

    let duplicate = directory.path().join("duplicate.yaml");
    fs::write(
        &duplicate,
        "version: 1\nplots:\n  - { name: a, type: density, x: FSC-A, y: SSC-A }\n  - { name: A, type: scatter, x: FSC-A, y: SSC-A }\n",
    )
    .unwrap();
    let error = fails(&[
        "plot",
        "--input",
        utf8(&input),
        "--config",
        utf8(&duplicate),
        "--output",
        utf8(&directory.path().join("other")),
    ]);
    assert!(error.contains("used twice"), "{error}");
}

#[test]
fn output_is_byte_identical_across_runs() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let gates = fixture("gating", "gates.xml");
    // Same file name in two directories: the plot name (and so the SVG ids)
    // comes from the file stem.
    let render = |run_directory: &str| {
        let output = directory.path().join(run_directory).join("plot.svg");
        run(&[
            "plot",
            "--input",
            utf8(&input),
            "--gating-ml",
            utf8(&gates),
            "--gate",
            "CD4_CD8_ellipse",
            "--population",
            "CD3pos",
            "--type",
            "contour",
            "--output",
            utf8(&output),
        ]);
        fs::read(output).unwrap()
    };
    assert_eq!(render("first"), render("second"));
}

#[test]
fn refuses_an_existing_output_without_overwrite() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let output = directory.path().join("plot.svg");
    let args = [
        "plot",
        "--input",
        utf8(&input),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ];
    run(&args);
    let error = fails(&args);
    assert!(error.contains("--overwrite"), "{error}");
    run(&[&args[..], &["--overwrite"]].concat());
}

#[test]
fn input_errors_name_the_problem() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("clean", "sample.fcs");
    let output = directory.path().join("plot.svg");
    let missing = fails(&[
        "plot",
        "--input",
        utf8(&directory.path().join("missing.fcs")),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    assert!(missing.contains("missing.fcs"), "{missing}");

    let channel = fails(&[
        "plot",
        "--input",
        utf8(&input),
        "--x",
        "FOO",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    assert!(
        channel.contains("`FOO`") && channel.contains("FSC-A"),
        "{channel}"
    );

    let mask = directory.path().join("control.parquet");
    write_mask(&fixture("clean", "control.fcs"), &mask, |_| true);
    let other = fails(&[
        "plot",
        "--input",
        utf8(&input),
        "--mask",
        utf8(&mask),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--output",
        utf8(&output),
    ]);
    assert!(other.contains("acquisition"), "{other}");

    let formats = fails(&[
        "plot",
        "--input",
        utf8(&input),
        "--x",
        "FSC-A",
        "--y",
        "SSC-A",
        "--format",
        "svg",
        "--format",
        "plotly",
        "--output",
        utf8(&output),
    ]);
    assert!(formats.contains("--config"), "{formats}");
    assert!(!output.exists());
}

#[test]
fn invalid_flags_are_rejected_at_parse_time() {
    let input = fixture("clean", "sample.fcs");
    for extra in [
        &["--bins", "4"][..],
        &["--x-range", "5,1"][..],
        &["--contour-step", "0.9"][..],
        &["--width", "10"][..],
        &["--config", "plots.yaml", "--type", "scatter"][..],
        &["--gate", "Cells"][..],
    ] {
        let output = cytiq(
            &[
                &["plot", "--input", utf8(&input), "--output", "plot.svg"][..],
                extra,
            ]
            .concat(),
        );
        assert_eq!(output.status.code(), Some(2), "{extra:?}");
    }
}
