FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y libclang-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ src/
COPY .git/ .git/
RUN cargo build --release

FROM debian:bookworm-slim
RUN useradd -r -s /usr/sbin/nologin vanilo
COPY --from=builder /build/target/release/vanilo /usr/local/bin/vanilo
