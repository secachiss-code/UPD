# Сетевой harness (H.11, решение D6)

Rootless: user namespace + network namespaces через `unshare -rn` / `nsenter`. Сеть хоста не
затрагивается, root не нужен. Проверено на хосте разработки (ядро 7.2, Garuda): veth,
адреса, TUN-устройство, `ip rule` по fwmark со своими таблицами, nftables с `policy drop`.

- `smoke.sh` — два namespace (client ↔ veth ↔ remote с TCP-echo): связь есть; `policy drop`
  на выходе клиента её закрывает (fail-closed); явное `accept` открывает. Запуск:
  `unshare -rn sh tests/netharness/smoke.sh` → `SMOKE_OK`.
- `tests/audit_h11_netharness.rs` — тот же сценарий из `cargo test`; без user namespaces —
  `SKIPPED` (в evidence так и пишется).

Дальше на этой основе: тестовые серверы ядер (mihomo/xray inbound) в remote-namespace,
когда есть бинарники; захват трафика — счётчики nftables и `ip -s link` (tcpdump на хосте
нет); сценарии отказов — `ip link set … down`, `tc netem` при наличии.

Границы: L2 не заменяет L3/L4. Реальные приложения, systemd, перезагрузка и
NetworkManager — на хосте по D10.
