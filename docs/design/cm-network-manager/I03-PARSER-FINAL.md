# I03.T04: итог парсера подписок и собственных серверов

Дата: 2026-10-06. Пин ядра: mihomo `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. Решения: [I03-DECISIONS](I03-DECISIONS-2026-10-06.md). Ревью: [блок 1](REVIEW-2026-10-06-BLOCK1.md), [блок 2](REVIEW-2026-10-06-BLOCK2.md). Оракулы полей: [`grok-review/`](grok-review/).

Код завершён на уровне L1 (unit/fixture). Installation runtime: NOT_RUN. Сверка корпуса с ядром (`.w`): SKIPPED, пока не задан `CM_TEST_MIHOMO`.

## Форматы

| Формат | Вход | Код |
|---|---|---|
| mihomo YAML / JSON | `parse_native`; один ведущий BOM снимается; YAML через потоковый guard | `src/sources/parser/native/` |
| Список URI | `parse_uri_list(UriList)`; каждая строка — тот же валидатор узла, что native | `src/sources/parser/list.rs` |
| base64-список | std и url-safe алфавит, padding необязателен, переносы строк допустимы; не больше двух слоёв | `list.rs` |
| Автоопределение | `detect_body` + `parse_source`; используется `pipeline::negotiate_source` | `list.rs`, `pipeline.rs` |
| Ручной сервер | `manual::parse_add_server_args` + `read_secret`/`read_secret_file` + `manual_source_input` | `src/sources/manual.rs` |

Решения по телу (`negotiate_source`): HTML/заглушка, base64 от HTML, тело без пригодных узлов — повтор со следующим UA; пустое тело, превышение лимита, третий слой base64 (`UnsupportedEncoding`) и неподдержанная семантика (host controls, неизвестные поля) — terminal без повтора.

## Протоколы и транспорты

| Протокол | Транспорты | Безопасность | Отказ |
|---|---|---|---|
| VLESS | tcp, ws (+HTTPUpgrade), http, h2, grpc, xhttp | TLS, REALITY (только tcp/grpc/xhttp, нужен `tls: true` и рабочий `client-fingerprint`), `flow` только vision на tcp с TLS/REALITY | ShadowTLS/Restls/JLS, ECH, неизвестный `mode` XHTTP |
| VMess | tcp, ws, http, h2, grpc | TLS, REALITY | xhttp |
| Trojan | tcp, ws, grpc | всегда TLS; REALITY | h2, `ss-opts` |
| Shadowsocks | tcp | — | плагины, shadow-tls, restls; ключ 2022 неверной длины |
| Hysteria2 | quic | всегда TLS; `obfs: salamander` + пароль | другой obfs, `pinSHA256` в URI |
| TUIC | quic | всегда TLS; v5 | v4 `token` |
| WireGuard | — | — | Amnezia, `dialer-proxy`, `remote-dns-resolve`, `dns` |
| HTTP / SOCKS5 | tcp | TLS по `tls: true` | — |

gRPC и H2 требуют TLS или REALITY. Сертификаты клиента (`certificate`/`private-key`) — только PEM в узле; путь хоста отвергается.

## Проверка сертификата (D2): `tls_verification`

| Значение | Когда |
|---|---|
| `not_applicable` | у узла нет TLS-канала к прокси (Shadowsocks, WireGuard, VLESS/VMess/HTTP/SOCKS5 без `tls: true`) |
| `verified` | TLS с обычной проверкой цепочки |
| `pinned` | задан `fingerprint` или `name-cert-verify` (ядро проверяет pin), либо REALITY (проверка по ключу сервера) |
| `disabled` | `skip-cert-verify: true` без pin; входит в сводку пропусков и требует подтверждения |

URI: `insecure`/`allowInsecure`/`allow_insecure` → то же поле; неявное `true` не ставится никогда.

## Пропуски и подтверждение (D1, D3)

- Секции `proxy-groups`, `rules`, `sub-rules`, `rule-providers`, `http`/`file` proxy-providers не применяются и попадают в `ImportOmissions`; `inline` proxy-providers дают узлы. Типизированные группы и правила — `I04.T04.f`.
- Строки списка, которые нельзя импортировать, пропускаются с классом (`unsupported_scheme`, `unsupported_feature`, `malformed`) и номером строки; повтор имени — `malformed`. Текст строки нигде не сохраняется. Ноль узлов — отказ.
- Пропуски идут из парсера (`ParsedSource::omissions`), не от вызывающего кода; число узлов `disabled` выводится из самих узлов.
- Публикация с пропусками требует `accept_omissions(digest)`. Автообновление публикует без вопроса только в пределах подтверждённой границы из сохранённого provenance, без нуля узлов и без падения больше 50%.

## Ручной сервер

Секрет не проходит через argv: флаги `--uuid`, `--password`, `--private-key`, … и любой URI в аргументах — `SecretInArgv`. Секрет читается из stdin (`--secret-stdin`, `--uri-stdin`) или из файла (`--secret-file PATH`, без следования symlink). Файл секрета, доступный группе или остальным, или чужой, отвергается (`InsecureCredentialFile`). Правка — новое поколение того же Source; **Node ID меняется** по модели I02, устойчивая идентичность — Source ID (решение 2026-10-06).

CLI-подкоманда `cm source add-server` в `main.rs` не подключена: это `V.02` (CLI нового пути). Библиотека и разбор аргументов готовы.

## Открыто

- `.w` — прогнать с `CM_TEST_MIHOMO`, когда доступен бинарник ядра pinned версии.
- `H.05.b` — миграция YAML на saphyr (`cargo fetch` разрешён 2026-10-06).
