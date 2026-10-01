# Выполнение AUDIT-LUNA6-DAG

Задачи выполнялись по одной. Ниже сохранена история проверок; актуальное решение о внешней приёмке указано в конце.

| Узел | Состояние | Проверенное изменение |
| --- | --- | --- |
| D00 | Готово | Изолированные fixtures с RAII и воспроизведения контрактов |
| D01 | Готово | Ограниченные кадры, постоянный BufReader, ошибки протокола |
| D02 | Готово | Уникальные atomic-write temp, fsync файла и каталога, cleanup |
| D03 | Готово | Версия протокола, operation_id/prompt_id, проверка владельца и replay |
| D04 | Готово | Runner владеет child/PGID/readers; ограниченный drain и завершение |
| D05 | Готово | RAII подписок, обнаружение disconnect, квоты, bounded queues, deadline |
| D06 | Готово | Короткий одноразовый ввод через runner, повторная проверка владельца, безопасные диагностики |
| D07 | Готово | Межпроцессные транзакции конфига, проверка до записи, runtime status и lock order |
| D08 | Готово | Явный контекст пользователя из peer и CLI, ошибки пользовательского proxy |
| D14 | Готово | Poll обоих pipe, типизированные лимиты/timeout/cancel, собственный PGID, RAII reap, общий deadline probes |
| D09 | Готово | Отдельные фазы jobs, один worker + latest repeat, поколения/query/page, I/O внутри spawn_blocking, deadlines и backoff |
| D10 | Код и автотесты готовы | Прямой AUR для текущего UID, root→user, отказ чужому UID и root build; ручная Arch VM приёмка остаётся |
| D11 | Готово | Draft/committed, request IDs, busy controls, сохранение формы до accepted, общая numeric schema, переводы |
| D12 | Готово | Фазы операции, request-bound answer/cancel, follow-tail, VecDeque + byte budget, терминал для AUR/редакторов |
| D13 | Готово | EOF flush UTF-8, tabs/Unicode byte caps, bounded journal/stages, ANSI/OSC/alternate-screen |
| D15 | Готово | Один bounded reaper, spawn/exit errors в GUI, fallback только NotFound, HTTP/HTTPS URLs |
| D16 | Готово | Свежий manifest/staging, версии бинарников и target, --locked, logs, 4 stub packaging tests |
| D17 | Код и автоматическая приёмка готовы | 143 PNG, обязательный renderer smoke, keyboard/focus bounds, 12 TUI поверхностей, PTY resize/UTF-8/Ctrl+C; real compositor/Arch VM остаются |
| D18 | Локальные regression gates готовы | 7 ресурсных серий, 700 attach/drop, partial/slow reader/burst/start/cancel, RSS plateau, delayed fake auth, restart, concurrent CLI/helper; VM остаётся |
| D19 | Локальная интеграция готова; VM acceptance не закрыта | check_audit.sh прошёл, документация/реестр F01–F20 обновлены, protocol mismatch в обе стороны проверен; критерий «нет открытых P1» зависит от VM |

Последняя полная проверка после ревью: 149 библиотечных, 41 CLI, 24 интеграционных теста; 24 теста Cosmic; 4 packaging fixtures. Unix socket / PTY fixtures требуют запуска вне ограничений sandbox. Проверки пользуются фиктивными программами; настоящий VPN и пользовательские proxy-настройки не менялись.

Дополнительное поручение пользователя: после завершения всего плана проверить изменения, сделанные до текущей работы, и исправить обнаруженные проблемы. Проверка выполнена после локальной интеграции D19; [находки и исправления](AUDIT-LUNA6-PRIOR-REVIEW.md).

Порядок блокировок и контракт сохранения: [AUDIT-LUNA6-LOCKS.md](AUDIT-LUNA6-LOCKS.md).

D17: [снимки before/after и ручной чеклист](audit-luna6-acceptance/README.md). Матрица с проверкой focus/Enter прошла; ещё 19 Cosmic tests прошли (длительный hundred_ticks проверялся ранее). CLI содержит 36 тестов после двух acceptance fixtures.

D18: root suite прошла полностью: 149 lib + 36 CLI + 24 integration. Ресурсные измерения, бюджеты и границы mock auth: [RESOURCE-GATES.md](audit-luna6-acceptance/RESOURCE-GATES.md). Build/package fixtures: 4 passed.

D18: полная Cosmic suite — 22 passed, включая 30-секундный stalled worker и обязательный renderer smoke.

D19: ./tests/check_audit.sh полностью прошёл (--locked --offline, явные musl/GNU targets); docs links и diff whitespace чистые. Исторический probe помечен historical baseline. [Реестр F01–F20 и незавершённые критерии](AUDIT-LUNA6-FINDINGS.md). Локальные задачи плана выполнены; внешняя приёмка остаётся открытой. Затем выполнено последующее ревью кода из исходной базы, как просил пользователь.

