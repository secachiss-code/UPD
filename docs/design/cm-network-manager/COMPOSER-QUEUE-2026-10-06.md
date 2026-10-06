# Очередь Composer, 2026-10-06

**После коммита `6e5ea7b` следующий блок (H.12, I03.T05, I04.T01.a, V.01–V.02, H.05.b) передан Grok: [GROK-QUEUE](GROK-QUEUE-2026-10-06.md) §Блок 3. Composer его не берёт.**

Исполнитель: **Composer 2.5**. Объём только этот файл. Независимое ревью, оракул полей pinned-ядра и отрицательные fixtures готовит Grok 4.7 (high): [GROK-QUEUE-2026-10-06](GROK-QUEUE-2026-10-06.md). Источник требований: [REMAINING-DAG-PLAN-2026-10-06](REMAINING-DAG-PLAN-2026-10-06.md). Контракт парсера: [I03-PARSER-IMPLEMENTATION-CONTRACT-2026-10-04](I03-PARSER-IMPLEMENTATION-CONTRACT-2026-10-04.md). Точка остановки: [WORK-STOP-2026-10-05](WORK-STOP-2026-10-05.md).

Поручение пользователя 2026-10-06 снимает запрет «не продолжать» из CODER-QUEUE **только для ID ниже**. Реализация `H.04` разрешена только после ADR Grok и решения пользователя D4. `H.03`, `H.05`–`H.11`, `I03.T05`, V, I04 и остальные рубежи не начинать.

## Правила

