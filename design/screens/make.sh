#!/bin/sh
# Screenshots for the download page, site/assets/shots/*.png, light and dark: the demo
# collection of this folder against servers.py, through the dev bridge (`make bridge` running).
set -e
cd "$(dirname "$0")"
HERE="$(pwd)"
OUT="$HERE/../../site/assets/shots"
mkdir -p "$OUT"
python3 servers.py &
SERVERS=$!
trap 'kill $SERVERS 2>/dev/null' EXIT INT TERM
sleep 1
TMP="$(mktemp -d)"
printf 'localStorage.clear();\nlocalStorage.setItem("collection", "%s/shop");\nlocalStorage.setItem("env:%s/shop", "stage");\n' "$HERE" "$HERE" > "$TMP/setup.js"
for shot in hero checks compare code; do
    cat lib.js "$shot.js" > "$TMP/$shot.js"
    for theme in light dark; do
        swift ../screenshot.swift http://localhost:1430/bridge.html "$OUT/$shot-$theme.png" 1360 820 "$TMP/setup.js" "$TMP/$shot.js" "$theme"
    done
done
rm -rf "$TMP"
