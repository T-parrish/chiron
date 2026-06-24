# syntax=docker/dockerfile:1
#
# Built from the repository root (context: .) so the whole Cargo workspace is
# available. Uses cargo-chef to cache the dependency graph: the `cook` layer is
# rebuilt only when Cargo.lock or a member manifest changes, not on every source
# edit.

FROM lukemathwalker/cargo-chef:latest-rust-1.88 AS chef
WORKDIR /app

# --- Plan: distil the workspace into a recipe that captures only the deps. ----
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Cook: build the dependency graph (cached), then build the binary. --------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# This layer is the expensive one and is cached as long as recipe.json is stable.
RUN cargo chef cook --release --package api-gateway --recipe-path recipe.json
COPY . .
RUN cargo build --release --package api-gateway

# --- Runtime ------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/api-gateway /usr/local/bin/api-gateway
EXPOSE 8080
CMD ["api-gateway"]
