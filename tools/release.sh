#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
root="$HOME/.local/opt/snellar"
versions="$root/versions"
link="$HOME/.local/bin/snellar"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor"
keep=5

activate() {
    ln -sfn "$1" "$link.new"
    mv -T "$link.new" "$link"
    readlink "$link"
}

case "${1:-}" in
list)
    ls -1t "$versions"
    echo "current: $(readlink "$link")"
    exit 0
    ;;
rollback)
    current=$(readlink "$link")
    previous=$(ls -1t "$versions"/*.AppImage | grep -vxF "$current" | head -1 || true)
    [ -n "$previous" ] || { echo "no previous version" >&2; exit 1; }
    touch "$previous"
    activate "$previous"
    exit 0
    ;;
"") ;;
*)
    echo "usage: tools/release.sh [list|rollback]" >&2
    exit 2
    ;;
esac

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "release only from main" >&2; exit 1; }
git diff --quiet HEAD -- src qml build.rs Cargo.toml Cargo.lock i18n fonts packaging \
    || { echo "uncommitted changes in the app" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
file="Snellar-$version-$(date +%Y%m%d-%H%M)-$(git rev-parse --short HEAD).AppImage"
mkdir -p "$versions" "$apps" "$(dirname "$link")"
tools/appimage.sh "$versions/$file.part"
mv "$versions/$file.part" "$versions/$file"
sync "$versions"
"$versions/$file" --appimage-offset >/dev/null
activate "$versions/$file"

assets=target/release-assets
asset=Snellar-x86_64.AppImage
rm -rf "$assets"
mkdir -p "$assets"
cp "$versions/$file" "$assets/$asset"
(cd "$assets" && sha256sum "$asset" > "$asset.sha256")
tar -cf "$assets/snellar-third-party-sources.tar" -C "${SNELLAR_BUILDER:-$HOME/.cache/snellar-builder}/build/mpv-src" .
sync "$assets"
echo "release assets in $assets: $(ls "$assets" | tr "\n" " ")"

sed -e "s|^Exec=.*|Exec=$link|" packaging/snellar.desktop > "$apps/snellar.desktop"
for dir in packaging/icons/hicolor/*/apps; do
    size=$(basename "$(dirname "$dir")")
    mkdir -p "$icons/$size/apps"
    cp "$dir"/snellar.* "$icons/$size/apps/"
done

ls -1t "$versions"/*.AppImage | tail -n +$((keep + 1)) | xargs -r rm -f
echo "installed $file, next launch uses it"
