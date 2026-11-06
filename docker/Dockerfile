# syntax=docker/dockerfile:1

# ---- Toolchain ---------------------------------------------------------------
# Rust as mise.toml pins it.
FROM rust:1.99.1-slim-trixie AS chef

# Static ONNX Runtime linking needs the C++ standard library development files.
RUN apt-get update && apt-get install -y --no-install-recommends g++ \
    && rm -rf /var/lib/apt/lists/*

# cargo-chef splits the build so that compiled dependencies get a layer of their
# own, which the registry cache in cytiq-ci's docker-publish.yml can export.
ARG CARGO_CHEF_VERSION=0.1.78
RUN cargo install cargo-chef --locked --version "$CARGO_CHEF_VERSION" \
    && rm -rf /usr/local/cargo/registry

WORKDIR /app

# ---- Dependency recipe -------------------------------------------------------
FROM chef AS planner

# The whole workspace is needed: every member listed in Cargo.toml must exist
# for `--locked` to resolve, even when only cytiq-engine is built.
COPY Cargo.toml Cargo.lock ./
COPY crates crates

# recipe.json lists only manifests and the lockfile, with the workspace's own
# version masked: source edits and release version bumps leave it unchanged.
RUN cargo chef prepare --recipe-path recipe.json

# ---- Build -------------------------------------------------------------------
FROM chef AS builder

# Compile the dependencies (and download ONNX Runtime) into this layer. There
# are no cache mounts: what this step writes must stay in the layer, so the
# step is reused for as long as recipe.json is unchanged. Any Cargo.lock
# dependency change rebuilds it.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook -p cytiq-engine --release --locked --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates crates

# The build context has no `.git`; provenance records take the commit from
# here (`just docker-build` also marks modified source as dirty). Declared after the
# cook step, so that a new commit does not invalidate the dependency layer.
ARG CYTIQ_BUILD_COMMIT=unknown

# This layer is never reused, since every source change invalidates it.
# Removing target/ keeps it small in the mode=max registry cache.
RUN cargo build -p cytiq-engine --release --locked \
    && install -D -m 0755 target/release/cytiq /out/cytiq \
    && rm -rf target

# ---- Runtime -----------------------------------------------------------------
FROM debian:trixie-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends libstdc++6 libgomp1 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --user-group --home-dir /data --create-home cytiq

COPY --from=builder /out/cytiq /usr/local/bin/cytiq

USER cytiq
WORKDIR /data

ENTRYPOINT ["cytiq"]
CMD ["--help"]
