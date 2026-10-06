# syntax=docker/dockerfile:1
ARG RUST_IMAGE=rust:1.98.1-bookworm
FROM ${RUST_IMAGE} AS gateway
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY third_party ./third_party
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p benilla-asset-gateway && \
    cp target/release/benilla-asset-gateway /usr/local/bin/benilla-asset-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends python3 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /run/benilla && chown 10001:10001 /run/benilla
COPY --from=gateway /usr/local/bin/benilla-asset-gateway /usr/local/bin/
# Named build context, supplied by scripts/build-production.sh. Never copies WoW.
COPY --from=web / /opt/benilla/web/
COPY deploy/entrypoint.py /opt/benilla/entrypoint.py
USER 10001:10001
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD ["python3", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/index.html', timeout=3).read(1)"]
ENTRYPOINT ["python3", "/opt/benilla/entrypoint.py"]
