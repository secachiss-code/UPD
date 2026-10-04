# I01: контракт исполнения проверок и допуски

Обновление исполнения 2026-10-04: startup guard C17a добавлен после нового поручения кодеру. Пять production-function fixture tests и 12 help CLI cases прошли отдельного исполнителя Luna; root CLI K0–K6 не запущены из-за запрета NETLINK_ROUTE socket в sandbox. Это не PASS C17a на уровне disposable CLI и не приёмка миграции. Актуальные результаты: [отчёт кодера](CODER-QUEUE-2026-10-04.md).

Обновление исполнения миграции 2026-10-04: общий production-used executor и explicit manual UPD 0.2.7 policy реализованы. Независимый исполнитель Luna получил 15/15 migration fixtures/fault/recovery tests, 5/5 guard, 4/4 language regressions, 12/12 help CLI и успешную offline binary build. [Реализация и ограничения](I01-MIGRATION-IMPLEMENTATION.md), [независимое ревью](i01-evidence/migration-implementation/review.md). Этот прогон не подменяет real systemd/package ownership adapters, root migration CLI и live helper races; полный I01 остаётся непринятым. Ниже сохранён исходный контракт и его исторические статусы.

Дата: 2026-10-03. Пользователь разрешил начать I01 и уточнил: координатор пишет контракты, **тесты выполняет отдельный исполнитель**. Этот контракт относится только к I01, не разрешает реализацию I02–I18. Пакет не собирать.

## Роли и фиксация результата

Координатор задаёт сценарии, допуски и критерии; читает исходники и отчёты, но не запускает тесты. Исполнитель отдельно проводит разрешённые проверки и сохраняет evidence. Рецензент отдельно проверяет исходники/контракты/evidence, не принимает выводы на доверии. Автор изменения не является единственным принимающим его тесты.

До проверки записать revision исходников, digest исполняемого файла/ядер, время UTC, target, argv без секретов, состояние prerequisites. Статусы: PASS, FAIL, BLOCKED, NOT_RUN, INCONCLUSIVE; только реально исполненный сценарий получает PASS. Старый отчёт не заменяет свежий запуск. Fixture/unit, private-network и live-host evidence различаются.

## Допуски исполнителя: права и границы

**Разрешено:** чтение исходников и установленного состояния; read-only systemd/API GET; локальные fixture/unit/regression tests; компиляция тестов и отладочного бинарника через Cargo без package/build scripts; создание временных файлов под `/tmp` и отдельных артефактов evidence в проекте; запуск worker без TUN/auto-route в изолированном окружении на выделенных loopback портах; ограниченные запросы к публичному test endpoint через такие workers.

Здесь «без package/build scripts» запрещает проектные `build.sh`, `package.sh` и пакетные сценарии; стандартные Cargo dependency `build.rs` при offline компиляции зафиксированного lockfile допустимы. Их выполнение не обозначает сборку дистрибутивного пакета. HEAD дополнить digest проверяемого dirty source tree/patch: один HEAD не описывает текущее дерево.

Root-чтение конфигов/журналов допустимо в ранее разрешённом пользователем read-only scope, с безопасным GUI askpass при необходимости. Пароль не передавать через чат/argv/logs. Escalation использовать только для конкретной разрешённой операции. Если инструмент отклонил её, записать BLOCKED и причину, не расширять полномочия обходным способом.

**Запрещено:** `package.sh`, `build.sh`, nfpm/pacman/apt/dnf/zypper install/update/remove; запуск общего `tests/check_audit.sh` без аудита его транзитивных действий; изменение действующих `/etc`, `/var/lib`, host routes/firewall/DNS, stop/restart установленного VPN или FlClash; запуск install/migrate на настоящем хосте; обновление ядра, подписки или запрос её URL; PUT/PATCH/DELETE к живому core API; сторонние конвертеры; изменение реального browser profile. Реальную миграцию проверять только в disposable sandbox/fixture filesystem.

Нельзя считать переменные `CM_STATE_DIR`/`UPD_STATE_DIR` доказательством изоляции миграции: исходный `migrate_upd()` до C17 в этом режиме пропускал работу. Текущий guard не исполняет миграцию и не обходится этими переменными. Для миграционных сценариев нужен фактически выполняющий миграцию test seam либо private mount namespace с безопасно заменёнными абсолютными путями и внешними командами. Имитация всего `migrate_upd()` не проверяет его correctness.

