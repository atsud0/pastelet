# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.99.1
FROM rust:${RUST_VERSION}-slim-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY static ./static
COPY scripts/build-css.sh ./scripts/build-css.sh
RUN sh scripts/build-css.sh --check \
    && cargo build --release --locked --bin rpaste

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 rpaste \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /data --shell /usr/sbin/nologin rpaste \
    && install -d -o 10001 -g 10001 -m 0700 /data
COPY --from=builder /build/target/release/rpaste /usr/local/bin/rpaste
ENV BIND_ADDR=0.0.0.0:3000 \
    DATABASE_URL=sqlite:///data/rpaste.db \
    BASE_URL=http://localhost:3000 \
    RUST_LOG=rpaste=info,tower_http=info
WORKDIR /data
USER 10001:10001
EXPOSE 3000
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl --fail --silent --show-error --connect-timeout 2 --max-time 4 http://127.0.0.1:3000/ > /dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/rpaste"]
