#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != --in-builder ]; then
    out=$(realpath -m "${1:?usage: tools/appimage.sh OUT.AppImage}")
    base=${SNELLAR_BUILDER:-$HOME/.cache/snellar-builder}
    mkdir -p "$base"
    flock "$base/.build.lock" env CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" nice -n 19 tools/builder.sh run /work/tools/appimage.sh --in-builder
    appdir=$base/build/appimage/AppDir
    rm -f "$out"
    runtime=$base/build/runtime-x86_64
    [ -f "$runtime" ] || curl -fsSL --retry 3 -o "$runtime" https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
    nice -n 19 env ARCH=x86_64 appimagetool --no-appstream --runtime-file "$runtime" "$appdir" "$out" >/dev/null
    printf 'AppImage runtime (type2-runtime, MIT, statically linked libfuse LGPL-2.1, squashfuse BSD-2-Clause) SHA-256: %s\n' "$(sha256sum "$runtime" | cut -d' ' -f1)"
    sync "$out"
    du -h "$out"
    exit 0
fi

mpv_top=/opt/build/mpv
tools/mpv-build.sh "$mpv_top"

cargo build --release --locked

qt_plugins=$(qmake6 -query QT_INSTALL_PLUGINS)
qt_qml=$(qmake6 -query QT_INSTALL_QML)
qt_libexec=$(qmake6 -query QT_INSTALL_LIBEXECS)
qt_libs=$(qmake6 -query QT_INSTALL_LIBS)
appdir=/opt/build/appimage/AppDir
licenses=$appdir/usr/share/licenses/snellar
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/qt6/plugins" "$appdir/usr/lib/qt6/qml" "$licenses"

cp /opt/build/target/release/snellar "$appdir/usr/bin/snellar"
patchelf --remove-rpath "$appdir/usr/bin/snellar"
cp LICENSE "$licenses/LICENSE"
cp packaging/snellar.desktop "$appdir/snellar.desktop"
cp packaging/icons/hicolor/256x256/apps/snellar.png "$appdir/snellar.png"
ln -s snellar.png "$appdir/.DirIcon"
mkdir -p "$appdir/usr/share/icons"
cp -r packaging/icons/hicolor "$appdir/usr/share/icons/"

for group in platforms wayland-shell-integration wayland-graphics-integration-client wayland-decoration-client \
    xcbglintegrations imageformats iconengines platforminputcontexts tls networkinformation; do
    [ -d "$qt_plugins/$group" ] && cp -r "$qt_plugins/$group" "$appdir/usr/lib/qt6/plugins/"
done