**Остановка:** любая попытка изменить живую сеть/службы, пересечение worker ports/resources, секрет в stdout/export, неожиданная запись вне выделенной области или неоднозначное ownership. Остановить только собственные временные процессы, записать факт и сохранить обезличенное evidence. Не уничтожать чужие ресурсы при cleanup.

## Допуски результата

- Потеря/порча credentials или пользовательских данных: **0**. Сравнивать полное локальное содержимое и permissions; секреты не экспортировать.
- Изменение чужих файлов/служб/ресурсов: **0**. Snapshot до/после должен совпадать за пределами явно принадлежащих тесту ресурсов.
- Секреты в evidence: **0**. Полные URL, UUID, passwords, keys, headers и профили не включать; логи обрабатывать перед записью в проект.
- PASS локальных регрессий: exit code 0 и все выбранные assertions выполнены; пропуск/timeout не PASS.
- Parity: полное определение узла идентично; никакие различия не допускаются в UUID, server, port, transport, SNI, REALITY, WS path/headers. Различия effective defaults должны быть отдельно перечислены и изменяться по одному.
- Время снимка не приравнивается к времени исторической ошибки. Без contemporaneous config причинность — INCONCLUSIVE.
- Скорость: заранее принятого числового преимущества CM нет. Сначала пилот, затем оценка разброса и предложение практического порога. 30 запросов — пилот, а не доказательство устойчивого p95 или превосходства.
- Проход пилота не закрывает TUN/host routing, если worker тестировался только через loopback proxy. Readiness API не заменяет remote connectivity.

## Проверки

| ID | Сценарий | Критерий | Уровень / evidence |
|---|---|---|---|
| C01 | Инвентаризация source / installed UPD / installed CM / FlClash core | Версии, локальные digests, UTC, пути, доступность API и process ownership отдельно; отсутствие CM явно указано | READ_ONLY: обезличенный baseline JSON |
| C02 | Снимки UPD source/runtime и FlClash | Количество и полное равенство узлов, изменения fields/counts, разрешённые global knobs и mtime; никаких credentials | READ_ONLY: сравнительная таблица |
| C03 | Ошибки действующего/исторического журнала | Классы DNS/dial/TLS/WS/timeout/TUN разделены; события связаны с поколением либо unknown | READ_ONLY: counts/time range и ограничения |
| C04 | Реальный выбранный узел | Только GET API: resolver chain до конкретного node, loop detection, actual core version; файловый config не объявляется active selection | READ_ONLY: типы/состояния цепочки без имён/адресов |
| C05 | План и подготовка parity | Сопоставимый узел, конфиги и test path; binary revisions известны; живой FlClash не остановлен; proxy-only scope подписан | ISOLATED: redacted methodology |
| C06 | A/B/A пилот одного транспорта | Не более 30 измеряемых запросов суммарно плюс до 3 warm-up; timeout до 10 с каждый; concurrency 1; общий deadline 15 мин. Записать success/error class/latency/TTFB если измеряется | ISOLATED NETWORK: observations; PASS только connectivity в этом scope, performance INCONCLUSIVE без статистического вывода |
| C07 | Миграция только old layout | Данные и права перенесены, state/config paths согласованы; после прерывания сохраняется восстанавливаемая копия | DISPOSABLE MIGRATION: before/after inventory |
| C08 | Одновременно old/new config или directories | Preflight отклоняет неоднозначный конфликт **до** остановки служб/rename/removal; old/new остаются нетронутыми | DISPOSABLE MIGRATION: negative scenario |
| C09 | Отказ на каждом шаге миграции | Fault injection stop/rename/write/hooks/new install: rollback либо доказанно восстанавливаемая транзакция; не оставлять бесконтрольную частичную миграцию | DISPOSABLE MIGRATION: fault matrix |
| C10 | Чужие/package-owned units, hooks, desktop files, binaries | Не удаляются/не заменяются по одному имени; распознаётся exact managed content/ownership | DISPOSABLE MIGRATION: foreign sentinel fixtures |
| C11 | Повторная миграция, legacy env, смешанный layout | Idempotency, однозначный приоритет CM/UPD env; unsafe смешанный layout диагностируется; нет silent смены credentials | FIXTURE/UNIT: actual code assertions |
| C12 | VPN local regressions | Parsing/UA fallback, failed import/validation preserves state, locks, redaction, readiness semantics; пройти существующие подходящие tests без сети | UNIT/FIXTURE: команды и summaries |
| C13 | Helper/backend/TUI/launcher regressions | Пройти подходящие существующие проверки без package scripts и host mutations; сохранить локали/permissions/argv boundaries | UNIT/FIXTURE: named assertions и failures |
| C14 | Cleanup после worker/test failure | Собственные процессы/сокеты/temp configs убраны; host services/routes/global config не изменены | ISOLATED: before/after и cleanup result |
| C15 | Старый helper занят, обновление выполняется, locks удерживаются | Миграция отклонена до stop/rename либо доказано безопасное завершение операции; PID/name не заменяют actual operation ownership | DISPOSABLE MIGRATION: BUSY/lock fixtures |
| C16 | Legacy CLI/desktop/applet/cron после перехода | Не создают второе пустое UPD состояние; package-owned/foreign entries сохранены; для ручных входов есть явный проверенный переход | DISPOSABLE MIGRATION + source audit: entry-point matrix |
| C17 | Временная защита неподдержанной legacy установки | Production-used preflight вызывается до config save/install mutation; old data/config/units/hooks/cron/desktop/helper socket, symlink и ошибки чтения дают информативный отказ. Fresh layout проходит preflight; нет env bypass | ACTUAL CODE UNIT + CLI fixture: ноль мутаций; **не** PASS миграции C07/C09 |
| C18 | Help после добавления предупреждения миграции | Исходный USAGE translation key сохранён; RU/en/de/it/zh/ar help остаётся в выбранной локали, отдельный migration notice локализован; не читать/менять реальный config в fixture | ACTUAL CLI + dictionary/unit: private CM_CONF с lang, отсутствие Config save/external commands |
| C19 | После нестабильного WS pilot: TCP/REALITY и контроль живого FlClash | Новый отдельный диагностический budget: максимум 30 measured A1/B/A2 + 3 warm-up на одном полном TCP/REALITY node из локального Fl source; дополнительно максимум 4 control requests через подтверждённый живой loopback proxy FlClash. Не повторять C06 WS; использовать два публичных HTTPS targets, raw bodies не экспортировать. Сравнить error stage/config/node/outer path; не выбирать winner по пилоту | HOST-CONDITIONED PROXY: bounded new experiment; I01, без host mutations/private subscription requests |
| C20 | Локализация общего curl35 после C19 | Максимум четыре последовательных запроса к gstatic: один через собственный native worker, один через собственный Fl worker на том же полном TCP/REALITY узле C19, один через подтверждённый live Fl proxy и один без explicit proxy. Сохранить CONNECT status, HTTP status, TLS verification result, безопасную причину curl и доступный error stage собственного core. Не повторять throughput/pilot | DIAGNOSTIC: четыре запроса, вывод о стадии отказа или явно unknown; без заявления о физическом обходе host TUN |