- Одна подзадача за сессию, в порядке §Порядок. Следующую не начинать, пока текущая не закрыта своим «готово, когда».
- Неподдержанное поле, транспорт или секция → `UnsupportedFeature` с фиксированным кодом. Молчаливое удаление запрещено.
- Ошибки, `Debug`, status DTO и diagnostic export не содержат URL, UA, UUID, password, ключи и исходное тело.
- Принятые поля сохраняются в `full_definition` без потерь; digest стабилен.
- Сверка полей — с pinned mihomo `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. Только первичный источник. Если ключа в pinned ядре нет — `UnsupportedFeature` и ссылка на файл/строку источника.
- Установка пакета, изменение сети хоста, реальные запросы к провайдеру, запуск приложения и служб запрещены. Локальный `127.0.0.1` в unit-тесте разрешён. Runtime-статус остаётся `NOT_RUN`.
- Проверка подзадачи: `cargo test --offline --locked` затронутых тестов, затем полный прогон, если подзадача меняет общий код. Цифру «372 теста» из плана не копировать: записать фактический прогон.
- Не коммитить, пока пользователь не попросит.
- D1, D2, D3 решены пользователем 2026-10-06: [I03-DECISIONS-2026-10-06](I03-DECISIONS-2026-10-06.md). Реализовать строго по нему; несогласие — записать, не отступать. На D4 и других нерешённых вопросах остановиться и записать вопрос.
- **Fixtures от Grok.** Для `.b`, `.m`, `.o`, `.r`, `.t`, `.u` Grok заранее кладёт отрицательные fixtures в `grok-review/fixtures/<ID>/`. Подзадачу начинать только когда в `grok-review/STATUS.md` для неё стоит `FIXTURES_READY`. Первым шагом перенести их в `tests/` без изменений; они должны падать или не компилироваться до реализации и проходить после. Ослаблять или удалять fixture Grok нельзя: несогласие записать в отчёт подзадачи. Если fixtures не готовы, взять следующую готовую подзадачу из §Порядок.
- **Ревью.** После закрытия подзадачи записать в отчёт revision и команды. Следующую можно начинать, не дожидаясь ревью. `REOPEN` с `P0` от Grok останавливает подзадачи, зависящие от исправляемой, до исправления.

## Кто что делает

| Блок | Исполнитель | Ревью | Когда |
|---|---|---|---|
| `H.01` | Composer | Grok, кратко | сразу |
| `H.02` вместе с 21 предупреждением clippy в новых модулях | Composer | Grok, кратко | сразу |
| ADR `H.04` (HTTP), ADR `H.05` (YAML) | Grok | координатор; решения D4, D5 — пользователь | сразу |
| Реализация `H.04` | Composer | Grok, полное | после ADR и D4 |
| Материалы к D1–D3, оракул полей mihomo | Grok | — | сразу |
| `.a` | Composer | Grok, кратко | сразу |
| Отрицательные fixtures `.b` `.m` `.o` `.r` `.t` `.u` | Grok | Composer проверяет перенос | до своей подзадачи |
| `.b`, `.m`, `.n`, `.o`, `.r`, `.t`, `.u` | Composer | Grok, полное | по §Порядок |
| `.c` (без `skip-cert-verify`), `.d`, `.e`, `.f`, `.g`, `.h`, `.i`, `.j`, `.k`, `.l` | Composer | Grok по оракулу | по §Порядок |
| `.y` схема пропусков и TLS-отметки | Composer | Grok, полное | после `.b` (D1–D3 решены) |
| `skip-cert-verify` | Composer | Grok | после `.y` |
| `.q`, `.s` | Composer | Grok | после `.y`; `.q` ещё после `.i`, `.s` после `.r` |
| `.v`, `.w`, `.x` | Composer | Grok, итог T04 | в конце; `.x` после D5 |

## Порядок

Сразу, последовательно:

1. `H.01`
2. `H.02`
3. `I03.T04.a`
4. `I03.T04.b` (после `FIXTURES_READY`)
5. `I03.T04.y` схема пропусков и TLS-отметки (по [I03-DECISIONS](I03-DECISIONS-2026-10-06.md))
6. `I03.T04.c` (с `skip-cert-verify` по D2, если `.y` готов; иначе поле отвергается до `.y`)
7. `I03.T04.f` → `I03.T04.g` → `I03.T04.h` → `I03.T04.m` (после `FIXTURES_READY`) → `I03.T04.n`
8. `I03.T04.d`, `I03.T04.e`, `I03.T04.k`, `I03.T04.l` → `I03.T04.i` → `I03.T04.j` → `I03.T04.o` (после `FIXTURES_READY`) → `I03.T04.r` (после `FIXTURES_READY`)
9. `H.04` — когда есть ADR Grok и решение D4; можно вставить между любыми подзадачами I03

Стоп, пока нет решения пользователя:

| Стоп | Ждёт | Что не делать до решения |
|---|---|---|
| `I03.T04.x` | D5 (ADR YAML от Grok) | Не закрывать T04, пока YAML-стек не выбран. |

D1, D2, D3 решены ([I03-DECISIONS-2026-10-06](I03-DECISIONS-2026-10-06.md)), стопов по ним нет. `I03.T04.p` закрыт этим документом. Порядок после `.b`: `.y` → `skip-cert-verify` отдельной правкой в `.c`, `.j`, `.k`, `.l` (по мере их готовности). После `.y` и `.i`: `.q`. После `.y`, `.r` и `.q`: `.s` → `.t` (после `FIXTURES_READY`). Затем `.u` (после `FIXTURES_READY`) → `.v`, `.w` → `.x` (после D5).

## H.01 Обновить статусы evidence после локального прогона

`S` · L1 · зависит от: —

- **Что:** Перепрогнать `cargo test --offline --locked`. Записать revision, rustc, хеш `Cargo.lock`, число тестов и отказов. В статусах разделить «compile/unit PASS» и «installation runtime NOT_RUN».
- **Выход:** `docs/design/cm-network-manager/i03-evidence/local-tests-2026-10-06/summary.json`; правки `WORK-STOP-2026-10-05.md`, `CODER-QUEUE-2026-10-04.md`, `DAG-PLAN.json` только в части фактически выполненных unit/fixture.
- **Готово, когда:** Ни один документ не пишет `NOT_RUN` для unit/fixture, которые этот прогон выполнил. Runtime и установка не повышены до PASS.

- **Дополнение 2026-10-06 (координатор):** падение `audit_contracts::audit_closed_attach_releases_helper_thread_and_fd` — не гонка. Тест сравнивает число fd helper с абсолютным 4, а helper наследовал 5 fd без `CLOEXEC`, которые процесс тестов получил от окружения (терминал IDE/агента). Воспроизводится детерминированно: `exec 5</dev/null … 9</dev/null` перед запуском даёт ровно `threads: 1, fds: 9`. Исправлено в `tests/audit_contracts.rs` (`spawn_helper`: `close_range(3, ∞)` в `pre_exec`). Перепрогнать, обновить `summary.json`: статус теста PASS, причина — «inherited fds from environment», не «intermittent».

## H.02 Расширить локальный gate

`S` · L1 · зависит от: —

- **Что:** В `tests/check_audit.sh` добавить `cargo clippy --all-targets -- -D warnings` и `rustfmt --check` для `src/sources`, `src/profiles`, `src/migration`. Шаг `cosmic` оставить отдельным. Старые модули не включать в `-D warnings`.
- **Выход:** `tests/check_audit.sh`
- **Готово, когда:** Новое предупреждение clippy или отклонение rustfmt в этих трёх деревьях роняет gate. Предупреждения старых модулей gate не роняют.
- **Дополнение 2026-10-06 (rustfmt):** шаг `rustfmt --check` для `tests/audit_contracts.rs` в `check_audit.sh` уже падает на `HEAD`: 4 расхождения стиля с rustfmt 1.9.0. Отформатировать файл отдельным diff без изменения логики, иначе gate красный ещё до правок.
- **Дополнение 2026-10-06:** в этих деревьях уже 21 предупреждение (`profiles/model.rs` 6, `sources/negotiation.rs` 4, `profiles/store.rs` 3, `migration/transaction.rs` 2, `sources/parser/yaml_guard.rs` 2, по одному в `sources/artifact.rs`, `capabilities.rs`, `parser/native.rs`, `pipeline.rs`). Убрать их в рамках `H.02` без изменения поведения, отдельным diff по файлу. `#[allow]` допустим только с комментарием-обоснованием. Полный прогон тестов после правки.

