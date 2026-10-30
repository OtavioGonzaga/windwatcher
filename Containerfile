FROM docker.io/library/rust:trixie@sha256:09027f873d45b8ecd1dc86f1fa13b7a62b875c961f017a0e331267b350430137 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
RUN cargo build --release --locked -p windwatcher-server --bin windwatcher \
    && ldd target/release/windwatcher \
    && readelf -d target/release/windwatcher \
    && install -D -m 0755 target/release/windwatcher /out/windwatcher \
    && strip --strip-unneeded /out/windwatcher

FROM docker.io/library/debian:13-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4
ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="windwatcher-server" \
      org.opencontainers.image.description="Windwatcher standalone server and worker" \
      org.opencontainers.image.source="https://github.com/OtavioGonzaga/windwatcher" \
      org.opencontainers.image.documentation="https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/container.md" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="$VERSION" \
      org.opencontainers.image.revision="$REVISION"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 windwatcher \
    && useradd --uid 10001 --gid 10001 --home-dir /var/lib/windwatcher --no-create-home --shell /usr/sbin/nologin windwatcher \
    && install -d -o 10001 -g 10001 -m 0750 /etc/windwatcher /var/lib/windwatcher
COPY --from=builder /out/windwatcher /usr/bin/windwatcher
COPY LICENSE /usr/share/licenses/windwatcher-server/LICENSE
USER 10001:10001
WORKDIR /var/lib/windwatcher
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/bin/windwatcher", "--system"]
CMD ["serve"]
