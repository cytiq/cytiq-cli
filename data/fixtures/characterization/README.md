# Characterization digests

SHA-256 digests that pin the exact output of code paths about to be
refactored, so a refactor can prove it changes nothing.

- `fcs.json`: for every `*.fcs` under `data/fixtures`, the digest of what
  `FcsReader::read` returns (version, HEADER, TEXT pairs, parameters, ANALYSIS
  and every column value by bit pattern), or the error message. Checked by
  `crates/cytiq-fcs/tests/characterization.rs`.
- `classify.json`: for each classify fixture bundle, with and without a mask
  that drops every third event, a digest of every `predictions.parquet` value
  except the probabilities (ids, labels, schema, where values are null) plus
  `summary.json`, and the probabilities themselves. The digest must match
  exactly and the probabilities within 1e-6: ONNX Runtime picks its kernels by
  CPU, so probabilities differ between machines in the last bits (macOS arm64
  and Linux x86_64 CI do). Within one run the test also requires bit-identical
  outputs at batch sizes 1, 17 and 65,536. Checked by
  `crates/cytiq-classify/tests/characterization.rs`.

A digest change means the output changed. When the change is intended, rerun
the test with `CYTIQ_BLESS=1` and explain the change in the pull request.
Never re-bless to make a refactor pass.
