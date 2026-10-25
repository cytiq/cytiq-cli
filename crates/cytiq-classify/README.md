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

`cargo build` and `cargo test` automatically download the official ONNX Runtime
1.25.1 CPU archive, verify its pinned SHA-256 and stage the runtime and licenses
in `target/<profile>/lib/`. Supported targets are macOS Apple Silicon and Linux
x86_64/aarch64 with glibc. The first build needs HTTPS access to GitHub Releases;
subsequent builds reuse the verified archive in Cargo's build directory.

The runtime is found relative to the executable, independent of the working
directory. No environment variables, Python or network access are required for
inference. When moving a binary, copy its `lib/` directory with it. The optional
`ORT_DYLIB_PATH` override remains available for custom runtime installations.
Test using `cargo test -p cytiq-classify` and
`cargo test -p cytiq-cli --test classify_cli`.
