# syntax=docker/dockerfile:1

FROM rust:1.99.1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p tesla-logger

FROM debian:bookworm-slim
# Client only, from the signed PostgreSQL repo, so it matches the Postgres 17 server.
# curl is removed after the key is fetched.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && install -d /usr/share/postgresql-common/pgdg \
    && curl -fsSL -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
        https://www.postgresql.org/media/keys/ACCC4CF8.asc \
    && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends postgresql-client-17 \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
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