Если C05/C06 недоступны без изменения живой сети, C06 = BLOCKED, а не имитация успеха. Если FlClash core невозможно независимо запустить с тем же config, сравнение клиентов proxy path может быть дополнительным наблюдением, но не «same-config core A/B/A». При невозможности восстановить исторический конфиг C03 остаётся INCONCLUSIVE в причинности прошлого отказа.

C17 — минимальное безопасное исправление по независимому review. Оно блокирует автоматическую UPD→CM миграцию известных legacy layouts до появления проверенной транзакции; не преобразует файлы, не удаляет старые entries и не объявляет rollback готовым. Ограничение явно описать в help/документации. C07/C09/C15 остаются NOT_RUN/BLOCKED до полного executor/recovery. Этот рубеж безопасности не закрывает I01.T04 целиком.

C19 добавлен после реально исполненного C06: WS pilot дал 4/10, 2/10 и 0/10 успехов, все остальные curl exit 35. Новый эксперимент обоснован этим unresolved failure и последующим поручением пользователя «продолжай», а не автоматическим расширением прежнего budget. Общий допуск C19: concurrency 1, timeout <=10 с, deadline <=15 мин; два targets — `https://www.gstatic.com/generate_204` и `https://www.cloudflare.com/cdn-cgi/trace`, для каждого проверить TLS и ожидаемый HTTP status, тело trace не сохранять/печатать. Measured запросы распределяются одинаково между targets в каждой фазе (по пять), warm-up всего три. Live Fl proxy controls только при доказанном владении port/socket, до двух на target; если proxy unavailable, control BLOCKED. Настройку, выбранный узел и connections живого FlClash не менять.

