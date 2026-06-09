FROM rust:1.88 AS builder
WORKDIR /app

# Copy workspace manifests first so dependency caching works
COPY Cargo.toml Cargo.lock ./
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
# job-service is a workspace member; its manifest is needed to resolve the workspace
COPY services/job-service/Cargo.toml ./services/job-service/Cargo.toml

# Dummy build to cache dependencies before copying real source
RUN mkdir -p services/api-gateway/src services/job-service/src \
 && echo "fn main(){}" > services/api-gateway/src/main.rs \
 && echo "fn main(){}" > services/job-service/src/main.rs \
 && cargo build --release --package api-gateway \
 && rm -rf services/api-gateway/src
# NOTE: keep the job-service dummy src so the workspace still resolves; only
# api-gateway/src is replaced by real source below.

# Copy real source and rebuild
COPY services/api-gateway/src ./services/api-gateway/src
RUN touch services/api-gateway/src/main.rs \
 && cargo build --release --package api-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/api-gateway /usr/local/bin/api-gateway
EXPOSE 8080
CMD ["api-gateway"]
