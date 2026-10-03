#!/bin/sh
# Только при удалении пакета, не при обновлении:
# deb — "upgrade"/"failed-upgrade", rpm — $1=1, arch вызывает pre_remove только при удалении.
case "$1" in
upgrade | failed-upgrade | 1) exit 0 ;;
esac
CM_PACKAGE_SCRIPT=1 /usr/bin/cm uninstall --package || true