## H.04 HTTP-транспорт с общим deadline

`L` · L1 · зависит от: ADR Grok `grok-review/ADR-HTTP-draft.md` и решения пользователя D4 · ревью Grok полное

- **Что:** Реализовать выбранный в D4 вариант как `FetchTransport` для `sources::negotiation`: общий deadline на DNS, connect, TLS и тело; лимит тела; не больше заданного числа redirect, запрет https→http, лимиты проверяются заново после redirect; локальный proxy FlClash по прежним правилам `src/vpn.rs`. Подключать к legacy-пути `src/vpn.rs` не надо.
- **Выход:** `src/sources/transport.rs`; тесты на локальном сервере `127.0.0.1`
- **Готово, когда:** Медленный DNS (подменный resolver), slow-loris, бесконечный redirect, большое тело завершаются в пределах budget типизированной ошибкой. Секретный URL не встречается в ошибках, `Debug` и логах.

## I03.T04.a Разбить parser/native.rs на модули

`S` · L1 · зависит от: —

- **Что:** Разнести `src/sources/parser/native.rs` на `native/mod.rs` (вход, лимиты), `native/common.rs` (строгий visitor, строки/числа/enum-наборы), `native/tls.rs`, `native/transport.rs`, `native/proto/{vless,vmess,ss,trojan,hy2,tuic,wg,http,socks}.rs`. Поведение не менять.
- **Выход:** `src/sources/parser/native/*`
- **Готово, когда:** Проходят 9 native + 4 guard + 3 pipeline теста: `tests/audit_i03_native.rs`, unit-тесты в `src/sources/parser/yaml_guard.rs` (после переноса — по новому пути), `tests/audit_i03_pipeline.rs`.

## I03.T04.b Общий валидатор вложенных опций

`M` · L1 · зависит от: `I03.T04.a`

- **Что:** Типизированный builder вложенных map: строгий набор ключей, отказ на неизвестный ключ без эха имени, ограниченные строки (длина, управляющие символы), списки с лимитом, enum-наборы, взаимоисключающие и парные поля. Результат в `full_definition` без потерь, digest стабилен (каноническая сортировка).
- **Выход:** `native/common.rs`
- **Готово, когда:** Fixtures: неизвестный вложенный ключ, дубль, превышение длины, неверный тип → фиксированные коды ошибок.

## I03.T04.c TLS-опции

`L` · L1 · зависит от: `I03.T04.b`; `skip-cert-verify` — ещё от `I03.T04.y`

- **Что:** По pinned v1.19.32: `tls`, `servername`/`sni`, `alpn`, `client-fingerprint` (закрытый набор), `fingerprint` (hex sha256), `skip-cert-verify` по D2: `true` без pin → `tls_verification: Disabled`, `true` с `fingerprint`/`name-cert-verify` → `Pinned`, `false`/нет → `Verified`; до готовности `.y` поле отвергается фиксированным кодом, `certificate`/`private-key` (пути хоста — отказ), `ech-opts` → `UnsupportedFeature`.
- **Выход:** `native/tls.rs`; fixtures `tls_*`
- **Готово, когда:** У каждого поля есть позитивный и негативный fixture и ссылка на строку первичного источника.

## I03.T04.d REALITY

`M` · L1 · зависит от: `I03.T04.c`

