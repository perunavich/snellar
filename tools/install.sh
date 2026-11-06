#!/usr/bin/env bash
set -euo pipefail

repo=perunavich/snellar
asset=Snellar-x86_64.AppImage
release_url=${SNELLAR_RELEASE_URL:-https://github.com/$repo/releases/latest/download}
ytdlp_url=${SNELLAR_YTDLP_URL:-https://github.com/yt-dlp/yt-dlp/releases/latest/download}
script_url=https://raw.githubusercontent.com/$repo/main/tools/install.sh

data=${XDG_DATA_HOME:-$HOME/.local/share}
cache=${XDG_CACHE_HOME:-$HOME/.cache}
root=$HOME/.local/opt/snellar
versions=$root/versions
link=$HOME/.local/bin/snellar
desktop=$data/applications/snellar.desktop
icons=$data/icons/hicolor
marker='# snellar launcher'
tools=$data/snellar/tools

say() { printf '%s\n' "$*"; }
die() { printf 'snellar: %s\n' "$*" >&2; exit 1; }

usage() {
    say "Snellar installer. Everything goes into your home folder; sudo is not needed."
    say ""
    say "  install.sh                           install or update to the latest release"
    say "  install.sh tools                     only fetch or update yt-dlp"
    say "  install.sh uninstall                 remove the app, keep your library"
    say "  install.sh uninstall --delete-library  also delete your library and cache"
}

fetch() {
    if command -v curl >/dev/null; then
        curl -fL --retry 3 --progress-bar -o "$2" "$1"
    elif command -v wget >/dev/null; then
        wget -q --show-progress --progress=bar:force:noscroll -O "$2" "$1"
    else
        die "neither curl nor wget is installed"
    fi
}

checksum() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "neither sha256sum nor shasum is installed, cannot verify the download"
    fi
}

fuse_works() {
    [ -e /dev/fuse ] && { command -v fusermount3 >/dev/null || command -v fusermount >/dev/null; }
}

write_launcher() {
    local target=$1 tmp=$link.new
    mkdir -p "$(dirname "$link")"
    rm -f "$tmp"
    if fuse_works; then
        ln -s "$target" "$tmp"
    else
        say "FUSE is not available here, so Snellar will unpack itself into $root/run at the first start."
        printf '#!/bin/sh\n%s\nmkdir -p "%s/run"\nTMPDIR="%s/run" APPIMAGE_EXTRACT_AND_RUN=1 NO_CLEANUP=1 exec "%s" "$@"\n' "$marker" "$root" "$root" "$target" > "$tmp"
        chmod +x "$tmp"
    fi
    mv -T "$tmp" "$link"
}

ours() {
    if [ -L "$link" ]; then
        case "$(readlink "$link")" in "$root"/*) return 0 ;; esac
        return 1
    fi
    [ -f "$link" ] && grep -qxF "$marker" "$link"
}

own_ytdlp() {
    [ -e "$HOME/.local/bin/yt-dlp" ] || command -v yt-dlp >/dev/null
}

ytdlp_asset() {
    case "$(uname -m)" in
    x86_64) printf 'yt-dlp_linux' ;;
    aarch64) printf 'yt-dlp_linux_aarch64' ;;
    esac
}

install_ytdlp() {
    local asset want target=$tools/yt-dlp work
    asset=$(ytdlp_asset)
    [ -n "$asset" ] || { say "No yt-dlp build exists for $(uname -m), so Download will not work. Playing music is not affected."; return 0; }
    if [ ! -f "$target" ] && own_ytdlp; then
        say "Using the yt-dlp you already have."
        return 0
    fi
    mkdir -p "$tools"
    work=$(mktemp -d "$tools/.fetch.XXXXXX")
    say "Checking the latest yt-dlp at $ytdlp_url"
    if ! fetch "$ytdlp_url/SHA2-256SUMS" "$work/sums"; then
        rm -rf "$work"
        say "Could not reach the yt-dlp release, so it was not installed. Snellar tries again by itself. Playing music is not affected."
        return 0
    fi
    want=$(awk -v name="$asset" '$2 == name || $2 == "*" name { print $1; exit }' "$work/sums")
    if [ -z "$want" ]; then
        rm -rf "$work"
        say "The yt-dlp release does not list $asset, so nothing was installed."
        return 0
    fi
    if [ -f "$target" ] && [ "$(checksum "$target")" = "$want" ]; then
        rm -rf "$work"
        say "yt-dlp is up to date."
        return 0
    fi
    say "Downloading yt-dlp into $target"
    if ! fetch "$ytdlp_url/$asset" "$work/yt-dlp"; then
        rm -rf "$work"
        say "Could not download yt-dlp, so it was not installed. Snellar tries again by itself. Playing music is not affected."
        return 0
    fi
    if [ "$(checksum "$work/yt-dlp")" != "$want" ]; then
        rm -rf "$work"
        say "The downloaded yt-dlp failed its checksum, so it was not installed."
        return 0
    fi
    chmod +x "$work/yt-dlp"
    mv -f "$work/yt-dlp" "$target"
    rm -rf "$work"
    say "yt-dlp is installed. Nothing outside your home folder changed."
}

install_app() {
    [ "$(uname -m)" = x86_64 ] || die "only x86_64 builds exist, this machine is $(uname -m)"
    [ "$(id -u)" != 0 ] || die "run this as your own user, not as root: it installs into your home folder"

    mkdir -p "$versions"
    work=$(mktemp -d "$root/.install.XXXXXX")
    trap 'rm -rf "$work"' EXIT

    say "Downloading $release_url/$asset"
    fetch "$release_url/$asset.sha256" "$work/sum" || die "could not download the checksum"
    fetch "$release_url/$asset" "$work/$asset" || die "could not download the app"

    say "Verifying the checksum"
    local want have
    want=$(cut -d' ' -f1 "$work/sum")
    have=$(checksum "$work/$asset")
    [ -n "$want" ] && [ "$want" = "$have" ] || die "checksum mismatch: expected $want, got $have. Nothing was installed."
    chmod +x "$work/$asset"

    (
        cd "$work"
        "./$asset" --appimage-extract snellar.desktop >/dev/null
        "./$asset" --appimage-extract 'usr/share/icons/*' >/dev/null
    ) || die "the download does not unpack. Nothing was installed."

    local target=$versions/$asset
    say "Installing the app to $target"
    mv -f "$work/$asset" "$target"
    say "Adding the command $link"
    write_launcher "$target"

    say "Adding the menu entry $desktop"
    mkdir -p "$(dirname "$desktop")"
    sed -e "s|^Exec=.*|Exec=\"$link\"|" "$work/squashfs-root/snellar.desktop" > "$desktop"

    say "Adding the icons to $icons"
    (cd "$work/squashfs-root/usr/share/icons/hicolor" && find . -type f -name 'snellar.*' -print0 | while IFS= read -r -d '' icon; do
        mkdir -p "$icons/$(dirname "$icon")"
        cp -f "$icon" "$icons/$icon"
    done)
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" 2>/dev/null || true
    [ -f "$icons/index.theme" ] && command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$icons" 2>/dev/null || true

    install_ytdlp

    say ""
    say "Snellar is installed. Open it from the menu or run: $link"
    case ":$PATH:" in
    *":$(dirname "$link"):"*) ;;
    *) say "$(dirname "$link") is not in your PATH, so the short command snellar will not work in a terminal." ;;
    esac
    say "To remove it: curl -fsSL $script_url | bash -s -- uninstall"
    say "If Snellar does not show up in the menu, log out and back in."
}

uninstall_app() {
    local delete_library=$1
    if ours; then
        say "Removing the command $link"
        rm -f "$link"
    elif [ -e "$link" ] || [ -L "$link" ]; then
        say "Leaving $link: it does not point at this install"
    fi
    if [ -d "$root" ]; then
        say "Removing the app $root"
        rm -rf "$root"
    fi
    if [ -f "$desktop" ]; then
        say "Removing the menu entry $desktop"
        rm -f "$desktop"
        command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" 2>/dev/null || true
    fi
    if [ -d "$tools" ]; then
        say "Removing yt-dlp from $tools"
        rm -rf "$tools"
    fi
    if [ -d "$icons" ]; then
        say "Removing the icons from $icons"
        find "$icons" -type f -path '*/apps/snellar.*' -delete
    fi
    if [ "$delete_library" = yes ]; then
        say "Deleting your library $data/snellar"
        rm -rf "$data/snellar"
        say "Deleting the cache $cache/snellar"
        rm -rf "$cache/snellar"
    elif [ -d "$data/snellar" ]; then
        say "Your library stays in $data/snellar. To delete it too: uninstall --delete-library"
    fi
    say "Snellar is removed."
}

main() {
    case "${1:-install}" in
    install)
        [ $# -le 1 ] || { usage >&2; exit 2; }
        install_app
        ;;
    tools)
        [ $# -le 1 ] || { usage >&2; exit 2; }
        install_ytdlp
        ;;
    uninstall)
        case "${2:-}" in
        "") uninstall_app no ;;
        --delete-library) uninstall_app yes ;;
        *) usage >&2; exit 2 ;;
        esac
        ;;
    -h | --help | help)
        usage
        ;;
    *)
        usage >&2
        exit 2
        ;;
    esac
}

main "$@"
