FROM rust:1.87-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/cytiq-cli/Cargo.toml crates/cytiq-cli/Cargo.toml
COPY crates/cytiq-autospectral/Cargo.toml crates/cytiq-autospectral/Cargo.toml
COPY crates/cytiq-autospill/Cargo.toml crates/cytiq-autospill/Cargo.toml
COPY crates/cytiq-fcs/Cargo.toml crates/cytiq-fcs/Cargo.toml
COPY crates/cytiq-autospectral/src crates/cytiq-autospectral/src
COPY crates/cytiq-autospill/src crates/cytiq-autospill/src
COPY crates/cytiq-cli/src crates/cytiq-cli/src
COPY crates/cytiq-fcs/src crates/cytiq-fcs/src

RUN cargo build -p cytiq-cli --release --locked

FROM debian:bookworm-slim AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/cytiq /usr/local/bin/cytiq

CMD ["cytiq"]
