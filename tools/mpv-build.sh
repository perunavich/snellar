#!/bin/bash
set -euo pipefail
top=${1:?usage: tools/mpv-build.sh DIR}
jobs=${CARGO_BUILD_JOBS:-2}
src=$top-src
work=$top-work
prefix=$top/prefix
bundle=$top/bundle
stamp=$(sha256sum "$0" | cut -d' ' -f1)
[ "$(cat "$bundle/.stamp" 2>/dev/null)" = "$stamp" ] && exit 0

sources='
zlib|1.3.2|zlib-1.3.2.tar.xz|d7a0654783a4da529d1bb793b7ad9c3318020af77667bcae35f95d0e42a792f3|https://github.com/madler/zlib/releases/download/v1.3.2/zlib-1.3.2.tar.xz|Zlib
mbedtls|3.6.7|mbedtls-3.6.7.tar.bz2|a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6|https://github.com/Mbed-TLS/mbedtls/releases/download/mbedtls-3.6.7/mbedtls-3.6.7.tar.bz2|Apache-2.0 OR GPL-2.0-or-later
freetype|2.15.0|freetype-2.15.0.tar.xz|93cd47eb719c325107f2d3cf087d06299e944c3b48a40b55537302304dea1dd5|https://download.savannah.gnu.org/releases/freetype/freetype-2.15.0.tar.xz|FTL OR GPL-2.0-or-later
fribidi|1.0.18|fribidi-1.0.18.tar.xz|1de5e8073d035388b71d0b2d452b1da1f8caeea8a7702997b401fe1b6250727f|https://github.com/fribidi/fribidi/releases/download/v1.0.18/fribidi-1.0.18.tar.xz|LGPL-2.1-or-later
harfbuzz|15.0.0|harfbuzz-15.0.0.tar.xz|968536e9abd782f07f7954044c0c4020199fcdd39b41d221ec42d2a3be4a44df|https://github.com/harfbuzz/harfbuzz/releases/download/15.0.0/harfbuzz-15.0.0.tar.xz|MIT
libass|0.17.5|libass-0.17.5.tar.xz|2dca25c0e0c837ddf00b52011b3f82cac1e4ddd3ad018227806b0c2288864acc|https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz|ISC
libplacebo|7.387.0|v7.387.0.tar.gz|f9e62d3774a9c4cfd198ec3abaa88294aaade2e31c04d5ab02bcf4abc98b1df5|https://github.com/haasn/libplacebo/archive/refs/tags/v7.387.0.tar.gz|LGPL-2.1-or-later
ffmpeg|7.1.5|ffmpeg-7.1.5.tar.xz|de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f|https://ffmpeg.org/releases/ffmpeg-7.1.5.tar.xz|LGPL-3.0-or-later as configured here
lua|5.2.4|lua-5.2.4.tar.gz|b9e2e4aad6789b3b63a056d442f7b39f0ecfca3ae0f1fc0ae4e9614401b69f4b|https://www.lua.org/ftp/lua-5.2.4.tar.gz|MIT
alsa-lib|1.2.16.1|alsa-lib-1.2.16.1.tar.bz2|f740db7f488255944ffd4428416ee3390a96742856916433df468c281436480e|https://www.alsa-project.org/files/pub/lib/alsa-lib-1.2.16.1.tar.bz2|LGPL-2.1-or-later
libsndfile|1.2.2|libsndfile-1.2.2.tar.xz|3799ca9924d3125038880367bf1468e53a1b7e3686a934f098b7e1d286cdb80e|https://github.com/libsndfile/libsndfile/releases/download/1.2.2/libsndfile-1.2.2.tar.xz|LGPL-2.1-or-later
pulseaudio|17.0|pulseaudio-17.0.tar.xz|053794d6671a3e397d849e478a80b82a63cb9d8ca296bd35b73317bb5ceb87b5|https://freedesktop.org/software/pulseaudio/releases/pulseaudio-17.0.tar.xz|LGPL-2.1-or-later
pipewire|1.6.11|1.6.11.tar.gz|f3f70c4af173cc93d05070a9d412efe3ef5889bdf4ac32298e2c0d9ae1350559|https://github.com/PipeWire/pipewire/archive/refs/tags/1.6.11.tar.gz|MIT
mpv|0.41.0|v0.41.0.tar.gz|ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209|https://github.com/mpv-player/mpv/archive/refs/tags/v0.41.0.tar.gz|GPL-2.0-or-later
'

