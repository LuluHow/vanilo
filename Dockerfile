FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y libclang-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/simple /usr/local/bin/simple
