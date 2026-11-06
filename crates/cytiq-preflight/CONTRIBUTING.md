# Contributing to cytiq-preflight

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/checks.rs` — one function per check; `Check`, `CheckStatus`.
- `src/runner.rs` — `PreflightRunner`, the report, `ensure_passed`.
- `src/error.rs` — `PreflightError` (`thiserror`).

## Invariants

- Preflight only reads. It never modifies or writes FCS files.
- Every check reports `pass`, `warn` or `fail` with a message a user can act
  on; never drop a finding silently. The check table in this crate's README is
  the contract: update it with every new or changed check.
- The report is written even when checks fail; the CLI turns failures into
  exit code 3 (`checks_failed`) through `ensure_passed` and
  `PreflightError::Failed`.
- Reuse `cytiq-fcs` for parsing, signal state and spillover handling instead
  of re-implementing them here.

## Testing

```bash
cargo test -p cytiq-engine --test preflight_cli
```
