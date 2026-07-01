FROM rust:1.95-bookworm AS builder

WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/outlook-mail-pool-api /usr/local/bin/outlook-mail-pool-api

ENV MAIL_POOL_HOST=0.0.0.0
ENV MAIL_POOL_PORT=8098
ENV DATABASE_URL=sqlite://data/mail_pool.sqlite?mode=rwc

RUN mkdir -p /app/data
EXPOSE 8098
CMD ["outlook-mail-pool-api"]
