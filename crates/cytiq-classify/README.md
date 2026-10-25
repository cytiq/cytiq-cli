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

ONNX Runtime is shipped in `lib/` beside release executables. Developers can set
`ORT_DYLIB_PATH` to a compatible ONNX Runtime 1.25.1 shared library. No Python or
network access is required for inference. Test using `cargo test -p cytiq-classify`
and `cargo test -p cytiq-cli --test classify_cli`.
