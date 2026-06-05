FROM rust:1.82 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# Dummy build to cache dependencies
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/api-gateway /usr/local/bin/api-gateway
EXPOSE 8080
CMD ["api-gateway"]
