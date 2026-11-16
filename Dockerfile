FROM node:26-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-fund --no-audit
COPY web/ ./
RUN npm run build

FROM rust:1.98-slim AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p robine-dog

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data robine
COPY --from=server /src/target/release/robine-dog /usr/local/bin/robine-dog
COPY --from=web /web/dist /app/web
ENV ROBINE_DOG_LISTEN=0.0.0.0:8080 \
    ROBINE_DOG_TRACE_LISTEN=0.0.0.0:8126 \
    ROBINE_DOG_DB=/data/robine-dog.db \
    ROBINE_DOG_STATIC_DIR=/app/web
USER robine
VOLUME /data
EXPOSE 8080 8126
CMD ["robine-dog"]
