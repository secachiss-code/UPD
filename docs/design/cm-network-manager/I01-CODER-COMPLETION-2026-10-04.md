# I01.T05: результат кодовой части и проверки при установке

Sol завершил кодовую часть I01 перед переходом к I02: startup guard, явная миграция ручного UPD0.2.7, durable backup/journal/recovery, filesystem/process/service проверки и исправления по двум лентам. Исторические network unknown, C14 attribution и real migration acceptance не объявляются закрытыми.

## Свежий артефакт

`cargo build --offline --locked --bin cm` прошёл. Новый бинарник: `target/x86_64-unknown-linux-musl/debug/cm`, SHA-256 `9d80a30f4eb7762255b9b45534ff49dca67c58a63f201d64f6fa01394103ade5`. Предыдущий бинарник сохранён в `.audit/build-snapshots/cm-b46c5d9381ddadc93b8a1f2b428ca71f9772ceef814795df2c44e4d179498b3b`, чтобы старое evidence не потеряло свой артефакт. Это CLI build, не package build/installation.

COSMIC check `cargo check --offline --locked --target x86_64-unknown-linux-gnu --tests` прошёл. Shell syntax и AST help/launcher/regression-driver проверены. Runtime tests не запускались. [Сводка и source hashes](i01-evidence/coder-t05-build-2026-10-04/summary.json), исходные stdout/stderr рядом.

## Последовательный регистр T05

| Область | Подготовленная проверка | Текущий статус |
|---|---|---|
| Миграция, faults/recovery | audit_i01_migration + migration::manual unit tests | COMPILE_PASS; новый runtime NOT_RUN |
| Legacy startup | audit_i01_install_guard c17a | COMPILE_PASS; прежний fixture PASS сохранён отдельно |
| Updates/prefetch/install | bin cm tests + backend/mirrors suites | COMPILE_PASS; новый runtime NOT_RUN |
| Снапшоты | extras suites | COMPILE_PASS; runtime NOT_RUN |
| Helper | helper unit suites и отдельные audit_contracts integration scenarios | COMPILE_PASS; runtime NOT_RUN |
| TUI | bin cm tui tests, включая host metrics | COMPILE_PASS; runtime NOT_RUN |
| Launcher/COSMIC | cosmic compile + test_tui_launcher.py AST | COMPILE_PASS/PARSE_PASS; runtime NOT_RUN |
| Live systemd/loginctl/package ownership | installation runbook ниже | NOT_RUN |
| Host TUN, parity, historical config | baseline report | прежние UNKNOWN/FAIL/BLOCKED сохраняются |

## После установки соответствующего бинарника

Подготовлен `tests/check_i01_regressions.py`: без `--run` он только выводит план. С `--run` требует непривилегированного исполнителя, digest установленного бинарника, source manifest и новый evidence directory. Проверяет соответствие артефакта исходникам, выполняет local fixture suites последовательно, сохраняет каждый stdout/stderr и не считает zero-tests PASS. Ошибка останавливает очередь, прежние неуспехи не перезаписываются.

Пример будущего запуска (сейчас НЕ исполнялся):

```sh
python3 tests/check_i01_regressions.py --run \
  --installed-binary /usr/local/bin/cm \
  --manifest docs/design/cm-network-manager/i01-evidence/coder-t05-build-2026-10-04/summary.json \
  --evidence /tmp/cm-i01-install-verification-NEW
```

Manifest должен принадлежать реально установленной сборке; после изменения исходников требуется новый build manifest. Эти local fixtures не запускают настоящий migration apply и не доказывают host services/network.

Отдельно при установке проверить реальными adapters: source/target inventory и owner/mode, pinned legacy binary/resources, package ownership, manager FragmentPath/DropInPaths/NeedDaemonReload, отсутствие legacy процессов и неизвестных unlinked executable, enabled/inactive VPN, вывод plan без записи, pending journal guard, recovery при прерывании на synthetic layout. Не делать crash injection на единственной рабочей установке. Для настоящих credentials сравнивать локально bytes/metadata/digests, не выносить material в evidence.

## Переход кодовой очереди

Последнее поручение пользователя требует продолжать всю последовательную работу кодера, реальные проверки оставляя до установки. Поэтому после завершения code/build/check-preparation I01 начинается кодовая подготовка модели I02.T01 → T02 → T03; затем storage T04. Это не runtime acceptance I01/G0 и не разрешение package/host/network actions. Исторические запреты начинать другие направления описывали прежнее поручение. Downstream acceptance по-прежнему зависит от реальных prerequisites; code progress учитывается отдельно.
