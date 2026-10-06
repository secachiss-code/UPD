# Оракул полей: WireGuard и Shadowsocks

Пин mihomo: `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. Дата: 2026-10-06.

Длины ключей и имена шифров сняты с кода модулей, которые `go.mod` этого commit закрепляет. Документация репозитория не использовалась.

| Модуль | Версия в `go.mod` |
|---|---|
| `github.com/metacubex/sing-shadowsocks2` | `v0.2.8` |
| `github.com/metacubex/chacha` | `v0.1.5` (зависимость sing-shadowsocks2) |
| `github.com/metacubex/sing` | `v0.5.8` (`uot.Version = 2`, `uot.LegacyVersion = 1`) |
| `github.com/metacubex/wireguard-go` | `v0.0.0-20250820062549-a6cecdd7f57f` |
| `github.com/metacubex/sing-wireguard` | `v0.0.0-20260826105301-c3ae17d19f9e` |

Неизвестный ключ декодер `proxy` молча отбрасывает. См. [oracle-transports.md](oracle-transports.md).

## Shadowsocks: какой список вообще вызывается

[`adapter/outbound/shadowsocks.go` `NewShadowSocks`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/shadowsocks.go#L301) вызывает `shadowsocks.CreateMethod` из `sing-shadowsocks2`. Корень модуля пустым импортом регистрирует `shadowaead`, `shadowaead_2022` и `shadowstream`. Имя ищется как есть, без смены регистра. Нет в реестре — `unknown method`.

Список `transport/shadowsocks/core.ListCipher` / `PickCipher` этот конструктор не использует. Его использует Trojan `ss-opts` (см. [oracle-tls-protocols.md](oracle-tls-protocols.md)). Для `.n` таблица ниже.

`none` регистрируется отдельно и пароль не требует.

### AEAD, не 2022

`shadowaead.MethodList` и `keySaltLength` в `NewMethod`. Пароль не декодируется как base64: ключ — `legacykey.Key([]byte(password), keySaltLength)`. Готовый `options.Key` принимается только при точной длине.

| Имя | Байт ключа | Пометка в списке |
|---|---|---|
| `aes-128-gcm` | 16 | стандарт |
| `aes-192-gcm` | 24 | стандарт |
| `aes-256-gcm` | 32 | стандарт |
| `chacha20-ietf-poly1305` | 32 | стандарт |
| `xchacha20-ietf-poly1305` | 32 | стандарт |
| `chacha8-ietf-poly1305` | 32 | not standard |
| `xchacha8-ietf-poly1305` | 32 | not standard |
| `rabbit128-poly1305` | 16 | not standard |
| `aes-128-ccm` | 16 | not standard |
| `aes-192-ccm` | 24 | not standard |
| `aes-256-ccm` | 32 | not standard |
| `aes-128-gcm-siv` | 16 | not standard |
| `aes-256-gcm-siv` | 32 | not standard |
| `aegis-128l` | 16 | not standard |
| `aegis-256` | 32 | not standard |
| `aez-384` | 48 (`3 * 16`) | not standard |
| `deoxys-ii-256-128` | 32 | not standard |
| `lea-128-gcm` | 16 | not standard |
| `lea-192-gcm` | 24 | not standard |
| `lea-256-gcm` | 32 | not standard |
| `ascon128` | 16 | not standard |
| `ascon128a` | 16 | not standard |

Пустой пароль при отсутствии готового ключа — `ErrMissingPassword`.

### Поток

`shadowstream.MethodList`. Пароль так же через `legacykey.Key`. `chacha.KeySize` в `github.com/metacubex/chacha@v0.1.5` равен 32.

| Имя | Байт ключа |
|---|---|
| `aes-128-ctr` | 16 |
| `aes-192-ctr` | 24 |
| `aes-256-ctr` | 32 |
| `aes-128-cfb` | 16 |
| `aes-192-cfb` | 24 |
| `aes-256-cfb` | 32 |
| `rc4-md5` | 16 |
| `chacha20-ietf` | 32 |
| `xchacha20` | 32 |
| `chacha20` | 32 |

Имена в верхнем регистре (`AES-256-GCM`, `CHACHA20-IETF-POLY1305`) для этого конструктора — `unknown method`.

### `2022-blake3-*`

`shadowaead_2022.NewMethod`. Пароль режется по `:`. Каждый кусок — `base64.StdEncoding`. Ошибка base64 — `decode key`. Пустой список ключей — `ErrMissingPassword`.

| Имя | `keySaltLength` | Несколько ключей |
|---|---|---|
| `2022-blake3-aes-128-gcm` | 16 | да |
| `2022-blake3-aes-256-gcm` | 32 | да |
| `2022-blake3-chacha20-poly1305` | 32 | нет, `ErrNoEIH` если ключей больше одного |
| `2022-blake3-chacha8-poly1305` | 32 | нет, `ErrNoEIH` |
| `2022-blake3-aes-128-ccm` | 16 | да |
| `2022-blake3-aes-256-ccm` | 32 | да |

Каждый декодированный ключ должен иметь длину `keySaltLength`, иначе `bad key length, required <N>, got <M>`.

Мультипользовательский формат для AES-GCM и AES-CCM: несколько base64-кусков через `:`. Для всех, кроме первого, в `pskHash` кладутся первые `aes.BlockSize` (16) байт `blake3.Sum512(key)`. ChaCha-варианты больше одного ключа не принимают. Отдельного текстового формата «user key» помимо этого списка в `NewMethod` нет.

`2022-blake3-aes-128-gcm` и `2022-blake3-chacha20-poly1305` в списке стоят до комментария `began not standard methods`. `chacha8` и оба CCM — после него.

### `udp-over-tcp`

Ключи: `udp-over-tcp` (`bool`), `udp-over-tcp-version` (`int`).

[`shadowsocks.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/shadowsocks.go#L490): версия 2 и 1 проходят. 0 заменяется на 1 (`LegacyVersion`). Иное число — `unknown udp over tcp protocol version`. Адрес назначения: версия 2 → `sp.v2.udp-over-tcp.arpa`, версия 1 → `sp.udp-over-tcp.arpa` (`sing` `common/uot/protocol.go`).

