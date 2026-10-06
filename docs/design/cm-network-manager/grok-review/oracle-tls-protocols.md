# Оракул полей: TLS, REALITY и протоколы

Пин: `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. Дата: 2026-10-06.

Закрывает поля `I03.T04.c/.d/.e/.j/.k/.l`. Шифры Shadowsocks и ключи WireGuard — в [oracle-wg-ss.md](oracle-wg-ss.md). Транспорты — в [oracle-transports.md](oracle-transports.md).

Неизвестный ключ декодер `proxy` молча не кладёт в структуру. См. oracle-transports.md. Текст подзадач cm местами строже ядра: такие места собраны в конце, это факты, не смена решения.

## `client-fingerprint`

[`component/tls/utls.go` `GetFingerprint`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/component/tls/utls.go#L42).

Пустая строка и `none` возвращают `ok == false`, без uTLS. `random` выбирает одно из `chrome` / `safari` / `ios` / `firefox` по весам 6 / 3 / 2 / 1.

Ключи карты, при которых `ok == true`: `chrome`, `firefox`, `safari`, `ios`, `android`, `edge`, `360`, `qq`, `random`, `chrome120`, `firefox120`, `safari16`, `chrome_psk`, `chrome_psk_shuffle`, `chrome_padding_psk_shuffle`, `chrome_pq`, `chrome_pq_psk`, `randomized`. Последние пять с `chrome_psk*` и `randomized` в исходнике помечены как deprecated. `randomized` перезаписывается в `init`.

Любое другое значение: `ok == false` и предупреждение в лог, не ошибка декодера. Дальше [`transport/vmess/tls.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/transport/vmess/tls.go#L114): при `ok == false` и включённом REALITY соединение отвергается текстом `REALITY is based on uTLS, please set a client-fingerprint`. Без REALITY исполнение падает в обычный `tls.Client`. Неверное имя отпечатка само по себе узел не отвергает.

Поле есть у VLESS, VMess, Trojan, Shadowsocks (`proxy:"client-fingerprint,omitempty"`).

## `fingerprint` — pin сертификата, не браузер

