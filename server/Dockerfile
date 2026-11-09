# syntax=docker/dockerfile:1

FROM rust:1.99.1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p tesla-logger

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /data logger \
    && chown -R logger:logger /data
COPY --from=build /src/target/release/tesla-logger /usr/local/bin/tesla-logger
USER logger
WORKDIR /data
EXPOSE 4000
ENV TESLA_LISTEN_ADDR=0.0.0.0:4000
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["tesla-logger", "healthcheck", "--url", "http://127.0.0.1:4000/healthz"]
ENTRYPOINT ["tesla-logger"]
CMD ["serve"]