Последующее ревью исходной базы: исправлены TUI child/reader ownership и PGID cleanup, deadline PTY input, bounded notifications и retry failure, atomic install бинарников. Targeted regression tests и финальный ./tests/check_audit.sh прошли после всех изменений.

Итог: 242 теста passed (149 lib + 41 CLI + 24 integration + 24 Cosmic + 4 packaging), матрица 143 PNG прошла отдельно. diff whitespace, scoped rustfmt, shell syntax и ссылки новых документов проверены. Коммитов и реальных системных изменений нет. Полностью закрытым план не назван: доступны только local/mock/headless gates, mandatory VM/compositor acceptance остаётся открытой.

Дополнительная настоящая release-сборка выявила E0080 в COSMIC Subscription::map: захват op generation допустим в Task::map, но запрещён в Subscription::map. Исправлено через .with(generation) и non-capturing map; 3 operation regressions прошли. В check_audit.sh добавлены обычные production binary builds, чтобы unit-test dead-code elimination больше не скрывала такую ошибку. Повторная release-сборка завершилась успешно (CLI musl и COSMIC GNU 0.2.7); запрос доступа к disposable VM остаётся без ответа.

Продолжение реальной приёмки: nfpm 2.47.0 создал шесть настоящих пакетов. Исправлен пустой корневой entry Arch-архива (tree в / заменён отдельными /usr/share и /etc); bsdtar читает пакеты, metadata и SHA-256 вложенных ELF проверяются tests/verify_real_packages.py. Дополнительно helper хранит connection JoinHandles и делает join перед повторным допуском; строгий ресурсный тест использует моментальный Threads ядра вместо несогласованного обхода /proc/task. Семь ресурсных серий и полный gate прошли после этих изменений: 242 теста, production CLI/COSMIC builds, shell syntax и scoped rustfmt. Musl permit удерживается также после join до исчезновения TID из ядра; restart fixture явно игнорирует PTY SIGHUP. Настоящая UPD_NO_GUI=1 упаковка выдала только три CLI-пакета при наличии прежних GUI artifacts.

Итог продолжения: release CLI/COSMIC 0.2.7 и все шесть реальных пакетов пересобраны после последних исправлений. python3 tests/verify_real_packages.py прошёл: metadata/architecture/SHA-256 ELF совпадают с dist/package-manifest.tsv; [результат](audit-luna6-acceptance/REAL-PACKAGES.json). Manifest точных immutable inputs сохраняется после удаления package staging. Четыре packaging fixtures повторно прошли после добавления manifest. Пакеты не устанавливались, VM acceptance остаётся открытой.

Приёмка по отдельному запросу пользователя: повторный полный gate с UPD_FULL_VISUAL=1 завершился успешно (242 теста и 143 PNG); шесть пакетов повторно проверены. Дополнительно выполнен реальный Wayland smoke всех пяти страниц release GUI на host COSMIC, в изолированном config/state и без backend/helper: окна живы, buffers attach/commit, panic нет. [Протокол и ограничения](audit-luna6-acceptance/README.md). VM и интерактивная приёмка остаются открытыми.

Решение пользователя 2026-10-01: обязательная внешняя VM/интерактивная приёмка снята как условие сборки и коммита. Невыполненные ручные проверки остаются отмеченными в протоколе. `CARGO_NET_OFFLINE=true ./build.sh` успешно собрал release CLI (musl, static PIE) и COSMIC GUI (GNU) версии 0.2.7 в `dist/`; версии обоих ELF проверены.

Финальный локальный gate перед коммитом: `./tests/check_audit.sh` прошёл вне sandbox (Unix sockets/PTY): 149 lib + 41 CLI + 24 integration + 24 COSMIC + 4 packaging = 242 теста, production builds, shell syntax и scoped rustfmt. [Журнал](audit-luna6-acceptance/FINAL-GATE.log). Первоначальный sandbox-прогон был ограничен запретом Unix sockets; повтор вне sandbox завершился с кодом 0.

После сообщения пользователя об ошибке при работе: выявлены старый установленный helper и апплеты на удалённых inode; исправлены обновление helper при установке, replay после разрыва связи, переход на операцию другого клиента, запуск окна после замены бинарника и idle-retry. Дополнительно устранён ложный resource-test failure при старте helper. Полный gate: 249 тестов passed. [Диагностика, исправления и ограничения установленной системы](audit-luna6-acceptance/RUNTIME-REVIEW.md).

2026-10-01: по запросу пользователя добавлены компактный прогресс и раскрываемый журнал в TUI, выделенные фазы AUR, кнопки ответов и ручной вход в полный терминал. В VPN добавлена общая настройка российских серверов для автовыбора (TUI + графический апплет), в апплете — отдельный выбор Auto. Проверены 256 тестов, снимки интерфейса и release CLI/COSMIC; установка пользователя не затрагивалась. [Изменения и протокол проверок](audit-luna6-acceptance/TUI-PROGRESS-VPN.md).
