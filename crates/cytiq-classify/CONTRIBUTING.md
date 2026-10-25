# cytiq-classify

Keep the CLI thin. Validate the bundle and complete preprocessing before writing
outputs. Reuse FCS signal state and mask lineage. Never apply compensation twice.
`features.rs` implements the Python recipe; keep golden parity tests in sync.
Run `cargo test -p cytiq-classify` and the classify CLI tests with ONNX Runtime
available via `ORT_DYLIB_PATH` or beside the binary in `lib/`.
