#!/bin/sh
# Dev only: the interface in a browser against the real core.
# The bridge serves the app's commands on :1431, Vite serves bridge.html on :1430.
set -e
cd "$(dirname "$0")/.."
cargo build -q -p tsubamedo-bridge
target/debug/tsubamedo-bridge &
BRIDGE=$!
trap 'kill $BRIDGE 2>/dev/null' EXIT INT TERM
cd apps/desktop
[ -d node_modules ] || npm install
npx vite
