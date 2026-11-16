.PHONY: deps deps-down dev dev-noauth demo build web check test fmt clippy docker docker-push version clean

## GreptimeDB (:14000) and PostgreSQL (:15432) for development, in Docker.
deps:
	docker compose up -d --wait

deps-down:
	docker compose down

## Server + UI with Robine ID sign-in (settings in .env). Open http://localhost:5173
dev: web/node_modules deps
	scripts/dev.sh

## Same, without sign-in (loopback only). Open http://localhost:5173
dev-noauth: web/node_modules deps
	ROBINE_DOG_AUTH=disabled scripts/dev.sh

## Demo traffic: an hour of history, then live requests (traces + correlated logs).
demo:
	cargo run -q -p robine-dog-demo -- --backfill-minutes 60 --rate 3

web/node_modules: web/package.json
	cd web && npm install --no-fund --no-audit
	touch web/node_modules

web: web/node_modules
	cd web && npm run build

## Release binary + UI bundle (web/dist, served by the binary).
build: web
	cargo build --release -p robine-dog

check: web/node_modules
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	cd web && npm run typecheck

test:
	cargo test

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets -- -D warnings

REGISTRY ?= registry.bailleul.dev
## Immutable image tag: the commit when the tree is clean, else a timestamp.
VERSION ?= $(shell git diff --quiet HEAD -- 2>/dev/null && git rev-parse --short=12 HEAD 2>/dev/null || date -u +%Y%m%d-%H%M%S)

version:
	@echo $(VERSION)

docker:
	docker build --platform linux/amd64 --build-arg VERSION=$(VERSION) -t $(REGISTRY)/robine-dog:$(VERSION) .

## Builds and pushes the image; silo01's `make robine-dog` deploys it.
docker-push: docker
	docker push $(REGISTRY)/robine-dog:$(VERSION)

clean:
	cargo clean
	rm -rf web/dist
