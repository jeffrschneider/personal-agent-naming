# Built by Cloud Build from a staging dir containing both repos side by
# side (the catalog's Cargo.toml depends on ../AgentMesh/sdk-rust). See
# deploy/deploy.sh — nothing here runs on a developer machine.
FROM rust:1-bookworm AS build
WORKDIR /w
COPY AgentMesh /w/AgentMesh
COPY Agent-Catalog-ARD /w/Agent-Catalog-ARD
WORKDIR /w/Agent-Catalog-ARD
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /w/Agent-Catalog-ARD/target/release/agent-catalog /usr/local/bin/agent-catalog
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/agent-catalog"]
