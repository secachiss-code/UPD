# I01: исходный аудит миграции и регрессий

Дата: 2026-10-03. Уровень исходного evidence: чтение кода до ограниченного исправления отдельным аудитором; **не воспроизведение тестами**. Runtime и тесты выполняет отдельный исполнитель по [контракту и допускам](I01-TEST-CONTRACT.md). Последующее [исправление C17](I01-MIGRATION-GUARD.md) удаляет опасный путь автоматической миграции и блокирует переход, а не реализует полноценный перенос.

## Исходный verdict

Production UPD→CM migration в текущем виде **NO-GO**. Это не вывод о качестве сетевого ядра и не утверждение, что пользовательские данные уже повреждены. Миграция на реальном хосте не выполнялась.

Таблица ниже фиксирует исходные defects и незакрытые требования. После C17 прежние мутации `migrate_upd()` удалены; свойства full migration по-прежнему не реализованы. Нельзя читать MIG-01/03/07 как утверждение, что защищённая текущая функция всё ещё останавливает legacy службы.

| ID | Проблема | Подтверждение | Необходимый результат |
|---|---|---|---|
| MIG-01 | После отключения legacy units возможна ошибка rename/install без восстановления прежнего состояния | `src/main.rs`, `migrate_upd()` и следующие ранние возвраты `cmd_install()` | Preflight до мутаций; транзакция/rollback или явный безопасный отказ неподдержанного перехода |
| MIG-02 | Ручные legacy binaries, desktop/app IDs и пользовательские panel references не переводятся | `migrate_upd()`; legacy генераторы в Git HEAD | Явный ownership-aware переход входов; исключить создание второго пустого UPD state |
| MIG-03 | Managed vendor unit допускает остановку имени при foreign effective `/etc` override | Поиск маркера в двух directories, затем `disable --now` по имени | Проверять эффективный unit и drop-ins; не затрагивать foreign resource |
| MIG-04 | Оба `/etc/upd.conf` и `/etc/cm.conf` не проверяются как конфликт до stop | Preflight directories и отдельный условный rename config | Конфликт явно отклонён до первой записи/остановки |
| MIG-05 | Legacy cron не включён в переход, может продолжать запускать старый UPD | Старый installer создавал `/etc/cron.d/upd`; новое `migrate_upd()` его не рассматривает | Проверяемая политика cron и binaries; запрет удаления по имени |
| MIG-06 | Env-isolated install пропускает production migration | `test_mode()` → ранний `return Ok(false)` | Actual-code injectable seam; fixture исполняет решение и операции миграции |
| MIG-07 | Legacy helper/дочерняя операция могут продолжать писать при переносе | Legacy `KillMode=process`; новый busy query использует CM socket после части install | Preflight legacy busy/unknown; не stop/rename во время операции |
| MIG-08 | Последующий install может писать через foreign targets/symlinks, а manual cleanup удаляет часть targets без ownership | `write_system_files()`, `write_host()`, `install_gui()`, `install_binary()`, `remove_manual_files()` | C10 охватывает всю установку; preflight/rollback legacy path сам по себе это не исправляет |

Главный preflight должен учитывать также сохранение default config в `main()` до `cmd_install()`. Guard только внутри `migrate_upd()` не доказывает отсутствие ранней записи.

Symlinks, masks/drop-ins, simultaneous old/new paths, package/manual ownership и ошибки `systemctl` входят в отрицательные сценарии. Исторический marker не заменяет фактическую ownership; известное имя не даёт права удалить пользовательский файл.

## Что уже предусмотрено

- Directory conflict old/new обнаруживается до stop.
- Три hooks удаляются только при точном совпадении с ожидаемым содержимым.
- `CM_*` env имеет приоритет над legacy `UPD_*` fallback.
- Успешный `rename` не преобразует credential содержимое; это ещё не rollback всей установки.

## Границы регрессионного исполнения

`tests/check_audit.sh` нельзя запускать целиком: через `tests/test_build_artifacts.py` он достигает `package.sh` с fake Cargo/nfpm и создаёт package artifacts. Этот путь исключён при действующем запрете пакета. Исполнитель выбирает непакетные Cargo/unit/fixture и launcher проверки по контракту C12/C13.

Postinstall скрывает код ошибки `cm install --package` через `|| true`; успешная транзакция пакетного менеджера поэтому не означает успешную миграцию. Сам package/postinstall в текущей работе не выполняется.

Окончательная приёмка зависит от независимого review и фактически исполненных сценариев C07–C11/C15–C16. До появления такого evidence миграция не помечается готовой; документ служит перечнем исходных defects для исправления и проверки.

Первое ограниченное исправление — C17: read-only fail-closed preflight блокирует legacy installation до любых записей, пока нет доказанной транзакции. Это защита от известного unsafe перехода, а не готовая миграция. C07/C09 и общий ownership последующего install остаются отдельными незакрытыми результатами.
