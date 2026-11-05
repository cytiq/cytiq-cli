# Contributing to cytiq-gating

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/model.rs` — `GatingStrategy`, `Gate`, `GateKind`, `Dimension`,
  `CompensationRef`, `SpectrumMatrix`.
- `src/parse.rs` — Gating-ML 2.0 XML parser (`roxmltree`) and
  `build_strategy`: reference validation and topological ordering.
- `src/write.rs` — `to_gatingml`, the inverse of the parser.
- `src/evaluate.rs` — `gate_sample` / `gate_sample_masked`, the signal-state
  guard, compensation and transform caching, `GatingResults`.
- `src/geometry.rs` — rectangle, polygon and ellipsoid membership predicates.
- `src/transforms.rs` — `Transform` (`flin`, `flog`, `fasinh`, `logicle`, `hyperlog`).
- `src/runner.rs` — `GatingRunner`: the `cytiq gate` command, event masks,
  CSV/JSON counts and the membership Parquet.
- `src/error.rs` — `GatingError` (`thiserror`).

## Invariants

- Per-event membership must stay identical to FlowKit 1.3.0 on
  `data/fixtures/gating/`. Boundary rules in `geometry.rs` are deliberate:
  rectangles `min <= x < max`, polygons odd winding number (left/bottom edges
  inside, right/top outside), ellipsoids inclusive. Do not "fix" them.
- Every gate is ANDed with its parent after evaluation.
- `GatingStrategy` keeps gates in evaluation order: each gate after its parent
  and after every gate it references. Ordering is Kahn's algorithm with ties in
  document order. Construction (`parse_gatingml`, `GatingStrategy::new`) must
  reject duplicate ids, dangling references and cycles.
- Raw pre-processing (`$PnE`, `$PnG`, `$TIMESTEP` on `Time`) happens before
  compensation, as in FlowKit. It lives in `cytiq_fcs::linear_values`, shared
  with `cytiq-compensate`, so both compensate identically.
- An event mask is ANDed into every gate's membership before the parent
  intersection, so complements (`not`) never include masked events.
- `to_gatingml` is deterministic (transforms and matrices sorted by id, gates
  in evaluation order) and uses shortest round-trip number formatting, so
  `parse_gatingml(to_gatingml(s))` yields the same strategy. `cytiq-clean`
  relies on this to make its exported gates reproduce its masks.
- Unsupported elements fail loudly (`UnsupportedElement`); never skip them
  silently.

## Where changes belong

- `Logicle` / `Hyperlog` math lives in `cytiq-transform::scales`.
- FCS parsing and `$SPILLOVER` decoding belong in `cytiq-fcs`.
- Gate estimation from data does not belong here; fitted gates live in
  consumers such as `cytiq-clean`.
- CLI flags belong in `crates/cytiq-engine/src/commands/gate.rs`. Keep this crate free of `clap`.

## Testing

```bash
cargo test -p cytiq-gating
cargo test -p cytiq-gating --test flowkit_parity
cargo test -p cytiq-engine --test gate_cli
```

Unit tests sit next to the code in `geometry.rs`, `parse.rs`, `transforms.rs`
and `write.rs`. `tests/parse_transforms.rs` checks transformation parameters,
their boundaries and duplicate or reserved ids through `parse_gatingml`. When
you change semantics or add a gate/transform type, extend
`parity/gen/gating.py`, regenerate `data/fixtures/gating/` (command in its
README) and keep `tests/flowkit_parity.rs` green.
