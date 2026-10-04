# I01: задание на симуляцию Grokbuild

Симуляцию выполняет Grokbuild по отдельному поручению пользователя. В этой среде VM нет; здесь simulation **не запускалась**. Не выполнять продуктовые команды и не описывать предсказание как фактический CLI/Linux результат. Реальные Linux/systemd/package DB/root CLI проверки отложены до установки собранного бинарника; host install сейчас не разрешён.

## Материалы и фиксация

Используй полный снимок текущего dirty working tree, а не только HEAD (`07ef2381fb962fe74958c504a896c584f6a4e4be`). Включи все изменённые, untracked и ignored build inputs, а также целиком `docs/design/cm-network-manager/i01-evidence/migration-implementation/` со всеми исходными `*.log`. Запиши manifest и SHA-256 переданных файлов; summary ниже не заменяет digest всего снимка. Не включай реальные secrets.

Итоговые hashes из [summary.json](i01-evidence/migration-implementation/summary.json): production binary `target/x86_64-unknown-linux-musl/debug/cm` = `b46c5d9381ddadc93b8a1f2b428ca71f9772ceef814795df2c44e4d179498b3b`; `src/main.rs` = `4d2b1d0a50cddd018584b1f19bb3e642c94d147d9f881481b800c499dcfe4be5`; `src/migration.rs` = `c055f68d54ce98d5f98072cf8424aedd3e0eb7e78c772d53a15eab043e165faa`; `src/migration/manual.rs` = `2484f8cd6a6b90ab36a31baaed11e574c286b58d8f21066e7745391f6d19c57b`; `src/migration/transaction.rs` = `3081eadf3c05af2ec207673daffd463fc503c0db261ae881023f4d44371709f5`; `tests/audit_i01_migration.rs` = `c17bb276c968ce42db91df5001f12b649c47bf7b244233f18ccd4f29eada4854`; pinned legacy input `dist/upd-linux-amd64` = `58520fbe70796d5bd018d56ad4ebbb120cea92c8edcb2cb52f5983ea694b14c2`.

## Запрос Grokbuild

Смоделируй текущую реализацию explicit manual UPD 0.2.7 migration по исходникам, указанному production binary semantics, тестам и сохранённому evidence. Не запускать binary/команды, systemd, package manager, сеть или host операции. Не создавать transcript будто они запускались. Результат каждого шага маркировать `SIMULATED`; отдельно перечислить неизвестное без настоящих Linux adapters.

Построй полную append-only хронологическую ленту. Для каждой строки укажи: исходное состояние, событие/планируемую команду, предсказанный exit/status и сообщение, изменения journal/files/services, проверенные invariants, кодовое основание, assumption и unmodeled adapter behavior. Без пропусков от initial state до rollback/recovery; не затирать failures при retry.

Моделируй synthetic root-owned manual 0.2.7 layout: pinned executable только как bytes, frozen owned units/hooks/resources, fake credentials/data, CM targets отсутствуют; package ownership adapter условно отвечает «not owned», systemd условно сообщает только generated units, no drop-ins/NeedDaemonReload=no, службы inactive, user sessions/helper processes отсутствуют. Это явно записать как предпосылки, а не доказанные свойства среды.

Обязательные проходы:

1. `cm migration plan`: ожидаемый read-only путь, ownership/conflict/service preflight и итоговая сводка; точное число шагов считать из смоделированного layout, не придумывать.
2. Успешный `cm migration apply`: transaction lock → legacy locks → generation recheck → old admission services off → install новых CLI/config/data/units/enablement → retire старых owned paths → reload → восстановление целевых service states → committed journal. Сверить полное сохранение байтов/uid/gid/mode, сохранённые credentials, private backup и отсутствие лишних service/file mutations. Не утверждать, что systemctl/package CLI реально это сделали.
3. Отказ до мутаций: конфликт старого/нового, foreign/edited path, занят legacy flock, либо `locked` veto. Ожидается отказ без применения сохранённых service states поверх внешней операции; источники и inode должны остаться прежними. Разделить отказ до создания журнала и `mutations_started=false` compensation по коду.
4. Ошибка после начала мутаций: используйте test Observer из migration tests для ошибки на `after-files-0` и проследите обратную компенсацию, reload и возврат service states. Это тестовая инъекция, не production CLI flag.
5. Аварийное завершение после durable boundary: смоделируйте процесс, оставивший незавершённый journal/partial state, затем новый процесс `cm migration recover`; покажите проверку backup digests, восстановление, terminal `RolledBack` и повторяемость recover. Не приравнивать exception/обычную ошибку к process crash.
6. Повреждённый backup, foreign edit или unknown staging file при recovery: отказ до первой компенсационной записи, посторонние данные и backup остаются сохранены, journal блокирует обычные root команды.
7. Успешный commit и повтор: `apply` проверяет committed generation/service state, не копирует credentials снова; recovery committed journal не меняет состояние.
8. Обычные root `status`/`install` без migration opt-in при наличии legacy следов: актуальный guard должен fail closed до config/backend writes. Старые W0 ожидания FAIL для K1/K2/K4 не переносить: критерий теперь — отказ без мутаций. W0-1 сам был BLOCKED до старта команд.

Для каждого вывода процитируй файл/функцию и укажи, что останется неизвестно до установки бинарника: точные systemctl/loginctl результаты, поддержка package manager и тексты ownership ответов, PID/inode race, permissions/SELinux/FS behavior, daemon reload и реальные unit transitions. Не выводить PASS реальной миграции или acceptance I01 из моделирования.

Исторические результаты оставить отдельными записями ленты, не смешивать с симуляцией: W0-1 заблокирован `bwrap: Failed to create NETLINK_ROUTE socket: Operation not permitted`, K0–K6 не запускались ([status](i01-evidence/w0/w0-1/status.json)); первый fixture apply упал на root-file `ENOTDIR`, затем исправление дало 13/13; следующий expanded run нашёл удаление sentinel и дал 14/15, затем исправление; финальный fixture run — 15/15. Исходные логи находятся в полном каталоге migration evidence. Summary review timestamp: `2026-10-04T12:23:40Z`.

Верни simulation tape, таблицу предсказанных исходов, assumptions/unmodeled list, противоречия с исходниками и перечень конкретных проверок при установке собранного бинарника. Реальное поведение systemd/package ownership/root CLI остаётся `NOT_RUN` до отдельного разрешённого запуска. Simulation сама по себе не закрывает I01.T04, не принимает I01 целиком и не открывает I02.
