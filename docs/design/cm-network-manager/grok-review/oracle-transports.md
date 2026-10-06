# Оракул полей: транспорты pinned mihomo

Пин: `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. Дата: 2026-10-06. Это G1.1, ключи `I03.T04.f` / `.g` / `.h`.

Декодер прокси: `structure.NewDecoder` с тегом `proxy`, `WeaklyTypedInput: true` и сравнением ключей без регистра ([`adapter/parser.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/parser.go#L12)). Неиспользованные ключи входа после разбора в [`common/structure/structure.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/common/structure/structure.go#L564) никуда не пишутся и ошибкой не становятся, если у структуры нет поля с тегом `remain`. Ядро неизвестный вложенный ключ молча оставляет. Для импорта cm это не образец: неизвестный ключ у нас отказ, не копия этого поведения.

Нулевые значения Go — то, что получается при отсутствии ключа (`omitempty`). Отдельной проверки «path начинается с `/`» в разобранных `StreamConnContext` для WS нет.

Структуры `WSOptions`, `HTTPOptions`, `HTTP2Options`, `GrpcOptions` объявлены в пакете `outbound` в [`vmess.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L145). Ими пользуются VLESS, VMess и Trojan.

## Кто какой транспорт принимает

| `network` | VLESS [`vless.go` 157](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L157) | VMess [`vmess.go` 176](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L176) | Trojan [`trojan.go` 80](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/trojan.go#L80) |
|---|---|---|---|
| пусто / иное | TCP | TCP, плюс отдельные `mekya`, `mkcp`/`kcp` | TCP |
| `ws` | да | да | да |
| `http` | да | да | нет в switch |
| `h2` | да, через `streamTLSConn(..., true)` | да | нет в switch |
| `grpc` | да | да | да |
| `xhttp` | да | нет | нет |

## `.f` WS и HTTPUpgrade

`ws-opts`, [`vmess.go` 165](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L165):

| Ключ | Тип Go | Если ключа нет | Куда копируется |
|---|---|---|---|
| `path` | `string` | `""` | `WebsocketConfig.Path` как есть |
| `headers` | `map[string]string` | nil | `http.Header` через `Add` |
| `max-early-data` | `int` | `0` | `MaxEarlyData` |
| `early-data-header-name` | `string` | `""` | как есть |
| `v2ray-http-upgrade` | `bool` | `false` | как есть |
| `v2ray-http-upgrade-fast-open` | `bool` | `false` | как есть |

Взаимоисключение флагов upgrade в этом switch не проверяется: оба bool просто копируются. Проверки «нечисловой early-data» на этом слое нет: поле уже `int`, а `WeaklyTypedInput` принимает число из строки. Неверный тип, который в int не кладётся, даёт ошибку декодера.

У VLESS рядом есть отдельный ключ узла `ws-headers` (`map[string]string`, [`vless.go` 83](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L83)). В `.f` его нет.

## `.g` HTTP и H2

`http-opts`, [`vmess.go` 145](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L145):

| Ключ | Тип Go | Если ключа нет |
|---|---|---|
| `method` | `string` | `""` |
| `path` | `[]string` | nil |
| `headers` | `map[string][]string` | nil |

`h2-opts`, [`vmess.go` 151](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L151):

| Ключ | Тип Go | Если ключа нет |
|---|---|---|
| `host` | `[]string` | nil |
| `path` | `string` | `""` |

VLESS для `h2` сначала вызывает `streamTLSConn` с требованием TLS. Сама проверка «H2 только с TLS» живёт там, не в структуре `h2-opts`. Очередь относит её к `.i`.

## `.h` gRPC

`grpc-opts`, [`vmess.go` 156](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L156):

| Ключ | Тип Go | В тексте `.h` |
|---|---|---|
| `grpc-service-name` | `string` | да |
| `grpc-user-agent` | `string` | нет, есть в ядре |
| `ping-interval` | `int` | нет, есть в ядре |
| `max-connections` | `int` | нет, есть в ядре; VLESS передаёт в `gun.NewClient` |
| `min-streams` | `int` | нет, есть в ядре |
| `max-streams` | `int` | нет, есть в ядре |

Отсутствие ключа — нулевое значение. Четыре ключа сверх `grpc-service-name` — кандидаты в `UnsupportedFeature`, если импорт их не валидирует отдельно. Молча отдать их в ядро было бы расширением `.h`.

## XHTTP есть в этом пине

Только VLESS: поле `xhttp-opts` и `network: xhttp` ([`vless.go` 82 и 256](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L82)). В switch VMess и Trojan этой ветки нет.

`XHTTPOptions` ([`vless.go` 93](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L93)):

- `path`, `host`, `mode` — `string`
- `headers` — `map[string]string`
- `no-grpc-header`, `x-padding-obfs-mode` — `bool`
- `x-padding-bytes`, `x-padding-key`, `x-padding-header`, `x-padding-placement`, `x-padding-method`, `uplink-http-method`, `session-placement`, `session-key`, `session-table`, `session-length`, `seq-placement`, `seq-key`, `uplink-data-placement`, `uplink-data-key`, `uplink-chunk-size`, `sc-max-each-post-bytes`, `sc-min-posts-interval-ms` — `string`
- `reuse-settings`: `max-concurrency`, `max-connections`, `c-max-reuse-times`, `h-max-request-times`, `h-max-reusable-secs` (`string`), `h-keep-alive-period` (`int`, в клиенте умножается на секунду)
- `download-settings`: свои `path`/`host`/`headers`/`reuse-settings` и вложенные поля узла (`server`, `port`, `tls`, `alpn`, security-opts, `skip-cert-verify`, сертификат)

Пустой `mode` становится `auto` ([`transport/xhttp/config.go` 70](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/transport/xhttp/config.go#L70)). `auto` при REALITY даёт `stream-up`, если есть `download-settings`, иначе `stream-one`; без REALITY — `packet-up` (строка 77). Имя `stream-one` вместе с `download-settings` ядро отвергает ([`vless.go` 721](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L721)). Другие строки `mode` декодер не сверяет со списком: `NormalizedMode` возвращает их как есть.

Пустой `path` у XHTTP становится `/`. Путь без ведущего `/` ядро само дополняет слэшем ([`config.go` `NormalizedPath`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/transport/xhttp/config.go#L91)). Это не отказ.

HTTP/3 у XHTTP требует TLS и не сочетается с security mode ([`vless.go` 694](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L694)).

Ключи `.h` `path`, `host`, `mode` в ядре есть. Всё остальное из `XHTTPOptions` в текст `.h` не входит и остаётся кандидатом в `UnsupportedFeature`, пока для него нет своей проверки.
