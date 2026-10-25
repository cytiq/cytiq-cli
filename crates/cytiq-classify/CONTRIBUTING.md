# cytiq-classify

Keep the CLI thin. Validate the bundle and complete preprocessing before writing
outputs. Reuse FCS signal state and mask lineage. Never apply compensation twice.
`features.rs` implements the Python recipe; keep golden parity tests in sync.
`build.rs` verifies and stages the pinned CPU runtime and licenses in Cargo's
profile `lib/` directory. Keep the release version, upstream attribution and
archive SHA-256 values aligned. Run `cargo test -p cytiq-classify` and the
classify CLI tests without `ORT_DYLIB_PATH`; relocation tests must find the
staged runtime independently of the checkout and current directory.
