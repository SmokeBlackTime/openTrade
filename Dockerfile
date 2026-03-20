# Multi-stage build for OpenTrade
FROM rust:1.75-bookworm AS builder

WORKDIR /app
COPY . .

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --bin opentrade

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/opentrade /usr/local/bin/opentrade
COPY --from=builder /app/config /etc/opentrade/config

RUN mkdir -p /var/lib/opentrade/data
ENV OPENTRADE_DATA_DIR=/var/lib/opentrade/data

ENTRYPOINT ["opentrade"]
CMD ["--config", "/etc/opentrade/config/default.yaml", "doctor"]