[`component/ca/fingerprint.go` `NewFingerprintVerifier`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/component/ca/fingerprint.go#L14).

Имена `chrome`, `firefox`, `safari`, `ios`, `android`, `edge`, `360`, `qq`, `random`, `randomized` здесь ошибка: это не pin, для браузера нужно `client-fingerprint`. Иначе строка без пробелов и без `:`, hex, ровно 32 байта SHA-256. Не hex и иная длина — ошибка.

## REALITY

[`adapter/outbound/reality.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/reality.go#L13):

| Ключ | Тип | Проверка в `Parse` |
|---|---|---|
| `public-key` | `string` | `base64.RawURLEncoding`, ровно 32 байта, затем X25519. Иначе `invalid REALITY public key`. Пустая строка — `Parse` возвращает nil, REALITY не включается |
| `short-id` | `string` | hex, после декодирования не длиннее 8 байт (`RealityMaxShortIDLen`). Иначе `invalid REALITY short id` |
| `support-x25519mlkem768` | `bool` | флаг, без отдельного отказа |

Поле `reality-opts` есть у VLESS, VMess, Trojan. REALITY без успешного `client-fingerprint` не поднимается, см. выше.

VLESS ([`vless.go` `NewVless`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L536)): ShadowTLS, Restls, JLS и REALITY взаимно исключают друг друга. Если режим выбран и `tls` ложен, ошибка `<режим> requires TLS`. `realityConfig` передаётся в `streamTLSConn` (TCP и `http`), в gRPC и в XHTTP. Ветка `network: ws` REALITY в TLS не кладёт: либо `StreamTLSConn` без поля `Reality` (если включён ShadowTLS/Restls/JLS), либо `ca.GetTLSConfig` без REALITY. Ошибки «REALITY с WS» нет.

Trojan ([`trojan.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/trojan.go#L325)): те же четыре режима взаимно исключаются. Отдельной проверки «REALITY требует TLS» нет. `Reality` попадает в `StreamTLSConn` ветки по умолчанию (TCP). Ветка `ws` поле `Reality` не передаёт. Ветка `grpc` в `StreamConnContext` помечена `break` и обрабатывается в `dialContext`.

Проверки «REALITY только на TCP/gRPC/XHTTP» и «обязателен servername» в `NewVless` / `NewTrojan` нет. Пустой `servername` у VLESS подставляется хостом адреса в `streamTLSConn`.

## Общие TLS-поля

| Ключ | Где | Что делает ядро |
|---|---|---|
| `tls` | VLESS `bool` | без него security-режим (включая REALITY) отвергается |
| `servername` | VLESS, VMess | SNI; пустое значение заменяется хостом |
| `sni` | Trojan, Hysteria2, TUIC | то же назначение, другое имя ключа |
| `alpn` | Trojan, VMess, Hysteria2, TUIC, `[]string` | пустой массив, если ключ присутствует, затирает умолчание (`structure.Decode`) |
| `certificate`, `private-key` | строки в `ca.GetTLSConfig` | ядро читает PEM или путь. Отказ пути хоста — правило cm, не отказ ядра |
| `skip-cert-verify` | см. [D2-brief.md](D2-brief.md) | `InsecureSkipVerify` на hop к прокси |
| `name-cert-verify` | рядом с `fingerprint` | в текст `.c` не входит. Вместе с pin даёт `tls_verification: Pinned` по D2 |
| `ech-opts` | VLESS, VMess, Trojan, Hysteria2, TUIC | см. ниже. Текст `.c`: `UnsupportedFeature` |

`ech-opts` ([`adapter/outbound/ech.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/ech.go)):

| Ключ | Тип | Поведение |
|---|---|---|
| `enable` | `bool` | `false` — `Parse` возвращает nil |
| `config` | `string` | непустая строка: `base64.StdEncoding`. Ошибка декодирования — отказ. Декодированные байты отдаются как ECH config list |
| `query-server-name` | `string` | если `config` пуст, HTTPS-запрос ECH идёт на это имя, иначе на server name соединения |

## VLESS: `flow`, `packet-encoding`

[`vless.go` `NewVless`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L462). Константа `vless.XRV` = `xtls-rprx-vision` (ровно 16 символов). Рядом в том же файле объявлены `xtls-rprx-origin`, `xtls-rprx-direct`, `xtls-rprx-splice`, но `NewVless` их отвергает.

- Длина `flow` меньше 16: значение отбрасывается, ошибки нет, addons не создаются.
- Длина не меньше 16: берутся первые 16 символов. Они должны быть равны `xtls-rprx-vision`, иначе `unsupported xtls flow type`. Более длинная строка с этим префиксом принимается.
- Проверок «только TCP» и «только TLS/REALITY» для `flow` нет.

`packet-encoding`:

| Значение | Эффект |
|---|---|
| `packetaddr`, `packet` | `packet-addr = true`, `xudp = false` |
| любое другое, включая пустое | если `packet-addr` ещё не true, выставляется `xudp = true` |
| `xudp` как строка | попадает в ветку default и тоже включает XUDP |

Отдельного ключа `xudp` декодер читает (`proxy:"xudp,omitempty"`). Если после switch `XUDP` истинен, `packet-addr` сбрасывается.

`encryption` (`string`) уходит в `encryption.NewClient`. Пустая строка зависит от того конструктора; в тексте `.e` поля нет — кандидат на `UnsupportedFeature`, пока конструктор не сверен отдельно.

## VMess: padding, `alterId`, `packet-encoding`

[`vmess.go` `NewVmess`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L476) вызывает `sing-vmess` `v0.2.5` `NewClient` (модуль, не дерево mihomo).

`cipher` приводится к нижнему регистру. Допустимы: `auto`, `none`, `zero`, `aes-128-cfb`, `aes-128-gcm`, `chacha20-poly1305`. Иное — `ErrUnsupportedSecurityType`. Пустая строка тоже ошибка.

`global-padding` и `authenticated-length` — `bool`, включаются как опции клиента, без отказа.

`alterId` — `int`. Больше 0: включается legacy alter-id (HMAC-MD5, 16 байт в заголовке). 0 и отрицательные ошибку не дают и alter-id не включают. Верхней границы в `NewClient` нет.

`packet-encoding` у VMess, в отличие от VLESS, пустое значение в XUDP не превращает:

| Значение | Эффект |
|---|---|
| `packetaddr`, `packet` | `packet-addr = true` |
| `xudp` | `xudp = true` |
| иное | без ошибки и без смены флагов |

Если `xudp` истинен, `packet-addr` сбрасывается. Отдельный ключ `packet-addr` декодер тоже читает.

## Trojan

[`trojan.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/trojan.go).

`password` — строка, отдельной проверки пустоты в прочитанном конструкторе нет. `network`: `ws`, `grpc`, иначе TCP (комментарий `default tcp network`). Ветки `h2` нет: такое значение попадает в default и идёт как TCP плюс TLS, без ошибки «неизвестный network».

`udp` — `bool`. `sni`, `alpn`, `skip-cert-verify`, `client-fingerprint`, `fingerprint`, `reality-opts` — см. таблицы выше.

`ss-opts`: `enabled`, `method`, `password`. При `enabled: true` пустой пароль — ошибка `empty password`. Пустой `method` становится `AES-128-GCM`. Дальше `core.PickCipher` из `transport/shadowsocks/core` (старый список, имена в верхнем регистре), не `sing-shadowsocks2`. Текст `.j` требует `UnsupportedFeature` на всё `ss-opts`.

Рядом есть и в текст `.j` не входят: `ech-opts`, `shadow-tls-opts`, `restls-opts`, `jls-opts`, `name-cert-verify`, `certificate`, `private-key`.

## Hysteria2

[`hysteria2.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/hysteria2.go). `sing-quic` `v0.0.0-20260904234848-1c242664697a`.

`password` передаётся в клиент без отдельной проверки пустоты в `NewHysteria2`. `udp` в опциях нет: `UDP: true` задаётся всегда.

`obfs`: пустая строка — обфускация выключена. Непустая без `obfs-password` — `missing obfs password`. Значения из sing-quic: `salamander` (`ObfsTypeSalamander`), `gecko` (`ObfsTypeGecko`). Иное — `unknown obfs type`. Для gecko читаются `obfs-min-packet-size` и `obfs-max-packet-size`. Текст `.k` называет только salamander.

`ports` ([`common/utils/ranges.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/common/utils/ranges.go)): строка сегментов через `/` или `,`. Больше 28 сегментов — `too many ranges to use, maximum support 28 ranges`. Один сегмент — порт, `a-b` — диапазон. Если `a > b`, [`NewRange`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/common/utils/range.go) меняет концы местами, ошибки нет. Потолка на число портов внутри сегмента нет: `1-65535` разворачивается целиком. `port == 0` и пустой список портов — `invalid port`.

`hop-interval` разбирается только если `ports` непуст и дал хотя бы один порт. Пустая строка становится диапазоном 0–0. Начало 0 заменяется на `defaultHopInterval` (30 секунд). Начало меньше `minHopInterval` (5) поднимается до 5. Конец меньше начала поднимается до начала. Единица — секунды. Перевёрнутый интервал до этой логики уже выровнен `NewRange`.

`up` / `down` — [`StringToBps`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/common/utils/mbps.go):

- пустая строка и строка, не совпавшая с шаблоном, дают 0;
- голое целое число читается как мегабиты (`"%d Mbps"`);
- шаблон `^(\d+)\s*([KMGT]?)([Bb])ps$`: K/M/G/T — степени 1000, не 1024;
- суффикс `b` — биты, результат делится на 8; `B` — байты.

`sni`, `alpn`, `skip-cert-verify`, `fingerprint` — как в общей таблице. TLS минимум `VersionTLS13`.

В текст `.k` не входят и в ядре есть: `ech-opts`, `name-cert-verify`, `certificate`, `private-key`, `obfs` = `gecko`, `obfs-min-packet-size`, `obfs-max-packet-size`, `cwnd`, `bbr-profile`, `udp-mtu` (0 становится 1197), `handshake-timeout`, `realm-opts` (`enable`, `server-url`, `token`, `realm-id`, `stun-servers` и вложенные TLS-поля), окна QUIC `initial-stream-receive-window`, `max-stream-receive-window`, `initial-connection-receive-window`, `max-connection-receive-window`.

## TUIC

[`tuic.go` `NewTuic`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/tuic.go#L165).

Версия выбирается так: непустой `token` — клиент v4 (`GenTKN`, `ClientOptionV4`), даже если `uuid` тоже задан. Пустой `token` — v5: `uuid.FromStringOrNil` (невалидный uuid становится нулевым, ошибки нет) и `password`.

`udp-relay-mode`: строка ровно `quic` → режим QUIC. Любая другая, включая пустую, → NATIVE. Неизвестное значение ошибкой не является.

`congestion-controller` передаётся в [`SetCongestionController`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/transport/tuic/common/congestion.go#L16). `cwnd == 0` становится 32. Ветки: `cubic`, `new_reno`, `bbr_meta_v1`, `bbr_meta_v2` (проваливается в `bbr`), `bbr`. Пустая и любая иная строка ни одну ветку не выбирают: контроллер не ставится, ошибки нет.

`udp-over-stream-version`: допустимы `uot.Version` (2) и `uot.LegacyVersion` (1) из `sing` `v0.5.8`. 0 заменяется на 1. Иное — `unknown udp over stream protocol version`.

`disable-sni: true` очищает `ServerName` и ставит `InsecureSkipVerify`.

`reduce-rtt`, `alpn`, `sni`, `request-timeout` (миллисекунды) передаются в клиент. Отдельного отказа для пустого `uuid`/`password` в прочитанном конструкторе нет.

В текст `.l` не входят: `ip`, `heartbeat-interval`, `fast-open`, `max-open-streams`, `cwnd`, `bbr-profile`, `skip-cert-verify`, `name-cert-verify`, `fingerprint`, `certificate`, `private-key`, `recv-window-conn`, `recv-window`, `disable-mtu-discovery`, `max-datagram-frame-size`, `max-udp-relay-packet-size`, `ech-opts`, `udp-over-stream`, `udp-over-stream-version`. `bbr_meta_v1` и `bbr_meta_v2` текст `.l` не перечисляет.

## Расхождения текста подзадач с этим ядром

Это не смена D1–D3 и не правка очереди. Composer сверяет поведение ядра по этой таблице; где очередь cm уже требует отказ, оракул только фиксирует, что ядро само не отказывает.

| Текст подзадачи | Pinned ядро |
|---|---|
| `.d`: REALITY с WS → отказ; обязателен `servername`; только TCP/gRPC/XHTTP | Ошибки нет. WS не устанавливает REALITY. `servername` может быть пустым и подменяется хостом. H2 и `http` тоже получают `realityConfig` через `streamTLSConn` |
| `.e`: `flow` только `xtls-rprx-vision` и только TLS/REALITY на TCP | Префикс из 16 символов; короче 16 — молча без flow; TCP и TLS не проверяются |
| `.e`: `alterId > 0` «по источнику» | `> 0` включает legacy. Верхней границы нет. `<= 0` ошибки нет |
| `.j`: `network` h2 → отказ | Строка `h2` попадает в default и обрабатывается как TCP |
| `.k`: перевёрнутый диапазон портов → отказ | Концы меняются местами |
| `.k`: только `obfs: salamander` | `gecko` тоже принимается |
| `.l`: неизвестный congestion-controller → отказ | Строка игнорируется, контроллер не ставится |
| `.l`: v4 `token` — `UnsupportedFeature`, если нет отдельного решения | Ядро v4 принимает, когда `token` непустой |
