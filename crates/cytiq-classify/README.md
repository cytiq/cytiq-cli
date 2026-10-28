# cytiq-classify

Panel-specific supervised classification using a Cytiq model bundle v1 or v2 and CPU
ONNX Runtime. `ClassifyRunner::run(&ClassifyConfig)` reads an FCS, checks marker
labels and instrument, applies the declared preprocessing, and writes
`predictions.parquet` plus `summary.json`. The CLI adds `provenance.json`.

The first profile is FACSCanto II P1, reproducing expert Blast annotations in
BLAST110. The Blast phenotype does not imply a leukemic/LAIP-positive cell.

```sh
cytiq classify --input sample.fcs --model bundle/ --output results/
```

The bundle contains `model.onnx` and a manifest declaring instrument,
channels/marker aliases, preprocessing, class order, checksum and threshold.
The runner validates the panel and model hash, linearises raw FCS signals,
compensates from the spillover matrix and applies fluorescence arcsinh(x/150).
Compatible Cytiq inputs skip steps already recorded; incompatible transforms
are errors.

The optional `--mask` flags use cytiq-mask acquisition identity. Every input
event remains in the output; excluded events have null probabilities and labels.
`--abstain-margin 0.05` labels events within 0.05 of the saved threshold as
`unknown`. It is disabled by default and is not an out-of-distribution detector.

Raw conventional FCS and compatible Cytiq compensated/arcsinh FCS are supported.
Third-party processed FCS without verifiable Cytiq signal state are outside the
contract. Files without Cytiq state are interpreted as raw, as declared by the
dataset profile. A model is only supported for its declared instrument and panel.

`predictions.parquet` holds 0-based `event_id` and `source_event_id`, `assessed`,
nullable `p_non_blast`, `p_blast` and `label`. Its metadata records acquisition
identity and schema version. Labels default to blast/non_blast; the abstain
band can add unknown. `summary.json` reports counts and Blast fraction among
assessed events; `provenance.json` records model, input and mask hashes.
Existing outputs require `--overwrite`. `--batch-size` defaults to 65536;
`--threads` caps ONNX CPU threads.

ONNX Runtime 1.31.0 is linked statically by `ort` 2.0.0-rc.15 using its
`download-binaries` feature. The crate handles downloading, SHA-256 verification
and caching the prebuilt library during Cargo builds; no project download script
is needed. The first build needs HTTPS access to the upstream binary CDN.

The built executable runs without Python, runtime environment variables or a
separate ONNX Runtime installation. Moving `cytiq` alone is sufficient on the
supported macOS Apple Silicon and Linux x86_64/aarch64 targets; standard OS
libraries remain required. Test using `cargo test -p cytiq-classify` and
`cargo test -p cytiq-cli --test classify_cli`.

## Vocabulary-aware bundle v2

The same `cytiq classify` command also reads v2 bundles from `cytiq-ml`. Each bundle
adds `classes.json`, `panel.json` and `label-mapping.json`, pinned by SHA-256. The
runner checks vocabulary indices, task and decision rule, hierarchy references,
residual definitions, panel agreement and dense float32 `[N,K]` probabilities.
The CLI records all knowledge files in provenance. No Python runtime is needed.

Multiclass uses vocabulary order and argmax; ties choose the lowest index. Its
unknown policy is `disabled`. The learned `other` residual class has a declared
population scope and exclusions. `--abstain-margin` is only supported for binary
tasks. A binary phenotype v2 task retains the saved threshold and abstain band.
Existing v1 bundle outputs retain their original columns and summary.

V2 Parquet has `event_id`, `source_event_id`, `assessed`, one nullable `p_<class_id>`
column per ordered class, then `label`. Metadata pins class order and vocabulary
hash. Summary reports `class_counts`, `class_fractions`, task and vocabulary hash.
Excluded events retain their rows with null predictions. Multiclass summaries
omit the binary-specific Blast fields.

The `cytiq-flowcyt-stored-v1` profile reads the stored measurements in original
FlowCyt author exports. It applies no compensation, linearisation or arcsinh and
rejects inputs already carrying Cytiq processing state. The instrument, detector
names and reagent labels must match the pinned profile. This profile is scoped to
the published nonmalignant bone marrow cohort; it does not assert that the same
lineage definitions hold for AML/P1 or unrelated processed FCS files.

`ClassifyRunner` retains one ONNX environment for the process, allowing repeated
or concurrent library calls. Test both v1 and v2 with `cargo test -p cytiq-classify`
and `cargo test -p cytiq-cli --test classify_cli`.
