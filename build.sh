#!/usr/bin/env sh
# Сборка статического бинарника upd (x86_64, musl): работает на любом Linux без зависимостей.
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p dist
cp target/x86_64-unknown-linux-musl/release/upd dist/upd-linux-amd64
echo dist/upd-linux-amd64
