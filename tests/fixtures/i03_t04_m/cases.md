# Fixtures `I03.T04.m`

Дата: 2026-10-06. Входов: 11. Хосты только из `192.0.2.0/24` и `10.8.0.0/16`. Composer переносит каталог без изменений. Тест с `include_str!` пишется до `native/proto/wg.rs` и не проходит, пока разбора WireGuard нет.

Длина ключа — 32 байта после `base64.StdEncoding`, как `NoisePrivateKeySize` в оракуле [oracle-wg-ss.md](../../oracle-wg-ss.md). У ядра эта проверка происходит в `IpcSet`, не в `NewWireGuard`. Здесь она на разборе: 31 и 33 байта — отказ, ключ не усекается и не дополняется.

Потолок `peers` — 64. У pinned ядра верхнего предела нет; план подзадачи требует лимит. 65 peer в `peers-over-limit.json` — отказ. Другое число Composer может выбрать только записью в отчёт, файл не менять.

`amnezia-wg-option`, `dialer-proxy`, `remote-dns-resolve`, `dns` отвергаются по отдельности. Узел без этого поля в том же файле не становится принятым за счёт удаления поля.

Маркер утечки для `.o` — строка base64 приватного ключа принятого узла. Её нет в status DTO, `Debug`, `Display` ошибки, diagnostic export и evidence.

| Маркер | Где лежит |
|---|---|
| `Q01GSVhNLXdnLXByaXZhdGUta2V5ISEAAAAAAAAAAAA=` | `marker-accepted.json`, поле `private-key` |
| `Q01GSVhNLXBlZXItbGltaXQta2V5ISEAAAAAAAAAAAA=` | `peers-over-limit.json`, поле `private-key` |
| `CMFIXM-not-base64` | `key-not-base64.json` |

## Случаи

| Файл | Результат | Класс | Риск |
|---|---|---|---|
| `marker-accepted.json` | принять | ключ только в закрытом blob | `Q01GSVhNLXdnLXByaXZhdGUta2V5ISEAAAAAAAAAAAA=` виден вне blob |
| `key-not-base64.json` | отказ | `InvalidNode` | не-base64 принят или текст `CMFIXM-not-base64` попал в ошибку |
| `key-31-bytes.json` | отказ | `InvalidNode` | base64 декодируется в 31 байт и принят. Строка ключа не короче 32 символов: у 31, 32 и 33 байт base64 одной длины группы |
| `key-33-bytes.json` | отказ | `InvalidNode` | 33 байта после декодирования приняты |
| `allowed-ips-bad-cidr.json` | отказ | `InvalidNode` | `10.8.0.0/33` принят |
| `peers-overlap.json` | отказ | `InvalidNode` | `10.8.0.0/16` и вложенный `10.8.1.0/32` приняты. У ядра такой проверки нет — это правило cm |
| `peers-over-limit.json` | отказ | `InvalidNode` | 65 peer принят. В отказе нет `Q01GSVhNLXBlZXItbGltaXQta2V5ISEAAAAAAAAAAAA=` |
| `amnezia-wg-option.json` | отказ | `UnsupportedFeature` → `UnsupportedNodeFeature` | блок удалён, узел принят. В отказе нет `Q01GSVhNLXdnLXByaXZhdGUta2V5ISEAAAAAAAAAAAA=` |
| `dialer-proxy.json` | отказ | `RestrictedNodeOption` | `dialer-proxy` удалён, узел принят |
| `remote-dns-resolve.json` | отказ | `RestrictedNodeOption` | флаг удалён, узел принят |
| `dns.json` | отказ | `RestrictedNodeOption` | список `dns` удалён, узел принят |
