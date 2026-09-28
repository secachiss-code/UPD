#!/usr/bin/env sh
# Сборка статического бинарника upd (x86_64, musl): работает на любом Linux без зависимостей.
# Если есть системные библиотеки wayland/xkbcommon, рядом собирается интерфейс COSMIC (glibc): upd-cosmic.
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p dist
cp target/x86_64-unknown-linux-musl/release/upd dist/upd-linux-amd64
echo dist/upd-linux-amd64
if [ "${UPD_NO_GUI:-0}" != 1 ] && pkg-config --exists xkbcommon wayland-client 2>/dev/null; then
	(cd cosmic && cargo build --release)
	cp cosmic/target/x86_64-unknown-linux-gnu/release/upd-cosmic dist/upd-cosmic-linux-amd64
	echo dist/upd-cosmic-linux-amd64
fi
