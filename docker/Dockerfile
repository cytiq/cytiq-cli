# syntax=docker/dockerfile:1

# ---- Build -------------------------------------------------------------------
FROM rust:1.99-slim-trixie AS builder

WORKDIR /app

# The whole workspace is needed: every member listed in Cargo.toml must exist
# for `cargo build --locked` to resolve, even when only cytiq-cli is built.
COPY Cargo.toml Cargo.lock ./
COPY crates crates

# The build context has no `.git`; provenance records take the commit from
# here (`just docker-build` passes `git rev-parse HEAD`).
ARG CYTIQ_BUILD_COMMIT=unknown

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build -p cytiq-cli --release --locked \
    && install -D -m 0755 target/release/cytiq /out/cytiq

# CPU ONNX Runtime is a build dependency; the final image contains no Python.
FROM python:3.13-slim-trixie AS onnx-runtime
RUN pip install --no-cache-dir --target /ortpkg onnxruntime==1.25.1

# ---- Runtime -----------------------------------------------------------------
FROM debian:trixie-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends libstdc++6 libgomp1 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --user-group --home-dir /data --create-home cytiq

COPY --from=builder /out/cytiq /usr/local/bin/cytiq
COPY --from=onnx-runtime /ortpkg/onnxruntime/capi/libonnxruntime.so.1.25.1 /usr/local/bin/lib/libonnxruntime.so
COPY --from=onnx-runtime /ortpkg/onnxruntime/capi/libonnxruntime_providers_shared.so /usr/local/bin/lib/
COPY --from=onnx-runtime /ortpkg/onnxruntime/LICENSE /usr/local/bin/lib/ONNX-RUNTIME-LICENSE
COPY --from=onnx-runtime /ortpkg/onnxruntime/ThirdPartyNotices.txt /usr/local/bin/lib/

USER cytiq
WORKDIR /data

ENTRYPOINT ["cytiq"]
CMD ["--help"]
