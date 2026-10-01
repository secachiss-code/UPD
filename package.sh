#!/usr/bin/env sh
# Пакеты upd: .pkg.tar.zst (Arch), .deb, .rpm → dist/. Нужен nfpm (скачивается сам, с проверкой sha256).
set -e
cd "$(dirname "$0")"
mkdir -p dist
PACKAGE_STAGE=$(mktemp -d "$PWD/dist/.package.XXXXXX")
trap 'rm -rf "$PACKAGE_STAGE"' 0
BUILD_LOG=$(mktemp "$PWD/dist/package-build.XXXXXX.log")
MANIFEST="$PACKAGE_STAGE/artifacts.tsv"
if ! UPD_BUILD_MANIFEST="$MANIFEST" ./build.sh >"$BUILD_LOG" 2>&1; then
    cat "$BUILD_LOG" >&2
    echo "build log: $BUILD_LOG" >&2
    exit 1
fi
manifest_value() { awk -F '\t' -v key="$1" '$1 == key {print $2}' "$MANIFEST"; }
VERSION=$(manifest_value version)
ARCH=$(manifest_value arch)
BIN=$(manifest_value cli)
GUI_BIN=$(manifest_value gui)
[ -n "$VERSION" ] && [ "$ARCH" = amd64 ] && [ -x "$BIN" ] || { echo "invalid build manifest" >&2; exit 1; }
if [ "${UPD_NO_GUI:-0}" = 1 ]; then GUI_BIN=; fi
if [ -n "$GUI_BIN" ]; then [ -x "$GUI_BIN" ] || { echo "missing fresh GUI artifact" >&2; exit 1; }; fi
mkdir -p "$PACKAGE_STAGE/packages"

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
	STAGE="$PACKAGE_STAGE/$target"
	mkdir -p "$STAGE"
	install -Dm755 "$BIN" "$STAGE/usr/bin/upd"
	"$BIN" gen-files "$STAGE" "$target"
	# хуки пакетных менеджеров лежат вне /usr/lib — собираем их в extra/
	mkdir -p "$STAGE/extra/usr/share" "$STAGE/extra/etc" "$STAGE/usr/lib/NetworkManager"
	for d in usr/share etc; do
		[ -d "$STAGE/$d" ] && mkdir -p "$STAGE/extra/$d" && cp -a "$STAGE/$d/." "$STAGE/extra/$d/"
	done
	case $target in
	arch) pkg=archlinux ;;
	*) pkg=$target ;;
	esac
	# nfpm раскрывает переменные не во всех полях — подставляем путь сами
	escaped_stage=$(printf '%s' "$STAGE" | sed 's/[\\&|]/\\&/g')
	sed "s|\${STAGE}|$escaped_stage|g" packaging/nfpm.yaml >"$STAGE/nfpm.yaml"
	VERSION=$VERSION "$NFPM" package -f "$STAGE/nfpm.yaml" -p "$pkg" -t "$PACKAGE_STAGE/packages/"
	rm -rf "$STAGE"
	# интерфейс COSMIC — отдельный пакет, если build.sh его собрал
	if [ -n "$GUI_BIN" ]; then
		escaped_gui=$(printf '%s' "$GUI_BIN" | sed 's/[\\&|]/\\&/g')
		sed "s|\${GUI_BIN}|$escaped_gui|g" packaging/nfpm-cosmic.yaml >"$PACKAGE_STAGE/nfpm-cosmic.yaml"
		VERSION=$VERSION "$NFPM" package -f "$PACKAGE_STAGE/nfpm-cosmic.yaml" -p "$pkg" -t "$PACKAGE_STAGE/packages/"
	fi
done
for artifact in "$PACKAGE_STAGE/packages/"*; do
    [ -f "$artifact" ] || continue
    cp "$artifact" dist/
    echo "dist/$(basename "$artifact")"
done

# Keep the exact immutable inputs after private staging has been removed.
cp "$MANIFEST" dist/package-manifest.tsv
