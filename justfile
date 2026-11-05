# justfile
# -----------------------------------------------------------------------------
# Shared development commands for the cytiq Rust application.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# ---- Project configuration

CLI_PACKAGE := env("CLI_PACKAGE", "cytiq-cli")
DOCKER_IMAGE := env("DOCKER_IMAGE", "cytiq:dev")
DOCKERFILE := env("DOCKERFILE", "docker/Dockerfile")

# ---- Install the pinned upstream R/Python tools into parity/.env
[group('validation')]
parity-setup:
    parity/setup_env.sh

# ---- Validation against upstream tools (needs `just parity-setup` once)
[group('validation')]
parity:
    parity/run_all.sh

# ---- Validate on real acquisitions shipped with the R packages (and CYTIQ_REAL_DATA)
[group('validation')]
parity-real:
    parity/real/run_real.sh

# ---- Validate CytoNorm quantiles/splines and full clustering against pinned R
[group('validation')]
parity-cytonorm:
    cargo build -p cytiq-cli --locked
    R_LIBS_USER=parity/.env/Rlib Rscript parity/cytonorm.R target/debug/cytiq

# ---- Validate CytoNorm on six real CyTOF controls with a held-out donor
[group('validation')]
parity-cytonorm-real:
    cargo build -p cytiq-cli --locked
    R_LIBS_USER=parity/.env/Rlib Rscript parity/cytonorm_real.R target/debug/cytiq

# ---- Regenerate the independent R numerical fixtures for CytoNorm
[group('validation')]
parity-cytonorm-fixture:
    R_LIBS_USER=parity/.env/Rlib Rscript parity/cytonorm_numeric.R

# ---- Benchmark spillqc detection on real data (needs `just parity-real` outputs)
[group('validation')]
benchmark-spillqc:
    parity/benchmark/run_spillqc.sh

# ---- Paired algorithm performance measurements (build both binaries and the helper first)
[group('validation')]
performance *args:
    python3 performance/run.py {{ args }}

# ---- Fast benchmark-runner checks; no performance workloads
[group('quality')]
performance-check:
    python3 -m unittest discover -s performance -p 'test_*.py'
    cargo test -p cytiq-cli --example performance_support --locked

# ---- Regenerate the golden end-to-end CI fixture
[group('validation')]
parity-ci-fixture:
    parity/.env/py/bin/python parity/gen/pipeline.py data/fixtures/pipeline ci

# ---- Regenerate the CI parity smoke fixture: small datasets plus upstream R outputs
[group('validation')]
parity-smoke-fixture:
    parity/make_smoke.sh

# ---- Fuzz read_fcs (seeded from fixtures and $CYTIQ_REAL_DATA) or spillover_keyword; needs nightly and cargo-fuzz
# Saved regressions seed both targets. Seeds are named after their path, so
# fixtures that share a file name do not overwrite each other.
[group('validation')]
fuzz-fcs target="read_fcs" seconds="300":
    mkdir -p crates/cytiq-fcs/fuzz/corpus/{{ target }}
    for seed in crates/cytiq-fcs/tests/fuzz_regressions/{{ target }}/*; do \
      cp "$seed" "crates/cytiq-fcs/fuzz/corpus/{{ target }}/regression-$(basename "$seed")"; \
    done
    if [[ "{{ target }}" == read_fcs ]]; then \
      find data/fixtures "${CYTIQ_REAL_DATA:-data/fixtures}" -name '*.fcs' -size -4M -print0 | \
        while IFS= read -r -d '' seed; do \
          cp "$seed" "crates/cytiq-fcs/fuzz/corpus/read_fcs/seed-$(printf '%s' "$seed" | tr '/' '_')"; \
        done; \
    fi
    # cargo-fuzz otherwise builds for the triple it was built for, which is
    # musl (no sanitizers) for prebuilt Linux binaries.
    cd crates/cytiq-fcs && cargo +nightly fuzz run {{ target }} \
      --target "$(rustc +nightly -vV | sed -n 's/^host: //p')" \
      -- -max_total_time={{ seconds }} -timeout=10

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

# ---- Regenerate Cargo.lock from workspace manifests
[group('build')]
lock:
    cargo generate-lockfile

# ---- Run the test suite
[group('quality')]
test:
    cargo test --workspace

# ---- Run the CI test suite once with LLVM line coverage (see CONTRIBUTING.md)
[group('quality')]
coverage:
    bash .github/scripts/crap.sh coverage

# ---- Report CRAP and baseline deltas from an existing successful coverage run
[group('quality')]
crap:
    bash .github/scripts/crap.sh report

# ---- Test the report validation and summary without installing coverage tools
[group('quality')]
crap-check:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'

# ---- Mutation tests for the CRAP remediation functions (optionally one crate)
[group('quality')]
mutants crate="all":
    bash .github/scripts/mutants.sh {{ quote(crate) }}

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
    just performance-check
    just crap-check

# ---- Start Docker
[group('ci-cd')]
docker-up:
    @if command -v brew >/dev/null; then \
        open -a Docker; \
    else \
        sudo systemctl start docker; \
    fi

# ---- Stop Docker
[group('ci-cd')]
docker-down:
    @if command -v brew >/dev/null; then \
        osascript -e 'quit app "Docker"'; \
    else \
        sudo systemctl stop docker; \
    fi

# ---- Build the Docker image
[group('docker')]
docker-build:
    docker build -f {{ DOCKERFILE }} -t {{ DOCKER_IMAGE }} \
        --build-arg CYTIQ_BUILD_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)" .

# ---- Remove Rust build artifacts
[group('utils')]
clean:
    cargo clean

# ---- Show available recipes
help:
    @just --list
