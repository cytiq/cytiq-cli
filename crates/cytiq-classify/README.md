# cytiq-classify

Panel-specific supervised classification using a Cytiq model bundle v1 and CPU
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
