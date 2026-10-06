# Ревью этапа 2, волна 1 (Grok 4.7): пакеты A1, B, A2, D

Дата: 2026-10-06. Сдача Grok — коммит `2704726`. Бриф: [GROK-STAGE2-2026-10-06.md](GROK-STAGE2-2026-10-06.md).

## Итог

| Подзадача | Статус после ревью | Основание |
|---|---|---|
| `V.02` CLI только для чтения | **код принят; задача открыта** | Критерий требует dry-run настоящей подписки — это делает пользователь. Ревью A1 — ниже |
| `I04.T04.a` генератор config | закрыт | Добавлен тест: весь корпус (9 типов) → парсер → генератор → `mihomo -t` |
| `V.03` узлы Store → config | закрыт | Расхождения с legacy объяснены ниже. Одно из них — условие для `V.04` |
| `I04.T04.b` коды `mihomo -t` | закрыт после исправлений | Таймаут, `O_NOFOLLOW` |
| `H.11` harness с ядром | закрыт после исправлений | Закрыт прямой путь к echo, ожидание ограничено |
| `I04.T02.d` legacy host wrapper | закрыт | Сквозная обёртка. Lifecycle через trait честно отвечает `Unsupported` до `I04.T04.c` |
| `I06.T01.a` ADR Q08 | закрыт, принят вариант 3 | helper + systemd unit на worker |
| `I15-R.T02.a` coverage | закрыт | В каждой ячейке есть источник; «неизвестно» допустимо по брифу |
| `I17-D.T02.a` навигация | закрыт, рекомендация принята координатором | Пользователь может её пересмотреть до `I17-D.T03.a` |

## Замечания по процессу

1. Коммит `2704726` сделан исполнителем по поручению пользователя. Исправления ревью — отдельными коммитами.
2. В A1 исполнитель переформатировал целые файлы (`main.rs`, `tui.rs`, `i18n.rs` — около 3,7 тыс. строк). Координатор откатил это до сдачи. В бриф Grok добавлен явный запрет (правило 6).

## Найдено и исправлено

| # | Где | Дефект | Исправление |
|---|---|---|---|
| 1 | `core/mihomo/validate.rs` | `mihomo -t` запускался без таймаута. С правилами GEOIP/GEOSITE ядро идёт в сеть за базами и может зависнуть | Запуск через `capture_with_policy`: 20 с, предел вывода 64 КиБ, своя группа процессов, `LC_ALL=C` |
| 2 | `core/mihomo/validate.rs` | Конфиг писался в песочницу без `O_NOFOLLOW`: подложенный симлинк перенаправлял запись | `O_NOFOLLOW` и `O_CLOEXEC`; тест с симлинком (`validate_file_does_not_follow_a_planted_symlink`) |
| 3 | `core/mihomo/config.rs` | Узел с `interface-name`, `routing-mark` или `dialer-proxy` отправляет трафик мимо маршрутов CM. Парсер I03 таких ключей не пропускает, но `generate_config` принимает произвольные узлы | Отказ `Forbidden`; тест `nodes_that_bypass_cm_routing_are_forbidden` |
| 4 | `tests/netharness/core.sh` | echo доступен из namespace worker напрямую, поэтому успех не доказывал путь через прокси | `drop` прямого пути к echo в namespace worker и проверка, что он закрыт. Проверено отрицательным прогоном: worker с `MATCH,DIRECT` даёт `FAIL` |
| 5 | `tests/netharness/core.sh` | Цикл повторов (до ~225 с) переживал namespace-держатели (80 с): при отказе было зависание до внешнего таймаута вместо `FAIL` | Ожидание ~50 с, держатели 150 с, таймаут теста 140 с |
| 6 | `tests/audit_i04_mihomo_config.rs` | Критерий `I04.T04.a` («корпус → config → `mihomo -t`») не проверялся: `audit_i03_core_check` отдаёт ядру сырой корпус, минуя генератор | Тест `whole_corpus_passes_generator_and_pinned_core` |
| 7 | `sources/cli.rs` (A1) | `--url-file -x` сообщал «адрес в аргументах» | Ошибка использования |

## V.03: расхождения с legacy-генерацией (`src/vpn/config.rs::build_config_for`)

| Legacy | Worker | Почему |
|---|---|---|
| `tun`, `auto-route`, `auto-redirect`, `strict-route`, `dns-hijack`, `auto-detect-interface` | нет | Маршруты хоста принадлежат CM (A+C). Генератор отказывает, а не вырезает молча |
| `mixed-port`, `allow-lan`, `bind-address`, `external-controller-unix` | `listeners: [{type: mixed, listen: 127.0.0.1, port: <аренда>}]` | Один арендованный loopback-вход. API-сокет добавит `I04.T04.c` |
| `proxy-providers` (файл подписки) + группы с `use`/`include-all`/`exclude-filter` | `proxies` встроены, одна группа по именам | Узлы берутся из Store (неизменяемые, привязаны к поколению), а не из файла провайдера |
| `default-nameserver` с `system` | без `system` | У worker нет права на резолвер хоста |
| `direct-nameserver`, `profile.store-selected`, `store-fake-ip`, `tcp-concurrent`, `unified-delay`, `find-process-mode` | нет | Настройки поведения legacy-UI. Для worker их нужность решается в `I04.T04.c`–`.e` |
| **`geodata-mode`, `geox-url`, `mmdb`/`geoip`/`geosite`, `geo-auto-update`** | **нет** | **Условие для `V.04`.** Если в `rules.txt` есть `GEOIP`/`GEOSITE`, ядро worker само скачает базы по умолчанию — мимо контроля CM. До `V.04` нужно либо передавать worker пути к уже скачанным legacy-базам (offline), либо отклонять такие правила явной ошибкой |

## Ревью A1 (V.02)

- URL не принимается в аргументах. Чтение: stdin, файл с проверкой как у секрета, `--from-legacy` только от root.
- Store открывается только на чтение под общей блокировкой. `--dry-run` обязателен, без него — явный отказ.
- http разрешён только для loopback, редиректы запрещены транспортом.
- Тест сверяет байты и mtime Store, legacy и state до и после, а также проверяет `/proc/PID/cmdline`.
- Ручной прогон всех веток ошибок на собранном бинарнике.
- Открыто: dry-run настоящей подписки и `--from-legacy` от root — делает пользователь.

## Наблюдения для следующих задач

- `render_core_unit` (`src/core/unit.rs`) не задаёт `User=` (подтверждено, из ADR-CONTROLLER). Закрывает I06.
- Экран VPN: `s` мгновенно останавливает службу без вопроса (`src/tui.rs`, `KeyCode::Char('s')` в `key_vpn`). Для туннелей приложений принят вопрос с подтверждением (I17D-NAV).
- Legacy-юнит по-прежнему запускает ядро из `{home}/bin/mihomo` (каталог с правом записи). `CORE_BIN` к нему не применён.

## Gate

`CM_TEST_MIHOMO=… sh tests/check_audit.sh` после исправлений: EXIT=0, 492 passed, 0 failed, без SKIPPED (ядро и user namespaces доступны).
