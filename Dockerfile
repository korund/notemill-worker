FROM rust:1-slim-trixie AS build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libavdevice-dev \
    libclang-dev \
    build-essential \
    cmake \
    libsqlite3-dev \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY tests/ tests/
COPY config/models.toml config/models.toml
RUN cargo build --release --features full

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libavdevice61 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/target/release/notemill-worker ./

ENTRYPOINT ["./notemill-worker"]
