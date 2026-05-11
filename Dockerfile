ARG ORT_VERSION=1.26.0
# sha256 of onnxruntime-linux-x64-${ORT_VERSION}.tgz from https://github.com/microsoft/onnxruntime/releases
ARG ORT_SHA256=1254da24fb389cf39dc0ff3451ab48301740ffbfcbaf646849df92f80ee92c57

FROM debian:trixie-slim AS ortfetch
ARG ORT_VERSION
ARG ORT_SHA256
ADD --checksum=sha256:${ORT_SHA256} \
    https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz \
    /tmp/ort.tgz
RUN tar -xzf /tmp/ort.tgz -C /opt/ \
    && cp /opt/onnxruntime-linux-x64-${ORT_VERSION}/lib/libonnxruntime.so.${ORT_VERSION} /opt/libonnxruntime.so

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
COPY --from=ortfetch /opt/libonnxruntime.so /opt/ort/lib/libonnxruntime.so
ENV ORT_LIB_PATH=/opt/ort/lib
ENV ORT_PREFER_DYNAMIC_LINK=1
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY tests/ tests/
COPY config/models.toml config/models.toml
RUN cargo build --release --features full

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libavdevice61 \
    libstdc++6 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/target/release/notemill-worker ./
COPY --from=ortfetch /opt/libonnxruntime.so /usr/local/lib/libonnxruntime.so
RUN ldconfig

ENTRYPOINT ["./notemill-worker"]
