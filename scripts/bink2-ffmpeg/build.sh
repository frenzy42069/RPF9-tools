#!/usr/bin/env bash
set -euo pipefail

mobipeg_revision=c9ccb15f9d7193ba13e2913eec6568acce8807b5
x264_revision=cdc336c268450e9403cb10e0cda94923f5a7881c
mobipeg_url=https://github.com/quatric/mobipeg.git
x264_url=https://github.com/quatric/x264.git

case ${1:-} in
    linux-x86_64)
        [[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || {
            echo 'The Linux build requires an x86-64 Linux host.' >&2
            exit 1
        }
        executable=ffmpeg
        x264_target=()
        ffmpeg_target=()
        strip_command=strip
        ;;
    windows-x86_64)
        cross_prefix=x86_64-w64-mingw32-
        executable=ffmpeg.exe
        x264_target=(--host=x86_64-w64-mingw32 --cross-prefix="$cross_prefix")
        ffmpeg_target=(--enable-cross-compile --target-os=mingw32 --arch=x86_64
            --cross-prefix="$cross_prefix" --extra-ldflags=-static)
        strip_command=${cross_prefix}strip
        ;;
    *)
        echo "Usage: $0 {linux-x86_64|windows-x86_64} OUTPUT_DIR" >&2
        exit 2
        ;;
esac
if [[ $# != 2 ]]; then
    echo "Usage: $0 {linux-x86_64|windows-x86_64} OUTPUT_DIR" >&2
    exit 2
fi

script_dir=$(cd -- "$(dirname -- "$0")" && pwd -P)
mkdir -p -- "$2"
output_dir=$(cd -- "$2" && pwd -P)
package=rpf9-bink2-ffmpeg-$1
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/rpf9-bink2-ffmpeg.XXXXXXXX")
trap 'rm -rf -- "$work_dir"' EXIT
source_dir=$work_dir/source
bundle=$work_dir/$package
prefix=$work_dir/install
mkdir -p "$source_dir" "$bundle" "$work_dir/build-x264" "$work_dir/build-ffmpeg"

fetch_source() {
    local name=$1 url=$2 revision=$3
    mkdir -p "$source_dir/$name"
    if [[ -f "$script_dir/$name/configure" ]]; then
        cp -R "$script_dir/$name/." "$source_dir/$name"
    else
        git init -q "$work_dir/$name"
        git -C "$work_dir/$name" fetch -q --depth 1 "$url" "$revision"
        git -C "$work_dir/$name" archive FETCH_HEAD | tar -x -C "$source_dir/$name"
    fi
}

fetch_source mobipeg "$mobipeg_url" "$mobipeg_revision"
fetch_source x264 "$x264_url" "$x264_revision"
cp "$script_dir/build.sh" "$script_dir/AGPL-3.0.txt" "$source_dir"
cp "$source_dir/AGPL-3.0.txt" "$source_dir/mobipeg/COPYING.GPLv3" "$bundle"
cp "$source_dir/mobipeg/LICENSE.md" "$bundle/FFmpeg-LICENSE.md"
cp "$source_dir/x264/COPYING" "$bundle/x264-COPYING"

x264_flags=(--prefix="$prefix" --enable-static --disable-cli --disable-opencl
    "${x264_target[@]}")
ffmpeg_flags=(--disable-autodetect --disable-everything --disable-doc --disable-network
    --disable-avdevice --disable-ffplay --disable-ffprobe --enable-gpl --enable-version3
    --enable-libx264 --enable-protocol=file --enable-demuxer=bink
    --enable-decoder=bink,bink2,binkaudio_dct,binkaudio_rdft
    --enable-encoder=libx264,aac --enable-muxer=mp4
    --enable-filter=aresample,aformat,format,scale,trim,atrim
    --pkg-config=pkg-config --pkg-config-flags=--static "${ffmpeg_target[@]}")
if ! command -v nasm >/dev/null; then
    x264_flags+=(--disable-asm)
    ffmpeg_flags+=(--disable-x86asm)
fi
jobs=${JOBS:-4}

(
    cd "$work_dir/build-x264"
    "$source_dir/x264/configure" "${x264_flags[@]}"
    make -j "$jobs"
    make install
)
(
    cd "$work_dir/build-ffmpeg"
    PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" PKG_CONFIG_PATH= \
        "$source_dir/mobipeg/configure" "${ffmpeg_flags[@]}" || {
            tail -80 ffbuild/config.log >&2
            exit 1
        }
    make -j "$jobs" "$executable"
    "$strip_command" "$executable"
    cp "$executable" "$bundle/"
)

{
    printf '%s\n' 'RPF9 Bink 2 companion' '' \
        'The FFmpeg executable includes an AGPL-3.0-or-later Bink 2 decoder.' \
        'The binary uses libx264 (GPL-2.0-or-later) for H.264 encoding.' \
        'The Independent JPEG Group (IJG) wrote portions of the FFmpeg source.' \
        'Run it with rpf9 export --ffmpeg=PATH or rpf9 mount --ffmpeg PATH.' \
        "Corresponding source: $package-source.tar.gz" ''
    printf 'mobipeg: %s %s\nx264: %s %s\n\n' \
        "$mobipeg_url" "$mobipeg_revision" "$x264_url" "$x264_revision"
    printf 'x264 configure flags:'
    printf ' %q' "${x264_flags[@]}"
    printf '\nFFmpeg configure flags:'
    printf ' %q' "${ffmpeg_flags[@]}"
    printf '\n'
} > "$bundle/README.txt"
cp "$bundle/README.txt" "$source_dir/BUILD.txt"

tar -C "$source_dir" -czf "$work_dir/$package-source.tar.gz" .
tar -C "$work_dir" -czf "$work_dir/$package.tar.gz" "$package"
mv -- "$work_dir/$package-source.tar.gz" "$work_dir/$package.tar.gz" "$output_dir/"
mkdir -p -- "$output_dir/$package"
cp "$bundle/"* "$output_dir/$package/"
printf 'Companion and source archives saved to %s\n' "$output_dir"