### Плагины

Сравнение строки `plugin` в том же `NewShadowSocks`. Иное имя плагина в прочитанном конструкторе не отвергается отдельной ошибкой: ветки просто нет, плагин не поднимается.

| `plugin` | Опции (`plugin-opts`, тег `obfs`) | Отказ |
|---|---|---|
| `obfs` | `mode` (`tls` или `http`, иначе `obfs mode error`), `host` (пустое → `bing.com`) | да, на mode |
| `v2ray-plugin` | `mode` только `websocket`; `host`, `path`, `tls`, `headers`, `mux` (по умолчанию true), `v2ray-http-upgrade`, `v2ray-http-upgrade-fast-open`, `skip-cert-verify`, `fingerprint`, `name-cert-verify`, `certificate`, `private-key`, `ech-opts` | да, на mode |
| `gost-plugin` | `mode` только `websocket`; `host`, `path`, `headers`, `mux`, TLS-поля как у v2ray, `ech-opts` | да, на mode |
| `shadow-tls` | `version` (пустое в опциях до decode подменяется 2), `password`, `host`, `fingerprint`, `certificate`, `private-key`, `skip-cert-verify`, `name-cert-verify`, `alpn` | ошибка decode / конструктора |
| `restls` | `host`, `password`, `version-hint`, `restls-script`, `skip-cert-verify`, `fingerprint`, `name-cert-verify` | ошибка `NewRestlsConfig` |
| `jls` | разбирается `jls.Mode` (`"jls"`) | ошибка decode |
| `kcptun` | набор полей smux/kcp (`mode` внутри опций по умолчанию `fast`); после успеха `udp-over-tcp` принудительно true | ошибка decode |

`skip-cert-verify` внутри `v2ray-plugin`, `gost-plugin`, `shadow-tls` и `restls` — то же поле D2, если плагин поддержан. У самого шифра Shadowsocks (не плагина) ключа `skip-cert-verify` в структуре узла нет. Обфускация simple-obfs `tls`/`http` сертификат не проверяет этим ключом.

`client-fingerprint` есть на узле Shadowsocks и передаётся в shadow-tls и restls.

## WireGuard

