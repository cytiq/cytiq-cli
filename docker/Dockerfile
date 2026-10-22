# syntax=docker/dockerfile:1

# ---- Build -------------------------------------------------------------------
FROM rust:1.99-slim-trixie AS builder

WORKDIR /app

# The whole workspace is needed: every member listed in Cargo.toml must exist
# for `cargo build --locked` to resolve, even when only cytiq-cli is built.
COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build -p cytiq-cli --release --locked \
    && install -D -m 0755 target/release/cytiq /out/cytiq

# ---- Runtime -----------------------------------------------------------------
FROM debian:trixie-slim AS runtime

RUN useradd --system --uid 10001 --user-group --home-dir /data --create-home cytiq

COPY --from=builder /out/cytiq /usr/local/bin/cytiq

USER cytiq
WORKDIR /data

ENTRYPOINT ["cytiq"]
CMD ["--help"]
