# Step 1: Build the Rust project
FROM rust:slim-bookworm AS builder
WORKDIR /usr/src/kvs
COPY Cargo.toml ./

RUN apt-get update
RUN apt-get install -y pkg-config libssl-dev

# Create a dummy `main.rs` so cargo fetch can run
RUN mkdir -p src
RUN echo 'fn main() {}' > src/main.rs

# These were helping locally, when the registry was occasionally failing
RUN export CARGO_HTTP_MULTIPLEXING=false
RUN export CARGO_NET_RETRY=10
RUN export CARGO_NET_TIMEOUT=60

# Download and build the dependencies (this will cache the dependencies layer)
RUN cargo fetch
RUN cargo build --release

# Copy the rest of the source code and build the project
COPY ./src ./src
RUN cargo build --release

# Step 2: Create the image base image
FROM debian:bookworm-slim AS base
RUN apt-get update
RUN apt-get install -y pkg-config libssl-dev
COPY --from=builder /usr/src/kvs/target/release/kvs /usr/local/bin/kvs

# Step 2: Create the main image
FROM base AS kvs-main
COPY sample_configuration/config.json ./config.json
EXPOSE 3030
CMD ["kvs"]

# Step 2: Create the replica image
FROM base AS kvs-replica
COPY sample_configuration/config_replica.json ./config.json
EXPOSE 3030 3040
CMD ["kvs"]
