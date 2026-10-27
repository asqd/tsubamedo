#!/bin/bash
# Builds what a release gives for the system it runs on, into build/release, with a .sha256
# beside each file. No signing certificates: the macOS app is signed ad hoc, the Windows
# installer not at all; docs/RELEASE.md says how they are opened.
#
#   macOS     TsubameDo-<версия>-macos-apple-silicon.dmg, …-macos-intel.dmg
#             tsubamedo-<версия>-macos-apple-silicon.tar.gz, …-macos-intel.tar.gz   the CLI
#             (one DMG per processor: 9 MB each instead of 18 MB for a universal one)
#   Windows   TsubameDo-<версия>-windows-x64-setup.exe
#             tsubamedo-<версия>-windows-x64.zip
#   Linux     TsubameDo-<версия>-linux-<arch>.AppImage, tsubamedo_<версия>_<deb-arch>.deb
#             tsubamedo-<версия>-linux-<arch>.tar.gz
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$("$ROOT/scripts/version.sh")"
OUT="$ROOT/build/release"
rm -rf "$OUT"
mkdir -p "$OUT"
cd "$ROOT"

case "$(uname -s)" in
    Darwin) system=macos; targets=(aarch64-apple-darwin x86_64-apple-darwin); bundles=dmg ;;
    MINGW* | MSYS* | CYGWIN*) system=windows; targets=(x86_64-pc-windows-msvc); bundles=nsis ;;
    *) system=linux; targets=("$(uname -m)-unknown-linux-gnu"); bundles=appimage,deb ;;
esac

(cd apps/desktop && { [[ -d node_modules ]] || npm ci; })
cli="$OUT/cli"
built=()
for target in "${targets[@]}"; do
    # The app; old installers go first, so the copies below find only this one. CI=true keeps
    # the DMG script from driving Finder to lay out the window.
    BUNDLE="$ROOT/target/$target/release/bundle"
    rm -rf "$BUNDLE"
    (cd apps/desktop && CI=true npx tauri build --target "$target" --bundles "$bundles")
    cargo build --release -p tsubamedo-cli --target "$target"
    rm -rf "$cli"
    mkdir -p "$cli"
    exe=tsubamedo
    [[ "$system" == windows ]] && exe=tsubamedo.exe
    cp "target/$target/release/$exe" "$cli/"
    case "$system" in
        macos)
            name=apple-silicon
            [[ "$target" == x86_64-* ]] && name=intel
            cp "$BUNDLE"/dmg/*.dmg "$OUT/TsubameDo-$VERSION-macos-$name.dmg"
            tar -czf "$OUT/tsubamedo-$VERSION-macos-$name.tar.gz" -C "$cli" tsubamedo
            built+=("TsubameDo-$VERSION-macos-$name.dmg" "tsubamedo-$VERSION-macos-$name.tar.gz")
            ;;
        windows)
            cp "$BUNDLE"/nsis/*-setup.exe "$OUT/TsubameDo-$VERSION-windows-x64-setup.exe"
            (cd "$cli" && powershell -NoProfile -Command "Compress-Archive -Path tsubamedo.exe -DestinationPath '$(cygpath -w "$OUT")\\tsubamedo-$VERSION-windows-x64.zip'")
            built+=("TsubameDo-$VERSION-windows-x64-setup.exe" "tsubamedo-$VERSION-windows-x64.zip")
            ;;
        linux)
            # AppImages name the architecture as the kernel does, packages as Debian does.
            arch="${target%%-*}"
            deb=amd64
            [[ "$arch" == aarch64 ]] && deb=arm64
            cp "$BUNDLE"/appimage/*.AppImage "$OUT/TsubameDo-$VERSION-linux-$arch.AppImage"
            cp "$BUNDLE"/deb/*.deb "$OUT/tsubamedo_${VERSION}_$deb.deb"
            tar -czf "$OUT/tsubamedo-$VERSION-linux-$arch.tar.gz" -C "$cli" tsubamedo
            built+=("TsubameDo-$VERSION-linux-$arch.AppImage" "tsubamedo_${VERSION}_$deb.deb" "tsubamedo-$VERSION-linux-$arch.tar.gz")
            ;;
    esac
done
rm -rf "$cli"

cd "$OUT"
for name in "${built[@]}"; do
    if command -v sha256sum > /dev/null; then sha256sum "$name" > "$name.sha256"; else shasum -a 256 "$name" > "$name.sha256"; fi
    echo "$name  $(du -h "$name" | cut -f1)  $(cut -d' ' -f1 "$name.sha256")"
done
