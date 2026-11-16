#!/usr/bin/env bash
# The Rust server (API, intake, sign-in) on :8080 and :8126, and the Vite UI
# on :5173 proxying to it. Open http://localhost:5173.
set -euo pipefail
cd "$(dirname "$0")/.."
trap 'kill 0' EXIT
cargo run -p robine-dog &
(cd web && npm run dev -- --clearScreen false) &
wait
