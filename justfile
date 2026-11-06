# justfile
# -----------------------------------------------------------------------------
# Shared development commands for the cytiq Rust application.

# Recipes run with the tools pinned in mise.toml, through the mise release that
# `just mise` installs outside PATH. `mise-cytiq` links to it and exists nowhere
# else, so a recipe fails rather than run another mise on PATH.
set shell := ["mise-cytiq", "exec", "--", "bash", "-eu", "-o", "pipefail", "-c"]

MISE_VERSION := replace_regex(read(justfile_directory() / "mise.toml"), '(?sm)\A.*?^min_version = "([^"]+)"$.*\z', '$1')
MISE_DIR := env("XDG_DATA_HOME", home_directory() / ".local/share") / "cytiq/mise" / MISE_VERSION
MISE := MISE_DIR / "mise"
export PATH := MISE_DIR + ":" + env("PATH")

# ---- Project configuration

CLI_PACKAGE := env("CLI_PACKAGE", "cytiq-engine")
DOCKER_IMAGE := env("DOCKER_IMAGE", "cytiq:dev")
DOCKERFILE := env("DOCKERFILE", "docker/Dockerfile")

# ---- Install the pinned mise and the tools in mise.toml; rerun after either changes
[group('setup')]
[script('bash')]
mise:
    set -euo pipefail
    version={{ quote(MISE_VERSION) }}
    mise={{ quote(MISE) }}
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo 'mise.toml must set min_version to an exact mise release' >&2
        exit 1
    fi
    if [[ "$("$mise" --version 2>/dev/null)" != "$version "* ]]; then
        installer="$(mktemp)"
        trap 'rm -f -- "$installer"' EXIT
        # The release's own installer embeds the checksums of its archives.
        curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
            "https://github.com/jdx/mise/releases/download/v$version/install.sh" --output "$installer"
        MISE_VERSION="v$version" MISE_INSTALL_PATH="$mise" MISE_INSTALL_FROM_GITHUB=1 \
            MISE_INSTALL_HELP=0 sh "$installer"
    fi
    ln -sfn mise "$(dirname -- "$mise")/mise-cytiq"
    "$mise" trust --quiet
    "$mise" install --locked

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
    cargo build -p cytiq-engine --locked
    R_LIBS_USER=parity/.env/Rlib Rscript parity/cytonorm.R target/debug/cytiq

# ---- Validate CytoNorm on six real CyTOF controls with a held-out donor
[group('validation')]
parity-cytonorm-real:
    cargo build -p cytiq-engine --locked
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
    cargo test -p cytiq-engine --example performance_support --locked

# ---- Regenerate the golden end-to-end CI fixture
[group('validation')]
parity-ci-fixture:
    parity/.env/py/bin/python parity/gen/pipeline.py data/fixtures/pipeline ci

# ---- Regenerate the CI parity smoke fixture: small datasets plus upstream R outputs
[group('validation')]
parity-smoke-fixture:
    parity/make_smoke.sh

# ---- Fuzz read_fcs (seeded from fixtures and $CYTIQ_REAL_DATA) or spillover_keyword on the pinned nightly
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
    rustup toolchain install "$CYTIQ_FUZZ_TOOLCHAIN" --profile minimal --no-self-update
    # cargo-fuzz otherwise builds for the triple it was built for, which is
    # musl (no sanitizers) for prebuilt Linux binaries.
    export RUSTUP_TOOLCHAIN="$CYTIQ_FUZZ_TOOLCHAIN" && cd crates/cytiq-fcs && \
      cargo fuzz run {{ target }} --target "$(rustc -vV | sed -n 's/^host: //p')" \
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

# ---- Check crates.io metadata and internal versions, and package every crate (uploads nothing)
[group('quality')]
package-check:
    python3 .github/scripts/publish_crates.py --check --allow-dirty

# ---- Run the standard local quality gate
[group('quality')]
verify:
    just fmt
    just check
    just lint
    just test
    just performance-check
    just crap-check
    just package-check

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

# ---- First crates.io publication of crates not on crates.io yet (`cargo login` first; see CONTRIBUTING.md)
[group('ci-cd')]
publish-crates-first:
    python3 .github/scripts/publish_crates.py --allow-new --dry-run
    python3 .github/scripts/publish_crates.py --allow-new --no-verify --wait-on-rate-limit 30

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