[`adapter/outbound/wireguard.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/wireguard.go).

### Ключи

`private-key`, `public-key`, `pre-shared-key` и `header-protection-key` декодируются `base64.StdEncoding` в `NewWireGuard`. Ошибка base64 — отказ сразу (`decode private key` и аналоги). Длину декодированных байт `NewWireGuard` не проверяет: байты переводятся в hex и позже уходят в IPC.

Проверка длины — `device.IpcSet` → `NoisePrivateKey.FromMaybeZeroHex` → `loadExactHex` (`wireguard-go` `device/noise-types.go`):

| Константа | Байт |
|---|---|
| `NoisePublicKeySize` | 32 |
| `NoisePrivateKeySize` | 32 |
| `NoisePresharedKeySize` | 32 |

Иная длина hex — `hex string does not fit the slice`, снаружи `failed to set private_key`. Это происходит в `init0` при первом подъёме устройства, не в `NewWireGuard`. 32 нулевых байта `FromMaybeZeroHex` не отвергает.

Пустой `pre-shared-key` не декодируется. Пустой `private-key` на base64 падает сразу. `public-key` обязателен в ветке без `peers` и у каждого элемента `peers`.

### `reserved` и `peers`

`reserved` узла: пустой допустим. Непустой должен быть ровно из 3 байт, иначе `invalid reserved value, required 3 bytes`. То же для `reserved` каждого peer.

`peers` короче 2 элементов включает режим connect (один peer или адрес узла). Длины списка сверху `NewWireGuard` не ставит. Пересечения `allowed-ips` код не ищет.

У peer непустой список требует непустой `allowed-ips`, иначе `missing allowed_ips for peer`. Поля peer: `server`, `port`, `public-key`, `pre-shared-key`, `reserved`, `allowed-ips`.

Локальный адрес: `ip` без `/` дополняется `/32` (строка 327). Пустой набор префиксов — `missing local address`. `mtu` 0 становится 1408.

### Поля, которые двигают хост

Текст `.m` помечает их Restricted/Unsupported. В ядре они рабочие:

| Ключ | Эффект |
|---|---|
| `dialer-proxy` | наследуется из `BasicOption`, попадает в `ProxyInfo.DialerProxy` |
| `remote-dns-resolve` вместе с непустым `dns` | строится резолвер, у каждого nameserver `ProxyAdapter` — сам WireGuard |
| `dns` | список строк, `dns.ParseNameServer`; ошибка разбора — отказ конструктора |
| `ip-stack.mode` | `auto` (по умолчанию), `gvisor`, `mips`. Иное — `invalid IP stack mode`. `gvisor` требует build tag `with_gvisor` |
| `ip-stack.congestion-controller` | пусто, `cubic`, `reno`, `bbr`, `bbr3`. Иное — отказ. Это стек TUN, не QUIC TUIC |
| `refresh-server-ip-interval` | секунды между перезаписью IPC |

`amnezia-wg-option`: `version == 3` выбирает `amneziav3.NewDevice`, любое другое значение — legacy `amnezia.NewDevice`. Присутствие блока отключает разбор reserved (`SetParseReserved(false)`).

Ключи блока: `version`, `jc`, `jmin`, `jmax`, `s1`, `s2`, `s3`, `s4`, `h1`–`h4` (строки), `i1`–`i5`, `j1`–`j3`, `itime`, `header-protection-key` (base64), `content-padding-addition`, `rekey-after-time`, `rekey-timeout`, `reject-after-time`, `keepalive-timeout`, `max-handshake-attempts`, `random-trailers`, `disable-cookies`. Комментарии в структуре помечают часть полей как v1.5 / v2 / v3; отдельного отказа «поле не от той версии» в `NewWireGuard` нет, ненулевые значения пишутся в IPC.

`interface`, `routing-mark`, `ip-version` приходят из `BasicOption` и тоже задают, откуда уходит сокет.

## Что текст `.m` / `.n` ужесточает относительно ядра

| Текст | Ядро |
|---|---|
| `.m`: неверная длина ключа → отказ на разборе | `NewWireGuard` проверяет только base64. 32 байта требует `IpcSet` при старте устройства |
| `.m`: пересекающиеся peers → отказ | Проверки пересечения `allowed-ips` нет |
| `.m`: `amnezia-wg-option`, `dialer-proxy`, `remote-dns-resolve`, `dns` → Restricted/Unsupported | В ядре поля применяются |
| `.n`: шифры | Имена строго как в таблицах выше, нижний регистр. Верхний регистр старого `PickCipher` здесь `unknown method` |
