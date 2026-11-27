# syntax=docker/dockerfile:1
FROM rust:1.94.0-bookworm AS build
WORKDIR /source
RUN apt-get update -o Acquire::Retries=3 -o Acquire::http::Timeout=30 \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY apps/desktop/src-tauri/ apps/desktop/src-tauri/
ENV CARGO_PROFILE_RELEASE_DEBUG=0
RUN cargo +1.94.0 build --locked --release -p infimount_hosted

FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/infimount/infimount" \
      org.opencontainers.image.title="Infimount hosted artifact service"
RUN apt-get update -o Acquire::Retries=3 -o Acquire::http::Timeout=30 \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data/artifacts /run/infimount /run/secrets/infimount \
    && chown -R 65532:65532 /data/artifacts
COPY --from=build /source/target/release/infimount-hosted /usr/local/bin/infimount-hosted
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/infimount-hosted"]
# No default serve command. An explicit configuration and command are required.
CMD ["check"]
