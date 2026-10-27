#!/bin/bash
# One version for every build: the core and the CLI (workspace.package.version in Cargo.toml),
# the app (apps/desktop/src-tauri/tauri.conf.json) and the interface (apps/desktop/package.json).
#
#   scripts/version.sh             prints the version, or fails when the files disagree
#   scripts/version.sh 0.2.0       sets it everywhere
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

found() {
    sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/ядро и CLI (Cargo.toml): \1/p' Cargo.toml
    sed -n 's/^  "version": "\(.*\)",/приложение (apps\/desktop\/src-tauri\/tauri.conf.json): \1/p' apps/desktop/src-tauri/tauri.conf.json
    sed -n '1,5s/^  "version": "\(.*\)",/интерфейс (apps\/desktop\/package.json): \1/p' apps/desktop/package.json
}

if [[ $# -eq 0 ]]; then
    versions=$(found)
    distinct=$(echo "$versions" | sed 's/.*: //' | sort -u)
    if [[ $(echo "$distinct" | wc -l) -ne 1 ]]; then
        echo "ошибка: версии расходятся:" >&2
        echo "$versions" >&2
        echo "Задайте одну: scripts/version.sh X.Y.Z" >&2
        exit 1
    fi
    echo "$distinct"
    exit 0
fi

VERSION="$1"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "ошибка: версия — это X.Y.Z, а не $VERSION" >&2; exit 1; }
sed -i.bak '/^\[workspace.package\]/,/^\[/s/^version = ".*"/version = "'"$VERSION"'"/' Cargo.toml
sed -i.bak 's/^  "version": ".*",/  "version": "'"$VERSION"'",/' apps/desktop/src-tauri/tauri.conf.json
sed -i.bak '1,5s/^  "version": ".*",/  "version": "'"$VERSION"'",/' apps/desktop/package.json
# The lock file names the package's version at its top and for its root package.
sed -E -i.bak '1,10s/^( {2}| {6})"version": ".*",/\1"version": "'"$VERSION"'",/' apps/desktop/package-lock.json
rm -f Cargo.toml.bak apps/desktop/src-tauri/tauri.conf.json.bak apps/desktop/package.json.bak apps/desktop/package-lock.json.bak
# Cargo.lock names the workspace's own crates with their versions; reading the metadata updates it.
cargo metadata --format-version 1 > /dev/null
"$0"