- **Что:** `reality-opts`: `public-key` (base64url, 32 байта), `short-id` (hex, ≤16 символов, чётная длина), прочие ключи pinned версии. Сочетания сверить с источником: VLESS + TCP/gRPC/XHTTP. Обязателен TLS-блок с `servername`.
- **Выход:** `native/tls.rs`
- **Готово, когда:** Неверная длина ключа, REALITY без servername, REALITY с WS → отказ с кодом.

## I03.T04.e Расширения VLESS/VMess

`M` · L1 · зависит от: `I03.T04.c`

- **Что:** VLESS: `flow` (`xtls-rprx-vision` только с TLS/REALITY на TCP), `packet-encoding` (`packetaddr`/`xudp`). VMess: TLS, `packet-addr`, `global-padding`, `authenticated-length`. `alterId` > 0 — по первичному источнику, без догадки.
- **Выход:** `native/proto/vless.rs`, `vmess.rs`
- **Готово, когда:** Матрица flow × transport покрыта fixtures.

## I03.T04.f Транспорт WS и HTTPUpgrade

`M` · L1 · зависит от: `I03.T04.b`

- **Что:** `network: ws`, `ws-opts`: `path`, `headers` (HTTP token, без дублей регистра), `max-early-data`, `early-data-header-name`, `v2ray-http-upgrade`, `v2ray-http-upgrade-fast-open`.
- **Выход:** `native/transport.rs`
- **Готово, когда:** Path без ведущего `/`, нечисловой early-data, конфликтующие флаги → отказ.

## I03.T04.g Транспорты HTTP и H2

`M` · L1 · зависит от: `I03.T04.b`

- **Что:** `http-opts`: `method`, `path` (список), `headers` (map → список строк). `h2-opts`: `host` (список), `path`. Требование TLS для H2 проверяет `I03.T04.i`, не эта подзадача.
- **Выход:** `native/transport.rs`
- **Готово, когда:** Fixtures: пустые списки, лишние ключи, header-инъекции CR/LF.

## I03.T04.h Транспорты gRPC и XHTTP

`M` · L1 · зависит от: `I03.T04.b`

- **Что:** `grpc-opts.grpc-service-name`. XHTTP: сверить наличие и ключи (`path`, `host`, `mode`, extra) в pinned v1.19.32. Если в ядре нет — `UnsupportedFeature` и правка [I03-CAPABILITY-MATRIX](I03-CAPABILITY-MATRIX-2026-10-04.md).
- **Выход:** `native/transport.rs`; capability matrix
- **Готово, когда:** Решение по XHTTP подтверждено ссылкой на pinned source.

## I03.T04.i Перекрёстная матрица protocol × transport × security

`M` · L1 · зависит от: `.d`, `.e`, `.f`, `.g`, `.h`

- **Что:** Таблица допустимых сочетаний из `capabilities::supported_transports` плюс TLS для grpc/h2, REALITY только для перечисленных транспортов, flow только TCP. Валидация после разбора узла.
- **Выход:** `native/matrix.rs`; таблица в capability doc генерируется из кода
- **Готово, когда:** Ручной копии таблицы нет. На каждый запрещённый класс есть fixture.

## I03.T04.j Trojan

`M` · L1 · зависит от: `I03.T04.i`

- **Что:** `password`, `sni`, `alpn`, `skip-cert-verify`, `client-fingerprint`, `network` tcp/ws/grpc, `reality-opts` если поддержан, `ss-opts` → `UnsupportedFeature`, `udp`.
- **Выход:** `native/proto/trojan.rs`
- **Готово, когда:** Позитивные tcp/ws/grpc; негатив без password, с h2, с ss-opts.

## I03.T04.k Hysteria2

`M` · L1 · зависит от: `I03.T04.c`

- **Что:** `password`, `ports` (диапазоны, лимит количества), `hop-interval`, `up`/`down`, `obfs: salamander` + `obfs-password`, `sni`, `alpn`, `skip-cert-verify`, `fingerprint`, `udp`.
- **Выход:** `native/proto/hy2.rs`
- **Готово, когда:** Диапазон вне 1–65535, перевёрнутый диапазон, obfs без пароля → отказ.

## I03.T04.l TUIC

`M` · L1 · зависит от: `I03.T04.c`

- **Что:** v5: `uuid` + `password`. v4 `token` — `UnsupportedFeature`, если отдельного решения нет. `congestion-controller` (cubic/new_reno/bbr), `udp-relay-mode` (native/quic), `reduce-rtt`, `alpn`, `sni`, `disable-sni`, `request-timeout`.
- **Выход:** `native/proto/tuic.rs`
- **Готово, когда:** Fixtures v4/v5 и неизвестного congestion-controller.

