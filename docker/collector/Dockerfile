# syntax=docker/dockerfile:1
# Image of the collector of NAS Stats (#46). Built from the root of the
# repository by scripts/build-collector-image.sh, see docs/collector.md.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p nas-collector \
    && cp target/release/nas-collector /nas-collector

FROM debian:bookworm-slim
# Own user without a shell; it owns the data directory.
RUN useradd --system --uid 10001 --user-group --no-create-home \
        --home-dir /nonexistent --shell /usr/sbin/nologin nascollector \
    && mkdir -p /data /config \
    && chown 10001:10001 /data \
    && chmod 700 /data
COPY --from=build /nas-collector /usr/local/bin/nas-collector
USER 10001:10001
ENV NAS_COLLECTOR_CONFIG=/config/collector.toml \
    NAS_COLLECTOR_DATA=/data
VOLUME /data
EXPOSE 8743
ENTRYPOINT ["nas-collector"]
CMD ["run"]
