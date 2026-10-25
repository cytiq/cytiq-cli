# cytiq-classify

Keep the CLI thin. Validate the bundle and complete preprocessing before writing
outputs. Reuse FCS signal state and mask lineage. Never apply compensation twice.
`features.rs` implements the Python recipe; keep golden parity tests in sync.
Use `ort`'s `download-binaries` feature and static linking. Keep the pinned
crate/runtime versions and upstream attribution aligned; do not add a custom
binary downloader. Run `cargo test -p cytiq-classify` and the classify CLI tests
without runtime environment variables. The relocation test must run the binary
alone, independently of the checkout and current directory.
