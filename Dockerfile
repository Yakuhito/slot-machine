FROM rust:1-bookworm AS builder
WORKDIR /app

# git crate deps; vendored openssl needs a C toolchain in the build image.
RUN apt-get update \
    && apt-get install -y --no-install-recommends git cmake build-essential pkg-config clang \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin slot-machine \
    && install -Dm755 target/release/slot-machine /out/slot-machine

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data \
    && useradd --system --uid 1000 --home-dir /data --no-create-home slot-machine \
    && chown slot-machine:slot-machine /data

COPY --from=builder /out/slot-machine /usr/local/bin/slot-machine

USER slot-machine
WORKDIR /data
EXPOSE 8080

ENV BIND_ADDR=0.0.0.0:8080

ENTRYPOINT ["slot-machine"]
