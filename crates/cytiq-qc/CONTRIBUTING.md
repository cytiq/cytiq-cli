# Contributing to cytiq-qc

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/policy.rs` — `QcMetrics` from an `EventMask`, `evaluate` (the policy
  rules), `QcCheck`, `CheckStatus`; unit tests on synthetic masks.
- `src/runner.rs` — `QcRunner`: manifest and sample lookup, acquisition and
  coverage checks, the report.
- `src/error.rs` — `QcError` (`thiserror`).
- `src/preflight/` — run-manifest and FCS checks, their runner and error enum;
  exposed only with feature `preflight`. Its check table lives in
  [the preflight guide](docs/preflight.md).

## Invariants

- QC reads masks, the contract every stage writes. Do not parse stage
  reports (`metrics.json`, `summary.json`) here; a metric a stage does not
  express as a mask column belongs in that stage's mask first.
- The policy type lives in `cytiq-manifest` (`QcPolicy`, `FilterPolicy`,
  `Severity`); new rules are optional fields there, documented in the root
  README `## QC` and this crate's check table.
- Metrics are only computed when the mask belongs to the sample's
  acquisition and has exactly one row per raw event; otherwise the report
  says why and holds no metrics. Filter metrics count mask rows, so they are
  raw-event counts only under that rule.
- The report is written even when checks fail; the CLI turns failures into
  exit code 3 (`checks_failed`) through `ensure_passed` and `QcError::Failed`.
- Never modifies inputs.
- Preflight preserves every finding, including warnings, and writes a report
  before `PreflightError::Failed` produces CLI exit code 3. Reuse `cytiq-fcs`
  parsing and spillover handling; do not duplicate it here.
- Both workflows use the same `CheckStatus`, with `pass < warn < fail` and
  snake-case serialization. Their finding/report structures stay separate.
- Keep `cytiq-compensate` optional and gated by `preflight`. Standalone QC and
  specimen users must not gain that dependency without enabling the feature.

## Testing

```bash
cargo test -p cytiq-qc
cargo test -p cytiq-qc --features preflight
cargo test -p cytiq-engine --test preflight_cli
cargo test -p cytiq-engine --test qc_cli
```
