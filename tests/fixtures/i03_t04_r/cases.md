# Fixtures `I03.T04.r`

Дата: 2026-10-06. Входов: 39. Composer переносит каталог без изменений. Тест с `include_str!` пишется до `src/sources/parser/uri.rs`.

Пара «URI ↔ native» принимается, и `definition_digest` узла из URI равен digest узла из соседнего JSON. Сравнение не по сырому тексту файла. Отказ не содержит исходную строку URI и маркеры `CMFIXR-*`.

Имя берётся из fragment после percent-decode. У `vmess://` имя берётся из JSON-поля `ps`; fragment после base64 игнорируется и в digest не входит. Пустой fragment — отказ: имя обязано быть непустым.

Схема сравнивается без учёта регистра (`protocol_for_uri_scheme` уже так делает). `VLESS://` и `vless://` с тем же хвостом дают один digest.

Потолок одной строки URI — 8192 байт, то же число, что `MAX_URL_BYTES` в `src/sources/negotiation.rs`. `reject/uri-over-limit.uri.txt` — строка длиной 8193 без завершающего перевода строки. Отказ `UriTooLong`. Свернуть в существующий лимит можно, принять строку нельзя.

Query TUIC: `congestion_control` → native `congestion-controller`, `udp_relay_mode` → native `udp-relay-mode`. `https://` — протокол `http` и `tls: true`. `hy2://` и `hysteria2://` — один digest.

Ошибка одной строки в списке подписки по D3 — пропуск строки, не отказ всего списка. Эти файлы — по одной строке: для одиночного ручного разбора отказ строки виден напрямую. В списке та же строка даёт пропуск класса из таблицы, без текста строки в сводке.

## Пары, digest совпадает

| URI | Native | Что проверено |
|---|---|---|
| `pairs/vless.uri.txt` | `pairs/vless.json` | fragment `edge%20vless` → имя `edge vless` |
| `pairs/vless-upper.uri.txt` | `pairs/vless.json` | схема `VLESS` |
| `pairs/vmess.uri.txt` | `pairs/vmess.json` | base64-JSON v2rayN, имя из `ps` (`edge vmess.` с точкой: так у base64 есть padding). Fragment `#ignored` в имя не входит |
| `pairs/vmess-nopad.uri.txt` | `pairs/vmess.json` | тот же JSON без `=` в конце base64 |
| `pairs/ss-sip002.uri.txt` | `pairs/ss.json` | SIP002, userinfo = base64(`method:password`) |
| `pairs/ss-legacy.uri.txt` | `pairs/ss.json` | legacy base64(`method:password@host:port`), тот же digest, что SIP002 |
| `pairs/ss-percent.uri.txt` | `pairs/ss-percent.json` | userinfo `p%40ss%2Fw%3D` → пароль `p@ss/w=`, fragment → `edge ss` |
| `pairs/trojan.uri.txt` | `pairs/trojan.json` | пароль в userinfo |
| `pairs/hysteria2.uri.txt` | `pairs/hysteria2.json` | схема `hysteria2` |
| `pairs/hy2.uri.txt` | `pairs/hysteria2.json` | схема `hy2`, тот же digest |
| `pairs/tuic.uri.txt` | `pairs/tuic.json` | `uuid:password` и имена query |
| `pairs/socks5.uri.txt` | `pairs/socks5.json` | userinfo user/password |
| `pairs/http.uri.txt` | `pairs/http.json` | `tls: false` |
| `pairs/https.uri.txt` | `pairs/https.json` | тот же userinfo, `tls: true`, digest другой, чем у `http` |
| `pairs/vless-path.uri.txt` | `pairs/vless-path.json` | `path=%2Fapi%2Fv1` → `ws-opts.path` = `/api/v1` |
| `pairs/vless-ipv6.uri.txt` | `pairs/vless-ipv6.json` | `[2001:db8::10]` → server `2001:db8::10` |

## Отказы

| Файл | Класс | Риск |
|---|---|---|
| `reject/ipv6-bare.uri.txt` | `Malformed` | `2001:db8::10:443` без скобок разобран как хост или как порт 443 |
| `reject/port-0.uri.txt` | `Malformed` | порт 0 принят |
| `reject/port-65536.uri.txt` | `Malformed` | 65536 усечён до 16 бит |
| `reject/port-nan.uri.txt` | `Malformed` | `abc` принят как порт |
| `reject/query-repeat.uri.txt` | `DuplicateKey` | второй `sni` затёр первый |
| `reject/query-unknown.uri.txt` | `UnsupportedField` | `not-a-real-opt` отброшен. В отказе нет этого имени и `CMFIXR-unknown` |
| `reject/empty-fragment.uri.txt` | `Malformed` | пустое имя принято |
| `reject/vmess-extra.uri.txt` | `UnsupportedField` | лишнее поле JSON `extra` отброшено. В отказе нет `CMFIXR-vmess-extra` |
| `reject/vmess-non-utf8.uri.txt` | `InvalidUtf8` | payload `ff fe` плюс ASCII принят как JSON. В отказе нет `CMFIXR-vmess-non-utf8` и самого base64 |
| `reject/ss-plugin.uri.txt` | `UnsupportedFeature` | `plugin=obfs-local` отброшен, узел принят как обычный ss. В отказе нет userinfo и `obfs-local` |
| `reject/uri-over-limit.uri.txt` | `UriTooLong` | строка 8193 байт принята. Имя из `a` здесь длиннее 128, но отказ именно на длине строки, до разбора полей |
