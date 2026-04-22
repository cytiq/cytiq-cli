# Contributing

## Project Scope

This repository is a small Rust workspace named `cytiq`. Keep contributor guidance specific to this codebase. Do not copy workflow, database, Python, or GitHub automation instructions from other projects unless those tools are added here.

## Project Structure

- `crates/cytiq-cli/` contains the CLI binary entry point.
- `data/` contains checked-in sample and research assets such as `data/data.fcs` and `data/CD22/`.
- `docker/Dockerfile` is the place for container build logic.

The repository should consist of crates only. Do not add a root-level `src/`; new binaries or libraries belong under `crates/`.

## Development Commands

Use `just` as the primary command runner. The recipes wrap the same Cargo commands used in CI.

- `just run` starts the `cytiq-cli` binary.
- `just build` builds the workspace in debug mode.
- `just build-release` builds an optimized release binary.
- `just check` runs `cargo check --workspace`.
- `just fmt` formats the codebase with `rustfmt`.
- `just lint` runs Clippy with warnings denied.
- `just test` runs the workspace test suite.
- `just verify` runs the standard local quality gate.
- `just docker-build` verifies container changes.

Before opening a pull request, run `just verify`. CI enforces the equivalent checks with `cargo check --workspace --locked`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --locked`.

## Coding Style

Follow Rust 2024 conventions and let `rustfmt` control formatting. Use:

- 4-space indentation
- `snake_case` for functions and modules
- `PascalCase` for structs, enums, and traits
- `SCREAMING_SNAKE_CASE` for constants

Keep `cytiq-cli` thin and move reusable logic into a focused library crate. Add brief comments only where the intent is not obvious from the code.

## Testing

The repository does not yet include tests, but new logic should include them. Put unit tests near the implementation with `#[cfg(test)]`, and place broader scenarios in `tests/`. Name tests by observable behavior, for example `loads_cd22_sample_data` or `fails_when_input_is_missing`.

## Commits and Pull Requests

Git history currently contains only `Initial commit`, so keep commit messages short, imperative, and scoped. Examples: `Add FCS file loader`, `Refactor input parsing`.

Pull requests should include:

- a short summary of the change
- the reason for the change
- the commands used for verification
- notes about any updates to `data/`

## Data Handling

Treat `data/` as intentional repository content. Do not commit secrets, temporary exports, or generated artifacts unless they are required for reproducible development or tests.
