#!/bin/sh
# Формат файлов завершающего пакета. Каждая проверка — только если файл уже есть.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

fmt_if() {
  if [ -f "$1" ]; then
    rustfmt --edition 2024 --check "$1"
  fi
}

py_if() {
  if [ -f "$1" ]; then
    python3 -m py_compile "$1"
  fi
}

fmt_if src/status/live.rs
fmt_if src/status/resources.rs
fmt_if src/status/page.rs
fmt_if src/status/panel.rs
fmt_if src/profiles/edit.rs
fmt_if src/tui/tunnels.rs
fmt_if cosmic/src/tunnels.rs

if [ -d src/diag ]; then
  diag=$(find src/diag -name '*.rs' -print)
  if [ -n "$diag" ]; then
    # shellcheck disable=SC2086
    rustfmt --edition 2024 --check $diag
  fi
fi

# Рядом с теми же модулями может появиться Python. Нет файла — проверки нет.
py_if src/status/live.py
py_if src/status/resources.py
py_if src/status/page.py
py_if src/status/panel.py
py_if src/profiles/edit.py
py_if src/tui/tunnels.py
py_if cosmic/src/tunnels.py
if [ -d src/diag ]; then
  diag_py=$(find src/diag -name '*.py' -print)
  if [ -n "$diag_py" ]; then
    # shellcheck disable=SC2086
    python3 -m py_compile $diag_py
  fi
fi

[ ! -f tests/check_fin_tests.sh ] || sh tests/check_fin_tests.sh
