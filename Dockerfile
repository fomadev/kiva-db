# KivaDB container image.
#
#   docker build -t kivadb .
#   docker run -d --name kivadb -e KIVADB_ADMIN_PASSWORD=change-me \
#              -p 2911:2911 -p 8080:8080 -v kivadb-data:/data kivadb

# --- Build stage -----------------------------------------------------------
FROM rust:1-slim-bookworm AS build
# A C toolchain is needed once, for the embedded Lua interpreter.
RUN apt-get update && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY LICENSE README.md ./
RUN cargo build --release --locked && strip target/release/kivadb

# --- Runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim
RUN useradd --system --create-home --home-dir /data --uid 10001 kivadb
COPY --from=build /src/target/release/kivadb /usr/local/bin/kivadb
COPY LICENSE /usr/share/doc/kivadb/LICENSE

# Everything KivaDB keeps (configuration, users, data) lives in /data.
ENV KIVADB_HOME=/data \
    KIVADB_BIND=0.0.0.0
VOLUME /data
USER kivadb
WORKDIR /data

# 2911: KivaDB protocol (shells, Kiva Studio, client libraries). 8080: REST API.
EXPOSE 2911 8080

# The image listens on every interface, so KivaDB insists on authentication:
# KIVADB_ADMIN_PASSWORD must be given the first time the container starts.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD ["kivadb", "status"]

ENTRYPOINT ["kivadb"]
CMD ["serve"]