marks=$top-steps
[ -d "$marks" ] || rm -rf "$work" "$top"
mkdir -p "$marks"
mkdir -p "$src" "$work" "$prefix/lib/pkgconfig" "$bundle"
export PKG_CONFIG_PATH=$prefix/lib/pkgconfig
export PKG_CONFIG_LIBDIR=$PKG_CONFIG_PATH:/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig
export CFLAGS="-O2 -fPIC -fstack-protector-strong -D_FORTIFY_SOURCE=2"
export CXXFLAGS=$CFLAGS
export LDFLAGS="-L$prefix/lib -Wl,--as-needed -Wl,-z,relro,-z,now"
export CPPFLAGS="-I$prefix/include"

unpack() {
    local line
    line=$(grep "^$1|" <<<"$sources")
    IFS='|' read -r name version file sum url licence <<<"$line"
    [ -f "$src/$file" ] || curl -fsSL --retry 3 -o "$src/$file" "$url"
    echo "$sum  $src/$file" | sha256sum -c --quiet
    rm -rf "${work:?}/$name"
    mkdir -p "$work/$name"
    tar -xf "$src/$file" -C "$work/$name" --strip-components=1 --no-same-owner
    cd "$work/$name"
    mkdir -p "$bundle/licenses/$name"
    find . -maxdepth 1 -type f \( -iname 'copying*' -o -iname 'license*' -o -iname 'copyright*' -o -iname 'lgpl*' -o -iname 'gpl*' -o -iname 'ftl*' \) -exec cp {} "$bundle/licenses/$name/" \;
    [ -d docs ] && find docs -maxdepth 1 -type f \( -iname 'ftl*' -o -iname 'license*' -o -iname 'gpl*' \) -exec cp {} "$bundle/licenses/$name/" \;
    return 0
}

built() { [ -e "$marks/$1" ]; }
mark() { touch "$marks/$1"; }

static_meson() {
    meson setup build --prefix="$prefix" --libdir=lib --buildtype=release --default-library=static --prefer-static --auto-features=disabled -Dwrap_mode=nodownload "$@" >/dev/null
    ninja -C build -j "$jobs" install >/dev/null
}

shared_meson() {
    meson setup build --prefix="$prefix" --libdir=lib --buildtype=release --default-library=shared --auto-features=disabled -Dwrap_mode=nodownload "$@" >/dev/null
    ninja -C build -j "$jobs" install >/dev/null
}

if ! built zlib; then
unpack zlib
./configure --prefix="$prefix" --static >/dev/null
make -j "$jobs" install >/dev/null

mark zlib
fi

if ! built mbedtls; then
unpack mbedtls
cmake -S . -B build -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_BUILD_TYPE=Release -DENABLE_TESTING=OFF -DENABLE_PROGRAMS=OFF -DUSE_STATIC_MBEDTLS_LIBRARY=ON -DUSE_SHARED_MBEDTLS_LIBRARY=OFF >/dev/null
cmake --build build -j "$jobs" --target install >/dev/null

mark mbedtls
fi

if ! built freetype; then
unpack freetype
static_meson -Dzlib=disabled -Dpng=disabled -Dbrotli=disabled -Dbzip2=disabled -Dharfbuzz=disabled

mark freetype
fi

if ! built fribidi; then
unpack fribidi
static_meson -Ddocs=false -Dbin=false -Dtests=false

