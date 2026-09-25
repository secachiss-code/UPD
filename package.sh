#!/usr/bin/env sh
# Пакеты upd: .pkg.tar.zst (Arch), .deb, .rpm → dist/. Нужен nfpm (скачивается сам, с проверкой sha256).
set -e
cd "$(dirname "$0")"
./build.sh >/dev/null 2>&1
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BIN=dist/upd-linux-amd64

NFPM_VER=2.47.0
TOOLS=${XDG_CACHE_HOME:-$HOME/.cache}/upd-build
NFPM=$(command -v nfpm || echo "$TOOLS/nfpm")
if [ ! -x "$NFPM" ]; then
	mkdir -p "$TOOLS"
	f=nfpm_${NFPM_VER}_Linux_x86_64.tar.gz
	u=https://github.com/goreleaser/nfpm/releases/download/v$NFPM_VER
	curl -fsSL --retry 4 --retry-all-errors -o "$TOOLS/$f" "$u/$f"
	curl -fsSL --retry 4 --retry-all-errors -o "$TOOLS/checksums.txt" "$u/checksums.txt"
	(cd "$TOOLS" && grep " $f\$" checksums.txt | sha256sum -c - && tar xzf "$f" nfpm)
fi

for target in arch deb rpm; do
	STAGE=$(mktemp -d)
	trap 'rm -rf "$STAGE"' EXIT
	install -Dm755 "$BIN" "$STAGE/usr/bin/upd"
	"$BIN" gen-files "$STAGE" "$target"
	# хуки пакетных менеджеров лежат вне /usr/lib — собираем их в extra/
	mkdir -p "$STAGE/extra" "$STAGE/usr/lib/NetworkManager"
	for d in usr/share etc; do
		[ -d "$STAGE/$d" ] && mkdir -p "$STAGE/extra/$d" && cp -a "$STAGE/$d/." "$STAGE/extra/$d/"
	done
	case $target in
	arch) pkg=archlinux ;;
	*) pkg=$target ;;
	esac
	# nfpm раскрывает переменные не во всех полях — подставляем путь сами
	sed "s|\${STAGE}|$STAGE|g" packaging/nfpm.yaml >"$STAGE/nfpm.yaml"
	VERSION=$VERSION "$NFPM" package -f "$STAGE/nfpm.yaml" -p "$pkg" -t dist/ >/dev/null
	rm -rf "$STAGE"
done
ls -1 dist/*"$VERSION"* 2>/dev/null || ls -1 dist/
