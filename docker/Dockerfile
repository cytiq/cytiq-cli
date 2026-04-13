FROM rust:1.87-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/cytiq-cli/Cargo.toml crates/cytiq-cli/Cargo.toml
COPY crates/cytiq-core/Cargo.toml crates/cytiq-core/Cargo.toml
COPY crates/cytiq-cli/src crates/cytiq-cli/src
COPY crates/cytiq-core/src crates/cytiq-core/src

RUN cargo build --workspace --release

FROM debian:bookworm-slim AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/cytiq-cli /usr/local/bin/cytiq

CMD ["cytiq"]
