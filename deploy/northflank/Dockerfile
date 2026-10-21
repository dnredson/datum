FROM rust:bookworm AS builder

WORKDIR /src/dserver

COPY dserver/Cargo.toml dserver/Cargo.lock ./
COPY dserver/src ./src

RUN cargo build --locked --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /src/dserver/target/release/DServer /usr/local/bin/dserver
COPY dserver/config.toml /app/config.toml

ENV DATUM_DSERVER_CONFIG_PATH=/app/config.toml
ENV DATUM_CONTROL_STATE_DB_PATH=/data/datum-control-plane.db

RUN mkdir -p /data

CMD ["dserver"]
