FROM rust:1.88 AS builder

# cmake is required to compile librdkafka from source (rdkafka cmake-build feature)
RUN apt-get update && apt-get install -y cmake && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests first so dependency caching works
COPY Cargo.toml Cargo.lock ./
COPY services/job-service/Cargo.toml ./services/job-service/Cargo.toml
# api-gateway is a workspace member; its manifest is needed to resolve the workspace
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml

# Dummy build to cache dependencies before copying real source
RUN mkdir -p services/job-service/src services/api-gateway/src \
 && echo "fn main(){}" > services/job-service/src/main.rs \
 && echo "fn main(){}" > services/api-gateway/src/main.rs \
 && cargo build --release --package job-service \
 && rm -rf services/job-service/src
# NOTE: keep the api-gateway dummy src so the workspace still resolves; only
# job-service/src is replaced by real source below.

# Copy real source and rebuild. migrations/ is needed at compile time because
# main.rs embeds the schema via include_str!("../migrations/001_init.sql").
COPY services/job-service/src ./services/job-service/src
COPY services/job-service/migrations ./services/job-service/migrations
RUN touch services/job-service/src/main.rs \
 && cargo build --release --package job-service

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/job-service /usr/local/bin/job-service
# Bundle migrations so the service can run them on startup
COPY services/job-service/migrations /app/migrations
WORKDIR /app
EXPOSE 8081
CMD ["job-service"]
