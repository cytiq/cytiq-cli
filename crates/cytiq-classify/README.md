# cytiq-classify

Panel-specific supervised classification using a Cytiq model bundle v1 and CPU
ONNX Runtime. `ClassifyRunner::run(&ClassifyConfig)` reads an FCS, checks marker
labels and instrument, applies the declared preprocessing, and writes
`predictions.parquet` plus `summary.json`. The CLI adds `provenance.json`.

```sh
cytiq classify --input sample.fcs --model bundle/ --output results/
```

The optional `--mask` flags use cytiq-mask acquisition identity. Every input
event remains in the output; excluded events have null probabilities and labels.
`--abstain-margin 0.05` labels events within 0.05 of the saved threshold as
`unknown`. It is disabled by default and is not an out-of-distribution detector.

Raw conventional FCS and compatible Cytiq compensated/arcsinh FCS are supported.
Third-party processed FCS without verifiable Cytiq signal state are outside the
contract. Files without Cytiq state are interpreted as raw, as declared by the
dataset profile. A model is only supported for its declared instrument and panel.

ONNX Runtime 1.31.0 is linked statically by `ort` 2.0.0-rc.15 using its
`download-binaries` feature. The crate handles downloading, SHA-256 verification
and caching the prebuilt library during Cargo builds; no project download script
is needed. The first build needs HTTPS access to the upstream binary CDN.

The built executable runs without Python, runtime environment variables or a
separate ONNX Runtime installation. Moving `cytiq` alone is sufficient on the
supported macOS Apple Silicon and Linux x86_64/aarch64 targets; standard OS
libraries remain required. Test using `cargo test -p cytiq-classify` and
`cargo test -p cytiq-cli --test classify_cli`.
