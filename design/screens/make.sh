#!/bin/sh
# Screenshots for the download page, light and dark, in Russian (site/assets/shots/*.png, the
# demo collection shop/) and in English (site/assets/shots/en/*.png, shop-en/), against
# servers.py, through the dev bridge (`make bridge` running).
set -e
cd "$(dirname "$0")"
HERE="$(pwd)"
SHOTS="$HERE/../../site/assets/shots"
python3 servers.py &
SERVERS=$!
trap 'kill $SERVERS 2>/dev/null' EXIT INT TERM
sleep 1
TMP="$(mktemp -d)"
for lang in ru en; do
    if [ "$lang" = ru ]; then shop="$HERE/shop"; out="$SHOTS"; else shop="$HERE/shop-en"; out="$SHOTS/en"; fi
    mkdir -p "$out"
    # The window's language is the bridge's setting: set before the page loads again.
    cat > "$TMP/setup.js" <<EOF
localStorage.clear();
localStorage.setItem("collection", "$shop");
localStorage.setItem("env:$shop", "stage");
const x = new XMLHttpRequest();
x.open("POST", window.__TSUBAMEDO_BRIDGE__, false);
x.setRequestHeader("Content-Type", "application/json");
x.send(JSON.stringify({ method: "setLanguage", args: { speak: "$lang", chosen: "$lang" } }));
EOF
    for shot in hero checks compare code; do
        cat lib.js "$shot.js" > "$TMP/$shot.js"
        for theme in light dark; do
            swift ../screenshot.swift http://localhost:1430/bridge.html "$out/$shot-$theme.png" 1360 820 "$TMP/setup.js" "$TMP/$shot.js" "$theme"
        done
    done
done
rm -rf "$TMP"