## I03.T04.m WireGuard

`L` · L1 · зависит от: `I03.T04.b`

- **Что:** `private-key`/`public-key`/`pre-shared-key` (base64, 32 байта), `ip`/`ipv6` (CIDR), `allowed-ips`, `mtu`, `reserved`, `peers` (список с лимитом), `persistent-keepalive`. `amnezia-wg-option`, `dialer-proxy`, `remote-dns-resolve`, `dns` → Restricted/Unsupported. Private key только в закрытом blob.
- **Выход:** `native/proto/wg.rs`
- **Готово, когда:** Неверная длина ключа, пересекающиеся peers, host-control поле → отказ. Ключ не встречается в status DTO.

## I03.T04.n Расширения Shadowsocks

`M` · L1 · зависит от: `I03.T04.b`

- **Что:** Шифры 2022 (`2022-blake3-*`, длина ключа base64 под шифр, мультипользовательский `:`), `udp-over-tcp` (+version). `obfs` и `v2ray-plugin` — поддержать или явно `UnsupportedFeature`. `shadow-tls` и `restls` — Unsupported по матрице.
- **Выход:** `native/proto/ss.rs`
- **Готово, когда:** Ключ 2022 неверной длины → отказ. Неизвестный plugin → `UnsupportedFeature`.

## I03.T04.o Классификация секретов узлов

`M` · L1 · зависит от: `.j`, `.k`, `.l`, `.m`, `.n`

- **Что:** Перечень секретных полей каждого протокола (uuid, password, private-key, psk, obfs-password, auth). Секреты только в закрытом blob.
- **Выход:** `native/secrets.rs`; тест-сканер по всем fixtures
- **Готово, когда:** Сканер находит 0 секретов вне закрытого blob на всём корпусе fixtures.

## I03.T04.p Решение: группы, правила, провайдеры

`S` · D · **закрыт 2026-10-06**: решение пользователя записано в [I03-DECISIONS-2026-10-06](I03-DECISIONS-2026-10-06.md) (D1 — вариант 2 сейчас, вариант 3 в `I04.T04.f`).

## I03.T04.y Схема пропусков, TLS-отметка, подтверждение

`M` · L1 · зависит от: `I03.T04.b` · ревью Grok полное

- **Что:** По разделу «Общий механизм» [I03-DECISIONS](I03-DECISIONS-2026-10-06.md): поле `ImportOmissions` в provenance (имена секций из закрытого набора; счётчики строк по классам `UnsupportedScheme`/`UnsupportedFeature`/`Malformed` с номерами; число узлов `Disabled`), поле узла `tls_verification` (`Verified`/`Pinned`/`Disabled`, входит в digest), новая версия схемы Store с миграцией старых записей (пустые пропуски, `Verified` для TLS-узлов), `PendingConfirmation` + `accept_omissions(<digest сводки>)` в pipeline, политика автообновления (не шире подтверждённого, > 0 узлов, падение не больше 50%).
- **Выход:** `src/profiles/model.rs`, `src/profiles/store.rs` (миграция схемы), `src/sources/pipeline.rs`; тесты
- **Готово, когда:** Старый Store открывается после миграции; импорт с пропусками без подтверждения не публикуется; подтверждение с чужим digest отвергается; автообновление с новым классом пропуска, нулём узлов или падением > 50% оставляет прежнее поколение; ни URI, ни текст строки, ни значения полей не попадают в сводку, ошибки и `Debug`.

## I03.T04.q Реализовать политику групп/правил/провайдеров

`L` · L1 · зависит от: `I03.T04.y`, `I03.T04.i`

- **Что:** Строго по таблице D1 в [I03-DECISIONS](I03-DECISIONS-2026-10-06.md): узлы из `proxies` и из `inline` proxy-providers импортируются; `proxy-groups`, `rules`, `sub-rules`, `rule-providers`, `http` и `file` proxy-providers не применяются и попадают в `ImportOmissions` (без сети, без чтения путей хоста); неизвестный `type` провайдера → `UnsupportedFeature` документа; 0 узлов → отказ источника. Содержимое секций не пишется в диагностику.
- **Выход:** `native/sections.rs`
- **Готово, когда:** На каждую строку таблицы D1 есть fixture; типовая подписка (proxies + groups + rules) даёт `PendingConfirmation` с тремя именами секций; подписка только из `http`-провайдеров отвергается.