mark fribidi
fi

if ! built harfbuzz; then
unpack harfbuzz
static_meson -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dicu=disabled -Dtests=disabled -Dbenchmark=disabled -Ddocs=disabled -Dutilities=disabled

mark harfbuzz
fi

if ! built libass; then
unpack libass
./configure --prefix="$prefix" --enable-static --disable-shared --disable-fontconfig --disable-require-system-font-provider --disable-asm >/dev/null
make -j "$jobs" install >/dev/null

mark libass
fi

if ! built libplacebo; then
unpack libplacebo
static_meson -Dvulkan=disabled -Dopengl=disabled -Dglslang=disabled -Dshaderc=disabled -Dlcms=disabled -Ddemos=false -Dtests=false

mark libplacebo
fi

if ! built ffmpeg; then
unpack ffmpeg
./configure --prefix="$prefix" --pkg-config-flags=--static --extra-cflags="$CFLAGS -I$prefix/include" --extra-ldflags="-L$prefix/lib" \
    --enable-static --disable-shared --enable-pic --disable-debug --disable-programs --disable-doc --disable-autodetect --disable-everything \
    --enable-version3 --enable-mbedtls --enable-zlib \
    --enable-protocol=file,pipe,fd,data,http,https,httpproxy,tcp,tls,crypto,subfile,hls \
    --enable-demuxer=matroska,mov,ogg,flac,mp3,aac,wav,aiff,ape,asf,hls,mpegts,w64,wv,caf,ac3,eac3,dts,mpc,mpc8,tta \
    --enable-decoder=opus,vorbis,aac,aac_latm,mp3,mp3float,mp2,mp2float,flac,alac,ape,wavpack,wmav1,wmav2,wmalossless,wmapro,ac3,eac3,dca,tta,mpc7,mpc8,'pcm*' \
    --enable-parser=aac,aac_latm,ac3,dca,flac,mpegaudio,opus,vorbis \
    --enable-bsf=aac_adtstoasc \
    --enable-filter='a*',volume,equalizer,bass,treble,bandpass,bandreject,highpass,lowpass,highshelf,lowshelf,extrastereo,dynaudnorm,loudnorm,sine,firequalizer,superequalizer,stereotools,stereowiden,pan,join,channelmap,channelsplit,compand,crossfeed,crystalizer,dcshift,deesser,earwax,haas,speechnorm,tremolo,vibrato,biquad,hdcd,lv2,replaygain \
    --enable-indev=lavfi >/dev/null
make -j "$jobs" install >/dev/null

mark ffmpeg
fi

if ! built lua; then
unpack lua
make -C src -j "$jobs" liblua.a MYCFLAGS="$CFLAGS -DLUA_USE_POSIX -DLUA_USE_DLOPEN" MYLIBS="-ldl" >/dev/null
cp src/liblua.a "$prefix/lib/"
cp src/lua.h src/luaconf.h src/lualib.h src/lauxlib.h src/lua.hpp "$prefix/include/"
cat > "$prefix/lib/pkgconfig/lua5.2.pc" <<EOF
Name: lua5.2
Description: Lua
Version: 5.2.4
Libs: -L$prefix/lib -llua -lm -ldl
Cflags: -I$prefix/include
EOF

mark lua
fi

if ! built alsa-lib; then
unpack alsa-lib
./configure --prefix="$prefix" --enable-shared --disable-static --disable-python --disable-topology --disable-old-symbols >/dev/null
make -j "$jobs" install >/dev/null

mark alsa-lib
fi

if ! built libsndfile; then
unpack libsndfile
./configure --prefix="$prefix" --enable-shared --disable-static --disable-external-libs --disable-mpeg --disable-full-suite --disable-alsa --disable-sqlite >/dev/null
make -j "$jobs" install >/dev/null

mark libsndfile
fi