TCP/REALITY node выбирается детерминированно из существующего локального source, фактическая live selection может оставаться unknown. Успешный control показывает работоспособность выбранного live proxy пути, не конкретного test node. Одинаковый Fl source node должен быть применён обоим temporary cores полностью, без преобразования UUID/keys/SNI/short-id; absent capability/invalid config — BLOCKED до remote requests. Native/Fl differing outcomes — гипотеза, требующая error evidence; оба failures при live controls PASS — повод локализовать node/transport/outer path, а не объявить весь VPN сломанным.

Уточнения ниже добавлены 2026-10-03 вместе с [проектом миграции](I01-MIGRATION-DESIGN.md). Они не расширяют сетевой бюджет и не поручают исполнителю новый прогон. Статус каждого уточнения — NOT_RUN, пока нет отдельного задания на disposable executor.

| ID | Сценарий | Критерий | Уровень / evidence |
|---|---|---|---|
| C10a | Fresh target CM является symlink, имеет чужого владельца или предок-symlink | Транзакция отклоняется до записи. Чужой target не заменяется и не получает содержимое CM | DISPOSABLE MIGRATION: sentinel до/после, NOT_RUN |
| C16a | Home и panel references вне известного inventory | Автоматический поиск и удаление по имени не выполняются. Неизвестный ярлык остаётся byte-for-byte | DISPOSABLE MIGRATION: sentinel в домашнем каталоге fixture, NOT_RUN |
| C17a | Legacy-след есть, команда не `install` | Корневая команда не создаёт `/etc/cm.conf`, не изменяет `/etc/upd.conf` и не пишет в каталог, который `state_dir()` выбрал бы как `/var/lib/upd`, включая lock-файлы `save()`. Fallback-чтение `/etc/upd.conf` не считается переносом | DISPOSABLE: исполнимая спецификация W0-1 K0–K6 в [контракте волны 0](WAVE0-REVIEW-AND-TEST-CONTRACT.md), NOT_RUN |

C10a закрывает хвост MIG-08, которого отказ legacy-путей не покрывает. C16a фиксирует границу текущего `inventory_legacy`: домашние ссылки вне списка не являются разрешением их трогать. C17a фиксирует чтение `src/main.rs` и `src/common.rs`: `preflight_install` стоит на `install`, а `conf_path()` и `state_dir()` на остальных командах могут указывать на дерево UPD.

C20 добавлен после завершённого C19: все 37 запросов завершились curl35, но драйвер не сохранил CONNECT status и подробную причину. Это отдельный инструментированный опыт для устранения недостатка evidence, не повтор пилота. Concurrency 1, timeout <=10 с, общий deadline <=5 мин, один публичный target `https://www.gstatic.com/generate_204`, body `/dev/null`. Запуск curl с `--disable` первым аргументом и явными proxy/noproxy; TLS verification сохраняется, `--insecure` запрещён. Из error text и логов экспортировать только безопасный класс и текст после удаления credentials/адресов узлов. Verbose/raw logs не сохранять в проекте. При неуверенной очистке оставить raw evidence только в private временной области и экспортировать UNKNOWN. Для direct запроса явно очистить proxy variables; это всё ещё host network path при работающем FlClash, не гарантированный обход TUN. Все прежние запреты и cleanup C14 сохраняются. Новые workers используют уже проверенные binary/config/node digests; живой core не перенастраивать.

## Порядок исполнения

1. Исполнитель: C01–C04 и чтение транзитивных test scripts; докладывает без изменений хоста.
2. Исполнитель: C12–C13 в offline/local режиме; package-generating tests исключает по имени с причиной.
3. По prerequisites: C05–C06 и C14, без stop живого VPN. До сети проверить no-TUN/no-auto-route и выделенность ports.
4. C07–C11/C15–C16: фактическая миграция только в disposable test seam; при отсутствии seam предложить минимальный refactor. До него эти проверки не обозначать PASS.
5. Рецензент проверяет выводы и raw evidence; координатор фиксирует gaps, исправления и statuses I01.T01–T05. Исправленный код повторно проверяет исполнитель, не координатор.
6. C10a, C16a и C17a остаются NOT_RUN. Текущее задание их не запускает и не считает покрытыми уже исполненным C17.

## Отчёт исполнителя

Передать выполненные Cxx, команды, timestamps/digests, environment, evidence paths, PASS/FAIL/BLOCKED/NOT_RUN/INCONCLUSIVE, фактические отклонения и влияние на scope. Отдельно: что не исполнялось, какие реальные источники могли различаться, где mock использован и какие выводы из него недопустимы. Недостаточность прав/prerequisites не скрывать.
