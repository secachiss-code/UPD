# Fixtures `I03.T04.o`

Дата: 2026-10-06. Входов: 15. Сканер секретов пишется вместе с классификацией и до неё обязан находить эти маркеры, если они вышли из закрытого blob. Composer переносит каталог без изменений.

Закрытый blob — приватные байты определения узла (`full_definition` и материал credential). Маркер принятого документа есть только там.

Маркера нет ни в одном из мест:

- status DTO источника и узла;
- `Debug` разобранного значения, ошибки и artifact;
- `Display` ошибки;
- diagnostic export;
- evidence и текст, который тест печатает в stdout/stderr.

Если документ отвергнут, маркера нет нигде, включая текст ошибки. Отказ не эхо входа.

Список Composer в подзадаче: `uuid`, `password`, `private-key`, `pre-shared-key`, `obfs-password`, `auth`. Строки ниже с пометкой «вне списка» тоже секреты. Их отсутствие в перечне не разрешает печатать значение.

| Файл | Маркер | Поле | В списке Composer | Результат |
|---|---|---|---|---|
| `vless.json` | `c0ffee00-0001-4000-8000-000000000001` | `uuid` | да | принять, только blob |
| `vmess.json` | `c0ffee00-0002-4000-8000-000000000002` | `uuid` | да | принять, только blob |
| `ss.json` | `CMFIXO-ss-password` | `password` | да | принять, только blob |
| `trojan.json` | `CMFIXO-trojan-password` | `password` | да | принять, только blob |
| `hysteria2.json` | `CMFIXO-hy2-password` | `password` | да | принять, только blob |
| `hysteria2.json` | `CMFIXO-hy2-obfs` | `obfs-password` | да | принять, только blob |
| `tuic.json` | `c0ffee00-0003-4000-8000-000000000003` | `uuid` | да | принять, только blob |
| `tuic.json` | `CMFIXO-tuic-password` | `password` | да | принять, только blob |
| `tuic-token.json` | `CMFIXO-tuic-token` | `token` | вне списка | отказ `UnsupportedFeature`: узел v5 полный, лишнее поле — `token`. Маркера нет в ошибке, как и `CMFIXO-tuic-v4-password` и `c0ffee00-0009-4000-8000-000000000009` |
| `http.json` | `CMFIXO-http-user` | `username` | вне списка | принять, только blob |
| `http.json` | `CMFIXO-http-password` | `password` | да | принять, только blob |
| `socks.json` | `CMFIXO-socks-user` | `username` | вне списка | принять, только blob |
| `socks.json` | `CMFIXO-socks-password` | `password` | да | принять, только blob |
| `wireguard.json` | `Q01GSVhPLXdnLXByaXZhdGUta2V5ISEAAAAAAAAAAAA=` | `private-key` | да | принять, только blob |
| `wireguard.json` | `Q01GSVhPLXdnLXBzay1tYXJrZXIhISEAAAAAAAAAAAA=` | `pre-shared-key` | да | принять, только blob |
| `wireguard.json` | `Q01GSVhPLXdnLXBlZXItcHNrISEhISEAAAAAAAAAAAA=` | `peers[].pre-shared-key` | да, на peer | принять, только blob |
| `tls-pem.json` | `CMFIXO-tls-private-key` | `private-key` (PEM, не путь) | да | принять, только blob |
| `tls-pem.json` | `CMFIXO-tls-certificate` | `certificate` | вне списка | принять, только blob |
| `tls-path.json` | `/tmp/CMFIXO-tls-path.pem` | `private-key` путём хоста | да | отказ. Путь не читается и не попадает в ошибку |
| `auth-field.json` | `CMFIXO-auth` | `auth` | да, поля нет в схеме ss | отказ `UnsupportedField`. Маркера нет в ошибке. Рядом `CMFIXO-ss-password-2` тоже не печатается |
| `header-protection.json` | `Q01GSVhPLWhlYWRlci1wcm90ZWN0IQAAAAAAAAAAAAA=` | `amnezia-wg-option.header-protection-key` | вне списка | отказ `UnsupportedFeature` на блок Amnezia. Маркер и `Q01GSVhPLXdnLXByaXZhdGUta2V5ISEAAAAAAAAAAAA=` не печатаются |
| `ss-plugin-password.json` | `CMFIXO-plugin-password` | `plugin-opts.password` | вне списка | отказ `UnsupportedFeature`. Маркер и `CMFIXO-ss-password-3` не печатаются |

`public-key` в `wireguard.json` (`Q01GSVhPLXdnLXB1YmxpYy1rZXkhISEAAAAAAAAAAAA=`) секретом не считается: это открытый ключ peer. Приватные маркеры из той же таблицы секретами остаются.

Сканер гоняется по всем fixtures этого каталога и по каталогам `.b`, `.m`, `.r`, `.t`, `.u`, когда те подзадачи уже перенесены. Ноль совпадений вне blob.