if ! built pulseaudio; then
unpack pulseaudio
shared_meson -Ddaemon=false -Dclient=true -Ddoxygen=false -Dgcov=false -Dman=false -Dtests=false -Ddatabase=simple -Dipv6=true -Dhal-compat=false -Dadrian-aec=false

mark pulseaudio
fi

if ! built pipewire; then
unpack pipewire
shared_meson -Dsession-managers='[]' -Dexamples=disabled -Dtests=disabled -Dinstalled_tests=disabled -Ddocs=disabled -Dman=disabled \
    -Dspa-plugins=enabled -Dsupport=enabled -Daudioconvert=enabled -Dlegacy-rtkit=false -Dflatpak=disabled -Dpam-defaults-install=false -Drlimits-install=false -Dsnap=disabled

mark pipewire
fi

if ! built mpv; then
unpack mpv
meson setup build --prefix="$prefix" --libdir=lib --buildtype=release --default-library=static --prefer-static --auto-features=disabled -Dwrap_mode=nodownload \
    -Dgl=disabled -Dgpl=true -Dcplayer=true -Dlibmpv=false -Dbuild-date=false -Dtests=false -Dfuzzers=false \
    -Dlua=lua5.2 -Dzlib=enabled -Diconv=enabled -Dlibavdevice=enabled -Dalsa=enabled -Dpulse=enabled -Dpipewire=enabled \
    -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled \
    -Dc_args="$CFLAGS" -Dc_link_args="-Wl,-rpath,\$ORIGIN $LDFLAGS" -Dcpp_link_args="-Wl,-rpath,\$ORIGIN $LDFLAGS" >/dev/null
LD_LIBRARY_PATH="$prefix/lib:$prefix/lib/pulseaudio" ninja -C build -j "$jobs" install >/dev/null

mark mpv
fi

rm -rf "$bundle/lib"
mkdir -p "$bundle/lib"
cp "$prefix/bin/mpv" "$bundle/lib/mpv"
cp -a "$prefix"/lib/libasound.so* "$prefix"/lib/libpulse.so* "$prefix"/lib/pulseaudio/libpulsecommon-*.so "$prefix"/lib/libsndfile.so* "$prefix"/lib/libpipewire-0.3.so* "$bundle/lib/"
mkdir -p "$bundle/lib/spa-0.2" "$bundle/lib/pipewire-0.3" "$bundle/lib/pipewire-conf" "$bundle/lib/alsa"
cp -a "$prefix/lib/spa-0.2/support" "$prefix/lib/spa-0.2/audioconvert" "$bundle/lib/spa-0.2/"
for module in protocol-native client-node client-device adapter metadata session-manager rt; do
    cp -a "$prefix/lib/pipewire-0.3/libpipewire-module-$module.so" "$bundle/lib/pipewire-0.3/"
done
cp -a "$prefix/share/pipewire/client.conf" "$bundle/lib/pipewire-conf/"
cp -a "$prefix/share/alsa/." "$bundle/lib/alsa/"
find "$bundle/lib" -type f \( -name 'lib*.so*' -o -name mpv \) -exec strip --strip-unneeded {} +
find "$bundle/lib" -maxdepth 1 -type f \( -name 'lib*.so*' -o -name mpv \) -exec patchelf --set-rpath '$ORIGIN' {} +
find "$bundle/lib/spa-0.2" "$bundle/lib/pipewire-0.3" -type f -name '*.so' -exec patchelf --set-rpath '$ORIGIN/..:$ORIGIN/../..' {} +

{
    printf 'Component\tVersion\tLicence\tSource\tSHA-256\n'
    while IFS='|' read -r name version file sum url licence; do
        if [ -n "$name" ]; then printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$version" "$licence" "$url" "$sum"; fi
    done <<<"$sources"
} > "$bundle/SOURCES.tsv"
echo "$stamp" > "$bundle/.stamp"
rm -rf "$marks" "$work"
