#!/usr/bin/env sh
# Fresh artifacts and their explicit manifest; logs remain available on failure.
set -eu
cd "$(dirname "$0")"
mkdir -p dist
build_dir=$(mktemp -d "$PWD/dist/.build.XXXXXX")
manifest=${UPD_BUILD_MANIFEST:-"$PWD/dist/build-manifest.tsv"}
rm -f "$manifest" dist/upd-linux-amd64 dist/upd-cosmic-linux-amd64
cli_target=x86_64-unknown-linux-musl
gui_target=x86_64-unknown-linux-gnu
case "$cli_target" in x86_64-*) arch=amd64 ;; *) echo "unsupported target: $cli_target" >&2; exit 1 ;; esac
compile() {
    project=$1
    target=$2
    log=$3
    if ! (cd "$project" && cargo build --release --locked --target "$target") >"$log" 2>&1; then
        cat "$log" >&2
        echo "build log: $log" >&2
        exit 1
    fi
}
compile . "$cli_target" "$build_dir/cli.log"
cli="$build_dir/upd-linux-$arch"
cp "target/$cli_target/release/upd" "$cli"
report=$("$cli" --version)
case "$report" in 'upd '*) version=${report#upd } ;; *) echo "invalid build version: $report" >&2; exit 1 ;; esac
case "$version" in ''|*[!0-9A-Za-z.+-]*) echo "invalid build version" >&2; exit 1 ;; esac
gui=
if [ "${UPD_NO_GUI:-0}" != 1 ] && pkg-config --exists xkbcommon wayland-client 2>/dev/null; then
    compile cosmic "$gui_target" "$build_dir/gui.log"
    gui="$build_dir/upd-cosmic-linux-$arch"
    cp "cosmic/target/$gui_target/release/upd-cosmic" "$gui"
    gui_report=$("$gui" --version)
    [ "$gui_report" = "upd-cosmic $version" ] || { echo "CLI/GUI build versions disagree" >&2; exit 1; }
fi
manifest_tmp=$(mktemp "${manifest}.XXXXXX")
{
    printf 'version\t%s\narch\t%s\ncli_target\t%s\ncli\t%s\n' "$version" "$arch" "$cli_target" "$cli"
    printf 'cli_log\t%s\n' "$build_dir/cli.log"
    if [ -n "$gui" ]; then printf 'gui_target\t%s\ngui\t%s\ngui_log\t%s\n' "$gui_target" "$gui" "$build_dir/gui.log"; fi
} >"$manifest_tmp"
mv "$manifest_tmp" "$manifest"
cp "$cli" "dist/upd-linux-$arch"
echo "dist/upd-linux-$arch"
if [ -n "$gui" ]; then cp "$gui" "dist/upd-cosmic-linux-$arch"; echo "dist/upd-cosmic-linux-$arch"; fi
echo "build manifest: $manifest"
