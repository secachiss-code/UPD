# Пробелы Xray 26.3.27 относительно CoreAdapter (I05.T01.b)

Дата замера: 2026-10-08. Бинарник: `~/.cache/cm-cores/xray/xray`.

```text
Xray 26.3.27 (Xray, Penetrates Everything.) d2758a0 (go1.26.1 linux/amd64)
```

Пин и хеши — в [I05-PIN.md](I05-PIN.md). Адаптера Xray в коде нет. `CoreCapabilities` (`src/core/adapter.rs`) спрашивает две способности: `reload_without_restart` и `delay_probe`. Ниже — что отвечает этот бинарник. Живой handshake с сервером не выполнялся: решения про протоколы относятся к `xray run -test`, не к установленной сессии.

`xray run -test -dump` для неизвестного протокола завершается с кодом 0 и печатает JSON как есть. Принятым считается только `xray run -test` без `-dump`.

## Пробелы

### Полная перезагрузка конфига без рестарта — unsupported

`xray help` и `xray help api` не содержат команды reload. `SIGHUP` процесс не перечитывает конфиг, а завершает его.

Замер в `unshare -rn` (`ip link set lo up`, socks `127.0.0.1:18081`, API dokodemo `127.0.0.1:18082`):

```text
xray run -c run.json
# дождаться строки «Xray 26.3.27 started»
kill -HUP <pid>
```

Результат: `alive_after_hup=no`, `wait_status=129` (128+SIGHUP), `started_count=1`, `reload_count=0`.

Для будущего адаптера `reload_without_restart = false`. Полный новый конфиг — это рестарт процесса на следующем слое, не тихий успех `reload`.

Частичный обход, не равный reload: API меняет отдельные объекты без рестарта процесса. `xray help api` даёт `adi`, `ado`, `rmi`, `rmo`, `adrules`, `rmrules` (inbounds, outbounds, routing rules). Это не загрузка целого файла конфига и не повод ставить `reload_without_restart`.

### Запрос задержки по требованию — unsupported

`delay_probe` в CoreAdapter — это разовый запрос задержки, а не фоновый монитор. В `xray help` и `xray help api` команды delay нет.

В бинарнике есть поля конфига `json:"observatory"`, `json:"burstObservatory"` и `json:"probeURL"`. Это фоновая проверка живости. Подменой `delay_probe` она не является: решение **вне scope**. Для адаптера `delay_probe = false`.

### TUIC — unsupported

```text
xray run -test -c tuic.json
```

Код 23:

```text
unknown config id: tuic
```

Тот же код и та же форма ошибки у исходящего `not-a-protocol` (`unknown config id: not-a-protocol`). В бинарнике строки `tuic`, `TUIC` и `hysteria2` встречаются 0 раз. Обхода в этом пине нет.

## Не пробелы

### Hysteria с `version: 2`

Отдельного id `hysteria2` нет (0 вхождений). Протокол `hysteria` с `"version": 2` этот пин принимает:

```text
xray run -test -c hy2.json
Configuration OK.
```

`-dump` сохраняет `version`, `address` и `port`. Строка `hysteria` в бинарнике есть (744 вхождения).

### WireGuard

```text
xray run -test -c wg.json
Configuration OK.
```

`-dump` сохраняет `secretKey`, `peers` и `endpoint`. `xray wg` только выпускает ключи X25519 (`xray help wg`), это не проверка исходящего. Строка `wireguard` в бинарнике есть (1050 вхождений).

## Следствие

| Возможность CoreAdapter | Xray 26.3.27 |
|---|---|
| `reload_without_restart` | false. SIGHUP процесс завершает. Частичный API — не полный reload |
| `delay_probe` | false. Observatory в эту способность не входит |
| TUIC | не заявлять: `unknown config id: tuic` |
| Hysteria version 2 | схема конфига принимается |
| WireGuard outbound | схема конфига принимается |
