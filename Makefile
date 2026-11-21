.PHONY: deps deps-down dev dev-noauth demo build web check test fmt clippy docker docker-push image release version clean

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
COMMIT := $(shell git rev-parse --short=12 HEAD 2>/dev/null)
## Immutable image tag: the commit's v* tag (a release), else the commit when the tree is
## clean, else a timestamp. /health reports it.
VERSION ?= $(or $(shell git describe --tags --exact-match --match 'v*' HEAD 2>/dev/null),$(shell git diff --quiet HEAD -- 2>/dev/null && echo $(COMMIT) || date -u +%Y%m%d-%H%M%S))

version:
	@echo $(VERSION)

docker:
	docker build --platform linux/amd64 --build-arg VERSION=$(VERSION) -t $(REGISTRY)/robine-dog:$(VERSION) .

## Builds and pushes the image; silo01's `make robine-dog-deploy ROBINE_DOG_VERSION=<tag>` deploys it.
docker-push: docker
	docker push $(REGISTRY)/robine-dog:$(VERSION)

## The image of HEAD, tagged with the commit and its release (the CI's; refuses uncommitted changes).
image:
	@test -z "$$(git status --porcelain)" || { echo "Uncommitted changes: commit first, the image is tagged with the commit." >&2; exit 1; }
	docker buildx build --platform linux/amd64 --build-arg VERSION=$(VERSION) \
		-t $(REGISTRY)/robine-dog:$(COMMIT) -t $(REGISTRY)/robine-dog:$(VERSION) --push .
	@echo "Pushed $(REGISTRY)/robine-dog:$(VERSION) ($(COMMIT))"

## Releases main's HEAD: make release RELEASE=v0.2.0 pushes the tag; its CI checks, builds and
## pushes the image, and robine-actions rolls it out (silo01's robine-dog-deploy).
release:
	@echo "$(RELEASE)" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$$' || { echo "RELEASE=vX.Y.Z is required (got '$(RELEASE)')" >&2; exit 1; }
	@test -z "$$(git status --porcelain)" || { echo "Uncommitted changes." >&2; exit 1; }
	@test "$$(git rev-parse --abbrev-ref HEAD)" = main || { echo "Release from main." >&2; exit 1; }
	@git fetch --quiet --tags origin main
	@test "$$(git rev-parse HEAD)" = "$$(git rev-parse origin/main)" || { echo "main is not origin/main: pull or push first." >&2; exit 1; }
	@! git rev-parse --quiet --verify "refs/tags/$(RELEASE)" >/dev/null || { echo "$(RELEASE) already exists." >&2; exit 1; }
	git tag -a "$(RELEASE)" -m "$(RELEASE)"
	git push origin "refs/tags/$(RELEASE)"
	@echo "$(RELEASE) pushed: its CI then deploys it (GitHub Actions, job Deploy)."

clean:
	cargo clean
	rm -rf web/dist