## I03.T04.r URI-парсер

`L` · L1 · зависит от: `.i`, `.j`, `.k`, `.l`, `.n`

- **Что:** `vless://`, `vmess://` (base64-JSON v2rayN), `ss://` (SIP002 и legacy base64), `trojan://`, `hysteria2://`/`hy2://`, `tuic://`, `socks5://`, `http(s)://`. Percent-decoding, fragment → имя, query → тот же типизированный node, что и native (один digest). Неизвестный query-параметр → отказ. По D2: `insecure=1` (Hysteria2), `allowInsecure=1` (Trojan) и аналоги → то же поле `skip-cert-verify`; неявное `true` конвертера ядра для socks не воспроизводить. Ошибка строки в списке — пропуск по D3, не отказ списка.
- **Выход:** `src/sources/parser/uri.rs`
- **Готово, когда:** Для каждого протокола URI и эквивалентный native дают одинаковый definition digest.

## I03.T04.s base64-подписка и смешанные списки

`M` · L1 · зависит от: `I03.T04.r`, `I03.T04.y`

- **Что:** Std/urlsafe, с padding и без, перевод строк. Лимит после декодирования. Строки — по таблице D3 в [I03-DECISIONS](I03-DECISIONS-2026-10-06.md): неподдержанная схема/функция и испорченная строка — пропуск с классом и номером, пустые строки игнорируются, порча base64-слоя или 0 узлов — отказ источника.
- **Выход:** `src/sources/parser/base64.rs`
- **Готово, когда:** Fixtures: мусор, двойной base64, пустые строки, неизвестная схема.

## I03.T04.t Определение формата и negotiation для всех форматов

`M` · L1 · зависит от: `I03.T04.s`, `I03.T04.q`

- **Что:** Классификатор тела: native YAML/JSON, URI-список, base64, HTML/заглушка, пустой. Обобщить `negotiate_native_source` → `negotiate_source`: HTML и непригодное тело — bounded retry следующим UA; unsupported semantics — terminal отказ.
- **Выход:** `src/sources/parser/detect.rs`; `src/sources/pipeline.rs`
- **Готово, когда:** Таблица «тело → решение» покрыта fixtures, включая HTML-заглушку провайдера.

## I03.T04.u Ручной собственный сервер

`M` · L1 · зависит от: `I03.T04.r`, `I03.T04.o`

- **Что:** Вход: URI или поля CLI `cm source add-server --protocol … --server … --port …`. Секреты из stdin или файла, не из argv. `SourceKind::Manual`, origin local, без fetch settings. Изменение = новое поколение, Node identity сохраняется.
- **Выход:** `src/sources/manual.rs`; CLI-подкоманда за флагом
- **Готово, когда:** Секрет не попадает в argv. Правка сервера сохраняет Node ID.

## I03.T04.v Независимые источники для профилей

`M` · L1 · зависит от: `I03.T04.u`, `I03.T04.t`

- **Что:** `ConnectionProfile` ссылается на `(source, node)` любого источника. Несколько источников сосуществуют. Удаление используемого источника — staged removal из I02. Refresh одного источника не трогает другие.
- **Выход:** тесты `profiles/store`
- **Готово, когда:** Fixtures: два источника, профиль на каждом, удаление одного с активным профилем.

## I03.T04.w Сверка с ядром на корпусе fixtures

`M` · L1 · зависит от: `I03.T04.t`, `I03.T04.o`

- **Что:** Для каждого принятого fixture собрать минимальный config и прогнать `mihomo -t -d <tmp>` pinned версии, если бинарник уже есть локально. Сеть не использовать. Расхождение «парсер принял, ядро отвергло» — баг парсера. Бинарник не скачивать.
- **Выход:** `tests/audit_i03_core_check.rs` (`skip` без бинарника, статус явный)
- **Готово, когда:** 0 расхождений на корпусе. Skip в evidence — `SKIPPED`, не `PASS`.

## I03.T04.x Закрыть T04 в документации

`S` · L0 · зависит от: `I03.T04.v`, `I03.T04.w`, `H.05` (решение D5 по ADR Grok)

- **Что:** Обновить capability matrix из кода, parser contract и [I03-NATIVE-PARSER-FIRST](I03-NATIVE-PARSER-FIRST-2026-10-05.md).
- **Выход:** `I03-PARSER-FINAL.md`
- **Готово, когда:** Пункты остатка T04 из WORK-STOP закрыты или явно `Unsupported` со ссылкой. `I03.T05` не начинать.
