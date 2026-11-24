FROM node:26-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-fund --no-audit
COPY web/ ./
COPY docs/ /docs/
RUN npm run build

FROM rust:1.98-slim AS server
ARG VERSION=dev
ENV ROBINE_DOG_VERSION=$VERSION
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY README.md ./
RUN cargo build --release -p robine-dog

FROM debian:trixie-slim
# Chromium runs the browser tests of Synthetic Monitoring.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
        chromium fonts-liberation fonts-noto-color-emoji && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home robine
COPY --from=server /src/target/release/robine-dog /usr/local/bin/robine-dog
COPY --from=web /web/dist /app/web
ENV ROBINE_DOG_LISTEN=0.0.0.0:8080 \
    ROBINE_DOG_TRACE_LISTEN=0.0.0.0:8126 \
    ROBINE_DOG_DOGSTATSD_LISTEN=0.0.0.0:8125 \
    ROBINE_DOG_STATIC_DIR=/app/web \
    ROBINE_DOG_CHROMIUM=/usr/bin/chromium \
    ROBINE_DOG_CHROMIUM_ARGS=--no-sandbox \
    HOME=/tmp
USER robine
EXPOSE 8080 8126 8125/udp
CMD ["robine-dog"]
