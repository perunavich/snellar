#!/bin/bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
base=${SNELLAR_BUILDER:-$HOME/.cache/snellar-builder}
image=ubuntu:22.04
qt_version=6.8.2
qt_repo=https://download.qt.io/online/qtsdkrepository/linux_x64/desktop/qt6_682/qt6_682
qt_modules=(qtbase qtdeclarative qtwayland qtsvg qttools icu)
qt_addons=(qtimageformats qtshadertools)
packages=(build-essential pkg-config ninja-build nasm cmake m4 git curl ca-certificates jq xz-utils bzip2 p7zip-full patchelf file
    python3 python3-jinja2 python3-pip libglib2.0-0 flex bison gettext libltdl-dev libcap-dev
    libgl-dev libegl-dev libopengl-dev libvulkan-dev libfontconfig1-dev libfreetype-dev libxkbcommon-dev libdbus-1-dev libwayland-dev
    libxcb1-dev libx11-dev libx11-xcb-dev libxcb-cursor0 libxcb-icccm4 libxcb-image0 libxcb-keysyms1 libxcb-render-util0 libxcb-shape0 libxcb-xkb1 libxkbcommon-x11-0 libxcb-xinerama0)
meson_version=1.10.1
rust_toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$repo/rust-toolchain.toml")

pull() {
    local name=${1%:*} tag=${1#*:} dir=$2 token auth digest
    [ -e "$dir/etc" ] && return 0
    mkdir -p "$dir"
    token=$(curl -fsSL "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/$name:pull" | jq -r .token)
    auth=("Authorization: Bearer $token")
    digest=$(curl -fsSL -H "${auth[0]}" -H 'Accept: application/vnd.oci.image.index.v1+json' -H 'Accept: application/vnd.docker.distribution.manifest.list.v2+json' \
        "https://registry-1.docker.io/v2/library/$name/manifests/$tag" | jq -r '.manifests[] | select(.platform.architecture == "amd64" and .platform.os == "linux") | .digest' | head -1)
    curl -fsSL -H "${auth[0]}" -H 'Accept: application/vnd.oci.image.manifest.v1+json' -H 'Accept: application/vnd.docker.distribution.manifest.v2+json' \
        "https://registry-1.docker.io/v2/library/$name/manifests/$digest" | jq -r '.layers[].digest' | while read -r layer; do
        curl -fsSL -H "${auth[0]}" "https://registry-1.docker.io/v2/library/$name/blobs/$layer" \
            | unshare --user --map-auto --map-root-user tar -xz -C "$dir" --exclude='dev/*'
    done
    echo "$name:$tag $digest" > "$dir.digest"
}

enter() {
    local root=$1 env=(); shift
    [ -n "${CARGO_BUILD_JOBS:-}" ] && env+=("CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS")
    unshare --user --map-auto --map-root-user --mount --pid --fork --kill-child -- bash -c '
set -e
root=$1 repo=$2 base=$3 common=$4; shift 4
mount --rbind "$root" "$root"
mkdir -p "$root/dev" "$root/proc" "$root/tmp" "$root/run" "$root/work" "$root/opt/build"
mount --rbind /dev "$root/dev"
mount -t proc proc "$root/proc"
mount -t tmpfs tmpfs "$root/tmp"
mount -t tmpfs tmpfs "$root/run"
[ -f /etc/resolv.conf ] && { rm -f "$root/etc/resolv.conf"; : > "$root/etc/resolv.conf"; mount --bind -o ro "$(readlink -f /etc/resolv.conf)" "$root/etc/resolv.conf"; }
mount --bind "$repo" "$root/work"
mount --bind "$base/build" "$root/opt/build"
case "$common" in "" | "$repo"/*) ;; *) mkdir -p "$root$common"; mount --bind -o ro "$common" "$root$common" ;; esac
exec chroot "$root" /usr/bin/env -i PATH=/opt/qt/bin:/opt/rust/cargo/bin:/usr/local/bin:/usr/bin:/bin HOME=/root LANG=C.UTF-8 TERM=dumb \
    CARGO_HOME=/opt/rust/cargo RUSTUP_HOME=/opt/rust/rustup QMAKE=/opt/qt/bin/qmake LD_LIBRARY_PATH=/opt/qt/lib CARGO_TARGET_DIR=/opt/build/target \
    "$@"
' _ "$root" "$repo" "$base" "$(git -C "$repo" rev-parse --path-format=absolute --git-common-dir)" "${env[@]}" "$@"
}

prepare() {
    local root=$base/root stamp
    stamp=$(printf '%s\n' "$image" "$qt_version" "${qt_modules[*]}" "${qt_addons[*]}" "${packages[*]}" "$meson_version" "$rust_toolchain" | sha256sum | cut -d' ' -f1)
    mkdir -p "$base/build"
    [ "$(cat "$base/stamp" 2>/dev/null)" = "$stamp" ] && return 0
    pull "$image" "$root"
    enter "$root" sh -ec "
export DEBIAN_FRONTEND=noninteractive
apt-get -qq update
apt-get -qq install -y --no-install-recommends ${packages[*]} >/dev/null
pip3 install --quiet --break-system-packages meson==$meson_version 2>/dev/null || pip3 install --quiet meson==$meson_version
mkdir -p /opt/qt-dl /opt/qt-unpack
cd /opt/qt-dl
fetch() {
    dir=\$1; pat=\$2
    file=\$(curl -fsSL \"$qt_repo/\$dir/\" | grep -oE \"href=\\\"[^\\\"]*\$pat[^\\\"]*\\.7z\\\"\" | sed 's/href=\"//;s/\"\$//' | head -1)
    [ -n \"\$file\" ] || { echo \"no \$pat in \$dir\" >&2; exit 1; }
    [ -e \"\$file\" ] || curl -fsSL -o \"\$file\" \"$qt_repo/\$dir/\$file\"
    7z x -y -o/opt/qt-unpack \"\$file\" >/dev/null
    echo \"\$file\" >> /opt/qt-archives.txt
}
: > /opt/qt-archives.txt
for m in ${qt_modules[*]}; do fetch qt.qt6.${qt_version//./}.linux_gcc_64 \"\$m-\"; done
for m in ${qt_addons[*]}; do fetch qt.qt6.${qt_version//./}.addons.\$m.linux_gcc_64 \"\$m-\"; done
mv /opt/qt-unpack/libicu* /opt/qt-unpack/lib/
ln -sfn /opt/qt-unpack /opt/qt
ln -sf qmake /opt/qt/bin/qmake6
cd /
rm -rf /opt/qt-dl
mkdir -p /opt/rust
export CARGO_HOME=/opt/rust/cargo RUSTUP_HOME=/opt/rust/rustup
curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain $rust_toolchain >/dev/null
"
    echo "$stamp" > "$base/stamp"
}

case "${1:-}" in
pull)
    pull "$2" "$3"
    ;;
run)
    shift
    prepare
    enter "$base/root" "$@"
    ;;
*)
    echo "usage: tools/builder.sh pull IMAGE DIR | run COMMAND..." >&2
    exit 2
    ;;
esac
