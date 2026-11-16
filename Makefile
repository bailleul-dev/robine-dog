.PHONY: dev dev-noauth demo build web check test fmt clippy docker clean

## Server + UI with Robine ID sign-in (settings in .env). Open http://localhost:5173
dev: web/node_modules
	scripts/dev.sh

## Same, without sign-in (loopback only). Open http://localhost:5173
dev-noauth: web/node_modules
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

docker:
	docker build -t robine-dog .

clean:
	cargo clean
	rm -rf web/dist
