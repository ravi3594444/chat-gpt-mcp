FROM rust:1.99.1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY core ./core
COPY server ./server
RUN cargo build --release --locked -p browser-link-server

FROM debian:bookworm-slim
RUN useradd --uid 10001 --create-home bridge && mkdir /data && chown bridge:bridge /data
COPY --from=builder /build/target/release/browser-link-server /usr/local/bin/browser-link-server
USER bridge
ENV BROWSER_LINK_BIND=0.0.0.0:7319 BROWSER_LINK_DATA_DIR=/data
EXPOSE 7319
ENTRYPOINT ["browser-link-server"]
