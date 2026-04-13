# justfile
# -----------------------------------------------------------------------------
# Shared development commands for the cytiq Rust application.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# ---- Project configuration

CLI_PACKAGE := env("CLI_PACKAGE", "cytiq-cli")
DOCKER_IMAGE := env("DOCKER_IMAGE", "cytiq:dev")
DOCKERFILE := env("DOCKERFILE", "docker/Dockerfile")

# ---- Run the application
[group('build')]
run *args:
    cargo run -p {{ CLI_PACKAGE }} -- {{ args }}

# ---- Build the project in debug mode
[group('build')]
build:
    cargo build --workspace

# ---- Build the project in release mode
[group('build')]
build-release:
    cargo build --workspace --release

# ---- Check the project without producing a binary
[group('build')]
check:
    cargo check --workspace

# ---- Run the test suite
[group('quality')]
test:
    cargo test --workspace

# ---- Format the codebase
[group('quality')]
fmt:
    cargo fmt --all

# ---- Run clippy with warnings treated as errors
[group('quality')]
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# ---- Run the standard local quality gate
[group('quality')]
verify:
    just fmt
    just check
    just lint
    just test

# ---- Build the Docker image
[group('docker')]
docker-build:
    docker build -f {{ DOCKERFILE }} -t {{ DOCKER_IMAGE }} .

# ---- Remove Rust build artifacts
[group('utils')]
clean:
    cargo clean

# ---- Show available recipes
help:
    @just --list
