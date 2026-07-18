# Built by Cloud Build from a staging dir containing both repos side by
# side (the catalog's Cargo.toml depends on ../AgentMesh/sdk-rust). See
# deploy/deploy.sh — nothing here runs on a developer machine.
FROM rust:1-bookworm AS build
WORKDIR /w
COPY AgentMesh /w/AgentMesh
COPY personal-agent-naming /w/personal-agent-naming
WORKDIR /w/personal-agent-naming
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /w/personal-agent-naming/target/release/agent-catalog /usr/local/bin/agent-catalog
# The spec is a document, not code: shipped beside the binary, read at runtime.
COPY --from=build /w/personal-agent-naming/PAN-SPEC.md /srv/PAN-SPEC.md
WORKDIR /srv
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/agent-catalog"]
