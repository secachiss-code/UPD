# Пины ядер: mihomo и Xray (I05.T01.a)

Дата: 2026-10-06. Проверил координатор. Бинарники лежат вне репозитория, в `~/.cache/cm-cores/` (без root и без установки). В git они не попадают.

## mihomo 1.19.32 (ядро I03/I04)

| Поле | Значение |
|---|---|
| Тег | `v1.19.32` → коммит `88dcbf7f1614a67c3b36b848ee3592dfa92ada36` (`git ls-remote`). Совпадает с `PINNED_CORE_COMMIT` в `src/sources/capabilities.rs` |
| Ассет | `mihomo-linux-amd64-compatible-v1.19.32.gz` (GOAMD64 v1: работает на любом x86-64, в том числе на раннерах CI) |
| sha256 архива | `ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20`: совпадает с digest ассета на странице релиза GitHub и с хешем скачанного файла |
| sha256 бинарника | `7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1` |
| `-v` | `Mihomo Meta v1.19.32 linux amd64 with go1.26.8 Wed Sep 30 16:55:19 UTC 2026`, tags `with_gvisor` |
| Путь для тестов | `CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo` |
| Лицензия | GPL-3.0. Ядро — внешний бинарник, запускается отдельным процессом (Q25), не линкуется с CM |

`FlClashCore` из пакета FlClash (`/usr/lib/flclash`) — это обёртка, которая ждёт unix-сокет FlClash. Как CLI `mihomo -t` её использовать нельзя.

**Проверено:** `tests/audit_i03_core_check.rs` с `CM_TEST_MIHOMO` прошёл, корпус из 12 узлов (http, hysteria2, socks5, ss, trojan, tuic, vless, vmess) ядро принимает.

**Ограничение `-t`:** ядро принимает `{"type":"vless","port":0,"uuid":"bad"}` и печатает `test is successful`. Значит, `mihomo -t` проверяет только структуру конфига, а не допустимость значений. Строгая проверка значений остаётся за парсером CM (I03.T04.b), а `-t` — дополнительный слой, не замена.

## Xray 26.3.27 (I05)

| Поле | Значение |
|---|---|
| Версия | `v26.3.27` — последний стабильный релиз (`releases/latest`). Более новые теги 26.7–26.9 помечены как Pre-release и в пин не берутся |
| Тег | `v26.3.27` → коммит `d2758a023cd7f4174a5a5fa4ff66e487d4342ba0` |
| Ассет | `Xray-linux-64.zip` |
| sha256 архива | `23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae`. Источник 1 — `Xray-linux-64.zip.dgst` из релиза (`SHA2-256`), источник 2 — digest ассета на GitHub. Оба совпали со скачанным файлом |
| sha512 (из `.dgst`) | `e8bc40a0687cac184bbe4b5c1f047e69064ccedc489fb25e208889ae287bbf8736dff16b108d68fc00dc33edc8bb53502e47a9698a277f4f51b67b83d899e518` |
| `xray` | sha256 `8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed`; `Xray 26.3.27 … d2758a0 (go1.26.1 linux/amd64)` |
| `geoip.dat` | sha256 `744c97b74c52bae2ac8664fef6ac481d7765cb8432a0df54f0368a88b9b4a354` |
| `geosite.dat` | sha256 `adf92de0cfc70e458b399f04c5f912bf42d115ed7e37281b30e2f1c68605e4e9` |
| Лицензия | MPL-2.0 (`LICENSE` в архиве) |
| Путь | `~/.cache/cm-cores/xray/xray` |

## Как повторить

Через API GitHub (`api.github.com`) отсюда приходит 403 (лимит без токена), а обычные страницы и загрузки работают. Список ассетов с digest — `https://github.com/<repo>/releases/expanded_assets/<tag>`, файлы — `…/releases/download/<tag>/<asset>`. Скачивание обрывалось по таймауту и докачивалось через `curl -C -`.
