# Contributing to cytiq-manifest

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — manifest types, `load`, `validate`; unit tests.

## Invariants

- The manifest format is a user-facing contract documented in the root
  `README.md` (`## Preflight`) and this crate's README. Additions must stay
  optional (`#[serde(default)]`) so existing manifests keep loading.
- YAML and JSON describe the same document; tests load both.
- Every struct uses `deny_unknown_fields`, so typos fail loudly.
- No file I/O beyond reading the manifest: checking FCS files belongs in
  `cytiq-preflight`.

## Testing

```bash
cargo test -p cytiq-manifest
```