"$qt_libexec/qmlimportscanner" -rootPath qml -importPath "$qt_qml" \
    | jq -r '.[] | select(.type == "module" and .path != null) | .path' | sort -u \
    | while read -r module; do
        case "$module" in "$qt_qml"/*) ;; *) continue ;; esac
        target="$appdir/usr/lib/qt6/qml/${module#"$qt_qml"/}"
        mkdir -p "$(dirname "$target")"
        [ -e "$target" ] || cp -r "$module" "$target"
    done

cp -a "$mpv_top/bundle/lib" "$appdir/usr/lib/mpv"
cat > "$appdir/usr/bin/mpv" <<'EOF'
#!/bin/sh
lib=$(dirname "$(readlink -f "$0")")/../lib/mpv
export SPA_PLUGIN_DIR="$lib/spa-0.2" PIPEWIRE_MODULE_DIR="$lib/pipewire-0.3" PIPEWIRE_CONFIG_DIR="$lib/pipewire-conf" ALSA_CONFIG_DIR="$lib/alsa"
for dir in /usr/lib/x86_64-linux-gnu/alsa-lib /usr/lib64/alsa-lib /usr/lib/alsa-lib; do
    if [ -d "$dir" ]; then
        export ALSA_PLUGIN_DIR="$dir"
        break
    fi
done
exec "$lib/mpv" "$@"
EOF
chmod +x "$appdir/usr/bin/mpv"

host_only='^(ld-linux|libc\.|libm\.|libdl\.|libpthread\.|librt\.|libresolv\.|libutil\.|libgcc_s\.|libstdc\+\+\.|libz\.|libzstd\.|libGL|libEGL|libOpenGL|libgbm|libdrm|libwayland-|libX|libxcb\.|libxcb-(dri|glx|present)|libdbus-1|libsystemd|libudev|libasound|libpulse|libglib-|libgobject-|libgio-|libgmodule-|libgthread-|libfontconfig|libfreetype|libnss|libnspr|libcrypto|libssl)'

needed() {
    objdump -p "$1" 2>/dev/null | awk '$1 == "NEEDED" {print $2}'
}

locate() {
    for dir in "$qt_libs" /lib/x86_64-linux-gnu /usr/lib/x86_64-linux-gnu; do
        [ -e "$dir/$1" ] && { echo "$dir/$1"; return 0; }
    done
    return 1
}

copied=0
while :; do
    before=$copied
    while read -r name; do
        [[ $name =~ $host_only ]] && continue
        [ -e "$appdir/usr/lib/$name" ] && continue
        path=$(locate "$name") || { echo "no $name for the AppImage" >&2; exit 1; }
        cp -L "$path" "$appdir/usr/lib/$name"
        copied=$((copied + 1))
    done < <(find "$appdir/usr/bin/snellar" "$appdir/usr/lib" -path "$appdir/usr/lib/mpv" -prune -o -type f \( -name '*.so*' -o -name snellar \) -print \
        | while read -r file; do needed "$file"; done | sort -u)
    [ "$copied" = "$before" ] && break
done

outside=$(ldd "$appdir/usr/lib/mpv/mpv" | awk '/=> \// {print $3}' | grep -vE "^$appdir/usr/lib/mpv/|/(libc|libm|libdl|libpthread|librt|libresolv|libgcc_s|libstdc\+\+)\.so" || true)
[ -z "$outside" ] || { echo "mpv reaches outside the bundle: $outside" >&2; exit 1; }

for required in usr/lib/libxcb-cursor.so.0 usr/lib/qt6/plugins/platforms/libqxcb.so usr/lib/qt6/plugins/platforms/libqwayland-egl.so usr/lib/qt6/plugins/wayland-decoration-client/libbradient.so; do
    [ -e "$appdir/$required" ] || { echo "the AppImage must hold $required" >&2; exit 1; }
done

cat > "$appdir/usr/bin/qt.conf" <<'EOF'
[Paths]
Prefix = ..
Plugins = lib/qt6/plugins
QmlImports = lib/qt6/qml
EOF

cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/snellar" "$@"
EOF
chmod +x "$appdir/AppRun"

cp "$mpv_top/bundle/SOURCES.tsv" "$licenses/SOURCES.tsv"
cp -r "$mpv_top/bundle/licenses" "$licenses/components"
mkdir -p "$licenses/components/qt" "$licenses/components/fonts"
cp fonts/OFL-*.txt "$licenses/components/fonts/"
cp "$mpv_top/bundle/licenses/ffmpeg/COPYING.LGPLv3" "$mpv_top/bundle/licenses/ffmpeg/COPYING.GPLv3" "$licenses/components/qt/"
{
    cat packaging/THIRD-PARTY.txt
    printf '\nBuilt from these sources (tab separated: component, version, licence, URL, SHA-256):\n\n'
    cat "$mpv_top/bundle/SOURCES.tsv"
    printf '\nQt %s, LGPL-3.0-only, official binaries from download.qt.io, source https://download.qt.io/official_releases/qt/6.8/%s/single/:\n\n' "$(qmake6 -query QT_VERSION)" "$(qmake6 -query QT_VERSION)"
    cat /opt/qt-archives.txt
    printf '\nRust crates linked into the snellar binary (name, version, licence):\n\n'
    cargo metadata --locked --format-version 1 | jq -r '.packages[] | select(.source != null) | "\(.name)\t\(.version)\t\(.license // "see the crate")"' | sort -u
    printf '\nOther libraries copied from the Ubuntu 22.04 build base (Debian package, version):\n\n'
    for lib in "$appdir"/usr/lib/lib*; do
        name=$(basename "$lib")
        case "$name" in libQt6* | libicu*) continue ;; esac
        path=$(locate "$name") && dpkg -S "$(readlink -f "$path")" 2>/dev/null | cut -d: -f1 | xargs -r dpkg-query -W -f '${Package}\t${Version}\n' || true
    done | sort -u
} > "$licenses/THIRD-PARTY.txt"
