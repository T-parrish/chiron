# syntax=docker/dockerfile:1
#
# Built from the repository root (context: .) so the whole Cargo workspace is
# available. Uses cargo-chef to cache the dependency graph. cmake is needed to
# compile librdkafka from source (rdkafka "cmake-build" feature).

FROM lukemathwalker/cargo-chef:latest-rust-1.88 AS chef
WORKDIR /app

# --- Plan: distil the workspace into a recipe that captures only the deps. ----
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Cook: build the dependency graph (cached), then build the binary. --------
FROM chef AS builder
RUN apt-get update && apt-get install -y cmake && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
# This layer is the expensive one and is cached as long as recipe.json is stable.
RUN cargo chef cook --release --package job-service --recipe-path recipe.json
# Real source. migrations/ is needed at compile time because main.rs embeds the
# schema via include_str!("../migrations/001_init.sql").
COPY . .
RUN cargo build --release --package job-service

# --- Runtime ------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/job-service /usr/local/bin/job-service
# Bundle migrations so the service can run them on startup.
COPY services/job-service/migrations /app/migrations
WORKDIR /app
EXPOSE 8081
CMD ["job-service"]
