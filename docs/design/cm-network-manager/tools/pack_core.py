"""Пакет Grok, часть 1: доставка ядер (I05.T04), Xray (I05.T03), сеть приложений (I08, I09, I10).

Данные для pack_dag.py. Формат — как в i06_dag.py.
"""

DIRECTIONS = [
    ("D", "Доставка ядер", "Скачать закреплённую версию, проверить, распаковать с пределами, установить атомарно с откатом."),
    ("X", "Xray", "Конфиг и жизненный цикл второго ядра через тот же CoreAdapter."),
    ("N", "Сеть приложения", "Свой netns на туннель: veth → правило iif → таблица → TUN worker-а; при сбое трафик блокируется."),
]

V = []
E = []


def vertex(vid, direction, title, size, level, deps, files, spec, external=(), covers=()):
    V.append(dict(id=vid, direction=direction, title=title, size=size, level=level, deps=deps,
                  files=files, spec=spec, external=list(external), covers=list(covers)))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


# ───────────────────────────── D: доставка ядер ─────────────────────────────
vertex("I05.D1", "D", "Описать закреплённые версии ядер и проверку sha256.", "S", "L1", [],
       ["src/core/delivery/mod.rs", "src/core/delivery/pin.rs", "src/core/mod.rs"],
       """\
`pub mod delivery;` в `src/core/mod.rs`.
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum CoreKind { Mihomo, Xray }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Archive { Gzip, Zip { member: &'static str } }
pub struct Pin { pub kind: CoreKind, pub version: &'static str, pub url: &'static str, pub archive: Archive,
                 pub archive_sha256: &'static str, pub binary_sha256: &'static str,
                 pub max_archive: u64, pub max_binary: u64, pub version_marker: &'static str }
pub static PINS: &[Pin];
pub fn pin(kind: CoreKind) -> &'static Pin
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), DeliveryError>
```
Значения — из [I05-PIN.md](I05-PIN.md), без изменений:

| kind | version | url | archive | archive_sha256 | binary_sha256 | version_marker |
|---|---|---|---|---|---|---|
| Mihomo | v1.19.32 | `https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-linux-amd64-compatible-v1.19.32.gz` | Gzip | `ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20` | `7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1` | `v1.19.32` |
| Xray | v26.3.27 | `https://github.com/XTLS/Xray-core/releases/download/v26.3.27/Xray-linux-64.zip` | Zip { member: "xray" } | `23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae` | `8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed` | `Xray 26.3.27` |

`max_archive` = 64 МиБ, `max_binary` = 128 МиБ для обоих.

`#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum DeliveryError { HashMismatch, TooLarge, BadArchive, MemberMissing, Fetch, VersionMismatch, ConfigRejected, Io, NothingToRollBack }` — `Display` фиксированными фразами, без путей и URL.

`verify_sha256`: сравнение без учёта регистра hex; длина ≠ 64 или не hex → `HashMismatch`.""",
       covers=["I05.T04.a"])

vertex("I05.D2", "D", "Распаковать gzip и один файл из zip с пределами размера.", "M", "L1", ["I05.D1"],
       ["src/core/delivery/unpack.rs"],
       """\
```rust
pub fn unpack_gzip(bytes: &[u8], max: u64) -> Result<Vec<u8>, DeliveryError>
pub fn unpack_zip_member(bytes: &[u8], member: &str, max: u64) -> Result<Vec<u8>, DeliveryError>
```
`unpack_gzip`: `flate2::read::GzDecoder`, чтение через `take(max + 1)`; больше `max` → `TooLarge`; ошибка потока → `BadArchive`.

`unpack_zip_member` — собственный минимальный читатель (крейта zip в зависимостях нет, новых не добавлять):
1. найти End of Central Directory (`PK\\x05\\x06`) в последних 65557 байтах; нет → `BadArchive`;
2. zip64 (любое поле 0xFFFF/0xFFFFFFFF), многотомный архив (disk ≠ 0) → `BadArchive`;
3. пройти центральный каталог (`PK\\x01\\x02`), не больше 4096 записей; запись с точным именем `member` (без `/`, без `..`) — искомая; нет → `MemberMissing`;
4. флаг шифрования (bit 0) → `BadArchive`; метод только 0 (stored) или 8 (deflate), иначе `BadArchive`;
5. заявленный несжатый размер > `max` → `TooLarge` до распаковки;
6. локальный заголовок (`PK\\x03\\x04`) по смещению; данные в пределах файла, иначе `BadArchive`;
7. deflate — `flate2::read::DeflateDecoder` через `take(max + 1)`; фактический размер > `max` → `TooLarge`; фактический размер ≠ заявленному → `BadArchive`;
8. CRC-32 результата (`flate2::Crc`) ≠ записанному → `BadArchive`.

Все смещения и длины — с проверкой переполнения и границ; паник на произвольном входе нет.""",
       covers=["I05.T04.a"])

vertex("I05.D3", "D", "Получить бинарник ядра: загрузка, проверка архива, распаковка, проверка бинарника.", "M", "L1", ["I05.D2"],
       ["src/core/delivery/fetch.rs"],
       """\
```rust
pub trait Download { fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DeliveryError>; }
pub struct TransportDownload;   // через sources::transport::FetchTransport (H.04): предел тела, общий deadline
pub fn obtain(pin: &Pin, download: &dyn Download) -> Result<Vec<u8>, DeliveryError>
```
`obtain` строго по порядку:
1. `download.get(pin.url, pin.max_archive)`;
2. `verify_sha256(archive, pin.archive_sha256)`;
3. распаковка по `pin.archive`;
4. `verify_sha256(binary, pin.binary_sha256)`.

Первый отказ прерывает цепочку. Бинарник с неверным хешем наружу не возвращается.

`TransportDownload`: только `https://`, ошибки транспорта → `Fetch`, превышение предела → `TooLarge`. URL берётся только из `Pin`, не из аргументов пользователя.""",
       covers=["I05.T04.a"])

vertex("I05.D4", "D", "Установить кандидата атомарно и уметь откатиться.", "M", "L1", ["I05.D3"],
       ["src/core/delivery/install.rs"],
       """\
Раскладка под `<root>/<kind>/` (`kind` — `mihomo` или `xray`; в продукте `<root>` = `/var/lib/cm/cores`):
- `versions/<version>/<kind>` — бинарник 0755, каталог 0755;
- `current` → `versions/<version>/<kind>` (симлинк);
- `previous` → прежняя цель `current` (симлинк, может отсутствовать).

```rust
pub struct Installed { pub version: String, pub path: PathBuf }
pub fn install(root: &Path, pin: &Pin, binary: &[u8],
               probe: &dyn Fn(&Path) -> Result<String, DeliveryError>,
               accept: &dyn Fn(&Path) -> Result<(), DeliveryError>) -> Result<Installed, DeliveryError>
pub fn rollback(root: &Path, kind: CoreKind) -> Result<Installed, DeliveryError>
pub fn current(root: &Path, kind: CoreKind) -> Option<Installed>
```
`install`:
1. записать `versions/<version>.tmp/<kind>` (0755), `fsync` файла и каталога;
2. `probe(path)` — вывод версии кандидата; не содержит `pin.version_marker` → `VersionMismatch`;
3. `accept(path)` — вызывающий проверяет кандидатом действующие конфиги; ошибка → `ConfigRejected`;
4. переименовать `<version>.tmp` → `<version>` (если каталог версии уже есть — заменить);
5. `previous` ← прежняя цель `current` (если была и отличается);
6. `current` переключается атомарно: симлинк `current.tmp`, затем `rename` поверх `current`.

Отказ на шагах 1–3 удаляет `.tmp` и не трогает `current` и `previous`. Работающие процессы продолжают исполнять прежний файл: каталоги старых версий не удаляются.

`rollback`: `previous` нет → `NothingToRollBack`; иначе `current` и `previous` меняются местами (тем же атомарным способом).

`cm vpn core update` в эту вершину не входит: существующую команду не менять.""",
       covers=["I05.T04.b"])

# ───────────────────────────── X: Xray ─────────────────────────────
vertex("I05.X1", "X", "Построить config Xray из узлов Store.", "L", "L1", [],
       ["src/core/xray/mod.rs", "src/core/xray/config.rs", "src/core/mod.rs"],
       """\
`pub mod xray;` в `src/core/mod.rs`. Вход — тот же, что у генератора mihomo: объекты узлов в формате mihomo (`definition.full_definition()`), выход — JSON Xray.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XrayConfigError { Empty, InvalidNode, Unsupported(&'static str), InvalidPort }
pub fn outbound(node: &serde_json::Value, tag: &str) -> Result<serde_json::Value, XrayConfigError>
pub fn generate_config(nodes: &[serde_json::Value], leased_port: u16) -> Result<Vec<u8>, XrayConfigError>
```
`generate_config` (первый узел — выбранный):
```json
{"log":{"loglevel":"warning"},
 "inbounds":[{"tag":"cm","listen":"127.0.0.1","port":<leased_port>,"protocol":"mixed","settings":{"udp":true}}],
 "outbounds":[<outbound(nodes[0], "node")>, {"tag":"direct","protocol":"freedom"}, {"tag":"block","protocol":"blackhole"}],
 "routing":{"rules":[{"type":"field","inboundTag":["cm"],"outboundTag":"node"}]}}
```
`leased_port` 0 → `InvalidPort`; `nodes` пуст → `Empty`.

Соответствие `type` узла mihomo → outbound Xray:

| mihomo | Xray `protocol` и `settings` |
|---|---|
| `ss` (`server, port, cipher, password`) | `shadowsocks`, `{"servers":[{"address","port","method":cipher,"password"}]}` |
| `trojan` (`server, port, password`) | `trojan`, `{"servers":[{"address","port","password"}]}` |
| `vmess` (`server, port, uuid, alterId, cipher`) | `vmess`, `{"vnext":[{"address","port","users":[{"id":uuid,"alterId":alterId или 0,"security":cipher или "auto"}]}]}` |
| `vless` (`server, port, uuid, flow`) | `vless`, `{"vnext":[{"address","port","users":[{"id":uuid,"encryption":"none","flow":flow — только если задан}]}]}` |

`streamSettings` (добавляется, только если есть что задавать):
- `network`: mihomo `network` (`tcp` по умолчанию, `ws`, `grpc`); иное → `Unsupported("network")`;
- `ws-opts` → `wsSettings {"path": path, "headers": {"Host": …}}` (поле `headers` — только если Host задан);
- `grpc-opts.grpc-service-name` → `grpcSettings {"serviceName": …}`;
- `tls: true` (у trojan — всегда) → `"security":"tls"`, `tlsSettings {"serverName": servername или sni, "allowInsecure": skip-cert-verify — только если true, "fingerprint": client-fingerprint — только если задан}`;
- `reality-opts` → `"security":"reality"`, `realitySettings {"serverName", "publicKey": public-key, "shortId": short-id, "fingerprint": client-fingerprint или "chrome"}`.

Отказы:
- `type` не из таблицы (в том числе `tuic`, `hysteria2`, `wireguard`, `http`, `socks5`) → `Unsupported(<type>)` — без тихой замены;
- нет обязательного поля, порт вне 1..=65535, uuid не в формате 8-4-4-4-12 → `InvalidNode`;
- поля узла, меняющие маршрут мимо CM (`dialer-proxy`, `interface-name`, `routing-mark`) → `Unsupported("dialer")`.

Ошибки не содержат значений узла (адресов, паролей, uuid).""",
       external=["I05.T01.b"], covers=["I05.T03.a"])

vertex("I05.X2", "X", "Вести жизненный цикл Xray через CoreAdapter.", "L", "L2", ["I05.X1"],
       ["src/core/xray/lifecycle.rs"],
       """\
`pub struct XrayWorker` — те же каталоги и аренды, что у `MihomoWorker` (`InstanceRoot`, `LeaseRegistry`, порт), `impl CoreAdapter`:

- `capabilities()`: `core: "xray"`, `version: "26.3.27"`, `reload_without_restart: false`, `delay_probe: false`.
- `validate(config)`: файл во временном каталоге instance, `xray run -test -c <file>` с `env_clear()`, тайм-аут 20 с; код ≠ 0 → `InvalidConfig`. Вывод ядра в ошибку не попадает.
- `start(config)`: `CoreConfig` — документ Xray от `generate_config` с портом-заглушкой; адаптер подставляет арендованный порт в `inbounds[0].port` (и отказывает `InvalidConfig`, если входящих не ровно один, он не `mixed` или слушает не `127.0.0.1`), пишет `config.json` 0600, `validate`, запускает `xray run -c <file>` в новой сессии (`setsid`), ждёт до 5 с, пока порт принимает TCP. Готовность: `ApiState::ApiReady` (процесс жив и порт принимает), `RouteState::RouteReady`, `RemoteState::Unknown`.
- `reload(config)`: `Err(CoreError::Unsupported)` — у Xray нет перезагрузки без рестарта ([I05-GAPS.md](I05-GAPS.md)). Отдельный метод `pub fn restart(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError>`: `validate` нового конфига → `stop` → `start`; если новый не прошёл `validate`, работающий процесс не трогается.
- `health()`: процесс жив и порт принимает → прежняя готовность; иначе `DOWN`. `remote` всегда `Unknown`.
- `statistics()`: `Err(CoreError::Unsupported)`.
- `stop()`: как у `MihomoWorker` — SIGTERM группе, через 2 с SIGKILL, освобождение аренд.
- `set_run_as(RunAs)` — как у `MihomoWorker` (I06.I3).

Общий с `MihomoWorker` код (ожидание порта, остановка группы, запись приватного файла) вынести в `src/core/process.rs` и использовать в обоих; копий не оставлять. Остановка своего потомка — `process::terminate_child(&mut Child)`: ожидание прекращается по `try_wait`, иначе завершившийся лидер (зомби) держит группу «живой» все две секунды. В `lifecycle.rs` mihomo — только замена тел этих функций на вызовы.""",
       external=["I06.I3"], covers=["I05.T03.b"])

# ───────────────────────────── N: сеть приложения ─────────────────────────────
vertex("I08.N1", "N", "Вывести все имена и адреса сети туннеля из его номера.", "S", "L1", [],
       ["src/net/mod.rs", "src/net/plan.rs", "src/lib.rs"],
       """\
`pub mod net;` в `src/lib.rs`.
```rust
pub const MAX_TUNNELS: u8 = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelNet {
    pub index: u8,              // 0..MAX_TUNNELS, из аренды
    pub netns: String,          // "cm-<index>"
    pub veth_host: String,      // "cmv<index>h"
    pub veth_ns: String,        // "cmv<index>n"
    pub host_addr: String,      // "10.213.<index>.1/30"
    pub ns_addr: String,        // "10.213.<index>.2/30"
    pub gateway: String,        // "10.213.<index>.1"
    pub tun: String,            // "cmtun<index>"
    pub tun_addr: String,       // "198.18.<index>.1/30"
    pub dns_addr: String,       // "198.18.<index>.2"
    pub table: u32,             // 100 + index
    pub rule_priority: u32,     // 1000 + index
    pub mtu: u32,               // 1400
    pub owner_uid: u32,
}
pub fn tunnel_net(index: u8, owner_uid: u32) -> Result<TunnelNet, NetError>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError { BadIndex, CommandFailed, Timeout, Drift, Busy, Unsupported }
```
`index >= MAX_TUNNELS` → `BadIndex`. Функция чистая: одинаковый вход — одинаковый результат. Диапазоны `10.213.0.0/16` и `198.18.0.0/16` — константы модуля с комментарием: путь данных проверен лабораторией `tools/packet_flow_lab.sh`.""",
       covers=["I08.T01.b"])

vertex("I08.N2", "N", "Составить команды создания и удаления сети туннеля.", "M", "L1", ["I08.N1"],
       ["src/net/commands.rs"],
       """\
```rust
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Program { Ip, Nft, Sysctl }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct Cmd { pub program: Program, pub args: Vec<String>, pub stdin: Option<String> }
pub fn create_commands(t: &TunnelNet, ipv6: Ipv6Policy) -> Vec<Cmd>
pub fn destroy_commands(t: &TunnelNet) -> Vec<Cmd>
```
`create_commands` — ровно в этом порядке (значения для index 0, uid 1000):
1. `ip netns add cm-0`
2. `ip link add cmv0h type veth peer name cmv0n`
3. `ip link set cmv0n netns cm-0`
4. `ip addr add 10.213.0.1/30 dev cmv0h`
5. `ip link set cmv0h up`
6. `ip -n cm-0 link set lo up`
7. `ip -n cm-0 addr add 10.213.0.2/30 dev cmv0n`
8. `ip -n cm-0 link set cmv0n up`
9. `ip -n cm-0 route add default via 10.213.0.1`
10. `ip netns exec cm-0 sysctl -qw net.ipv6.conf.all.disable_ipv6=1` — только при `Ipv6Policy::Block`
11. `ip tuntap add dev cmtun0 mode tun user 1000`
12. `ip addr add 198.18.0.1/30 dev cmtun0`
13. `ip link set cmtun0 mtu 1400 up`
14. `ip route add blackhole default metric 200 table 100`
15. `ip route add default dev cmtun0 table 100`
16. `ip rule add iif cmv0h lookup 100 priority 1000`
17. `sysctl -qw net.ipv4.ip_forward=1`

Blackhole (14) ставится **раньше** маршрута в TUN (15) и правила (16): ни в какой момент трафик veth не попадает в основную таблицу.

`destroy_commands` — обратный порядок по смыслу: `ip rule del iif cmv0h lookup 100 priority 1000`, `ip route flush table 100`, `ip link del cmtun0`, `ip link del cmv0h`, `ip netns del cm-0`. `ip_forward` не выключается (им могут пользоваться другие).

Аргументы — отдельные элементы `args`, без оболочки. Шаг 10 — `Program::Ip` с аргументами `netns exec cm-0 sysctl …`.""",
       covers=["I08.T02.a", "I09.T02.a"])

vertex("I08.N3", "N", "Отрисовать таблицу nftables для всех туннелей.", "S", "L1", ["I08.N1"],
       ["src/net/firewall.rs"],
       """\
`pub fn render_table(tunnels: &[TunnelNet]) -> String` — точный текст (для туннелей 0 и 3; пары в порядке возрастания index):
```
table inet cm {
  chain forward {
    type filter hook forward priority filter; policy accept;
    iifname "cmv0h" oifname "cmtun0" counter accept
    iifname "cmtun0" oifname "cmv0h" counter accept
    iifname "cmv3h" oifname "cmtun3" counter accept
    iifname "cmtun3" oifname "cmv3h" counter accept
    iifname "cmv*" counter drop
    oifname "cmv*" counter drop
  }
}
```
- Политика цепочки `accept`: чужой forwarding (docker, libvirt) не затрагивается. Запрещается только трафик интерфейсов CM мимо своего TUN.
- Два последних правила есть всегда, даже при пустом списке туннелей.
- `pub fn replace_commands(tunnels: &[TunnelNet]) -> Vec<Cmd>`: одна команда `nft -f -` со stdin `"table inet cm\\ndelete table inet cm\\n" + render_table(...)` — атомарная замена таблицы одной транзакцией nft.
- Имя цепочки `forward`: `fwd` — зарезервированное слово nft.""",
       covers=["I08.T03.a", "I09.T02.a"])

vertex("I08.N4", "N", "Выполнять команды сети с откатом при отказе.", "M", "L2", ["I08.N2", "I08.N3"],
       ["src/net/exec.rs"],
       """\
```rust
pub trait NetExec { fn run(&mut self, cmd: &Cmd) -> Result<(), NetError>; }
pub struct SystemExec;                      // /usr/bin/ip, /usr/bin/nft, /usr/bin/sysctl; env_clear; тайм-аут 10 с
pub struct RecordingExec { pub ran: Vec<Cmd>, pub fail_at: Option<usize> }
pub fn create(t: &TunnelNet, ipv6: Ipv6Policy, all: &[TunnelNet], exec: &mut dyn NetExec) -> Result<(), NetError>
pub fn destroy(t: &TunnelNet, remaining: &[TunnelNet], exec: &mut dyn NetExec) -> Result<(), NetError>
```
`create`:
1. `replace_commands(all)` — таблица с запретами ставится **до** появления интерфейсов;
2. `create_commands` по порядку;
3. отказ команды → выполнить `destroy_commands` (ошибки отката игнорировать, кроме последней — её вернуть как `CommandFailed`) и вернуть `CommandFailed`.

`destroy`: `destroy_commands` — каждая команда выполняется, даже если предыдущая отказала (объект мог не существовать); затем `replace_commands(remaining)`.

`SystemExec`: программа — только по фиксированному абсолютному пути; код ≠ 0 → `CommandFailed`; вывод команды в ошибку не попадает; `stdin` передаётся через pipe.""",
       covers=["I08.T02.a"])

vertex("I08.N5", "N", "Включить TUN и перехват DNS в конфиге worker-а.", "M", "L2", ["I08.N1"],
       ["src/core/mihomo/config.rs"],
       """\
`pub fn attach_tun(document: &Value, t: &TunnelNet) -> Result<Vec<u8>, ConfigError>` — рядом с `attach_worker_listeners`:
- входной документ проходит те же запреты, что worker (`HOST_LISTENER_KEYS`, GEO-правила); ключи `tun` и `dns` во входе → `Forbidden`;
- добавляется:
```json
"tun": {"enable": true, "device": "<t.tun>", "stack": "gvisor", "auto-route": false, "auto-redirect": false,
        "auto-detect-interface": false, "mtu": <t.mtu>, "inet4-address": ["<t.tun_addr>"],
        "dns-hijack": ["any:53", "tcp://any:53"]},
"dns": {"enable": true, "ipv6": false, "enhanced-mode": "redir-host",
        "nameserver": ["https://1.1.1.1/dns-query", "https://dns.google/dns-query"],
        "default-nameserver": ["1.1.1.1", "8.8.8.8"]}
```
- `listeners` в TUN-режиме не добавляются; `external-controller-unix` по-прежнему ставит только `attach_instance_controller`.

`MihomoWorker`: новый метод `pub fn set_tunnel_net(&mut self, t: TunnelNet)`. Если задан, `start` и `reload` используют `attach_tun` вместо `attach_worker_listeners`, порт не арендуется, а готовность маршрута — «у интерфейса `t.tun` стоят флаги `IFF_UP` и `IFF_RUNNING`» вместо ожидания порта. Флаги читаются через `ioctl(SIOCGIFFLAGS)`, а не из `/sys/class/net`: TUN поднят самим CM ещё до запуска ядра, а `IFF_RUNNING` появляется, только когда ядро открыло устройство; sysfs к тому же показывает то сетевое пространство, в котором смонтирован.

`pub fn resolv_conf(t: &TunnelNet) -> String` в `src/net/dns.rs` → `"nameserver <t.dns_addr>\\noptions edns0\\n"`; файл кладётся в `/etc/netns/<t.netns>/resolv.conf` командой из I08.N6.""",
       covers=["I10.T01.a", "I10.T02.a"])

vertex("I08.N6", "N", "Создавать и удалять сеть туннеля операциями контроллера.", "L", "L2",
       ["I08.N4", "I08.N5"],
       ["src/controller/net_ops.rs", "src/controller/dispatch.rs", "src/core/leases.rs"],
       """\
Заполняет `NetApply` и `NetRevert`, которые в I06 отвечают `unsupported`.

- `ResourceKind::Tunnel` в `src/core/leases.rs`: значения `"0"`..`"63"` (первое свободное), держатель — `u<uid>-<instance>`.
- `NetApply { instance, generation }` — транзакция журнала владельца, шаги:
  1. `lease_tunnel` — apply: аренда номера; compensate: освобождение;
  2. `resolv_conf` — apply: каталог `/etc/netns/cm-<i>` 0755 и файл `resolv.conf` 0644 (в `test_mode` корень — `<base>/etc-netns`); compensate: удаление;
  3. `net_create` — apply: `net::exec::create`; compensate: `net::exec::destroy`.
  Ответ — новый вариант `ReplyData::Net { index: u8, netns: String }`.
- `NetRevert { instance, generation }` — шаги в обратном порядке: `net_destroy`, `resolv_conf` удалить, аренду освободить. Сети нет → `NotRunning`.
- Проверка поколения — как у worker (`check_start` / `check_running`), отдельная запись в `Generations` с ключом `net:<instance>`.
- `WorkerStart` для экземпляра, у которого есть сеть: `Workers::start` вызывает `adapter.set_tunnel_net(...)` до `start` (фабрика получает `Option<TunnelNet>`).
- Исполнитель — `Arc<Mutex<dyn NetExec + Send>>` в `Deps`: в продукте `SystemExec`, в тестах `RecordingExec`.
- `reconcile` (I06) получает компенсации этих шагов: после обрыва сеть либо создана целиком, либо удалена.

Класс операции и действие polkit — `Net` / `io.github.cm.net` (без изменений).""",
       external=["I06.W2"], covers=["I08.T02.a", "I08.T04.a"])

vertex("I09.N7", "N", "Сверять желаемое состояние сети с фактическим.", "M", "L1", ["I08.N1"],
       ["src/net/observe.rs"],
       """\
```rust
pub struct Observed { pub rules: Vec<ObservedRule>, pub routes: BTreeMap<u32, Vec<ObservedRoute>>, pub links: Vec<String>, pub netns: Vec<String> }
pub struct ObservedRule { pub priority: u32, pub iif: Option<String>, pub table: String }
pub struct ObservedRoute { pub dst: String, pub dev: Option<String>, pub kind: Option<String> /* "blackhole" */, pub metric: Option<u32> }
pub fn parse_rules(json: &str) -> Result<Vec<ObservedRule>, NetError>      // вывод `ip -j rule`
pub fn parse_routes(json: &str) -> Result<Vec<ObservedRoute>, NetError>    // вывод `ip -j route show table N`
pub fn parse_links(json: &str) -> Result<Vec<String>, NetError>            // вывод `ip -j link`, поле ifname
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drift { MissingBlackhole(u8), MissingRule(u8), MissingTunRoute(u8), MissingVeth(u8), MissingTun(u8), MissingNetns(u8), OrphanVeth(String), OrphanRule(u32) }
pub fn audit(desired: &[TunnelNet], observed: &Observed) -> Vec<Drift>
pub fn repair_commands(drift: &Drift, desired: &[TunnelNet]) -> Vec<Cmd>
```
- Нераспознанный JSON → `Drift` не выдаётся, функция разбора возвращает `Err(NetError::Drift)`.
- `audit`: для каждого желаемого туннеля проверяются blackhole (dst `default`, kind `blackhole`, таблица `t.table`), маршрут в TUN, правило (`priority`, `iif`), интерфейсы, netns. Интерфейс `cmv<N>h` без желаемого туннеля → `OrphanVeth`. Правило с приоритетом `1000+N` и `iif cmv<N>h` без желаемого туннеля → `OrphanRule`; чужое правило с тем же приоритетом (другой `iif` или без него) сиротой не считается.
- `repair_commands`: `MissingBlackhole` → команда 14 из I08.N2; `MissingRule` → команда 16; `OrphanVeth(name)` → `ip link del <name>`; `OrphanRule(p)` → `ip rule del iif cmv<p-1000>h priority <p>`. Для `MissingVeth`, `MissingTun`, `MissingNetns` — пустой список: сеть пересоздаётся целиком через `NetRevert` и `NetApply`, а не чинится по частям.
- Порядок результата `audit` — по index, внутри — в порядке перечисления вариантов.""",
       covers=["I09.T03.a"])

vertex("I09.N8", "N", "Вести состояние туннеля конечным автоматом.", "S", "L1", [],
       ["src/net/state.rs"],
       """\
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelState { Stopped, Starting, Up, Degraded, Blocked, Failed }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelEvent { StartRequested, NetReady, CoreReady, RemoteLost, RemoteBack, CoreDown, DriftFound, StopRequested, Repaired }
pub fn next(state: TunnelState, event: TunnelEvent) -> TunnelState
pub fn net_axis(state: TunnelState) -> VerificationValue
pub fn passes_traffic(state: TunnelState) -> bool
```
Переходы (остальные пары оставляют состояние прежним):

| Состояние | Событие → новое состояние |
|---|---|
| Stopped | StartRequested → Starting |
| Starting | CoreReady → Up; CoreDown → Failed; DriftFound → Blocked; StopRequested → Stopped |
| Up | RemoteLost → Degraded; CoreDown → Blocked; DriftFound → Blocked; StopRequested → Stopped |
| Degraded | RemoteBack → Up; CoreDown → Blocked; DriftFound → Blocked; StopRequested → Stopped |
| Blocked | CoreReady → Up; Repaired → Starting; StopRequested → Stopped |
| Failed | StartRequested → Starting; StopRequested → Stopped |

- `NetReady` в `Starting` состояние не меняет (ждём ядро).
- `net_axis`: Up → Verified; Degraded → Partial; Blocked → Blocked; Failed → Error; Stopped и Starting → Unknown.
- `passes_traffic`: только Up и Degraded. `Blocked` — трафик приложения не идёт никуда (blackhole), а не напрямую.
- Остановка туннеля с живыми сессиями приложений (Q12): вызывающий оставляет сеть (`NetRevert` не вызывается) — приложение остаётся в `Blocked`; это правило записать в doc-комментарии `next`.""",
       covers=["I09.T01.a", "I09.T04.a"])

# ───────────────────────────── рёбра ─────────────────────────────
edge("K01", "I05.D1", "I05.D2",
     "Пины совпадают с I05-PIN.md, а проверка хеша отвергает любое расхождение.",
     "`tests/audit_pack_delivery.rs`.",
     [
         "pin(Mihomo).archive_sha256 == \"ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20\"; binary_sha256 == \"7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1\"",
         "pin(Xray).archive_sha256 == \"23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae\"; binary_sha256 == \"8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed\"; archive == Zip{member \"xray\"}",
         "каждый url начинается с `https://github.com/` и содержит версию пина",
         "verify_sha256(b\"{}\", \"44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a\") Ok; тот же hex в верхнем регистре Ok; один изменённый символ → HashMismatch; строка 63 символа → HashMismatch",
         "с `CM_TEST_MIHOMO`: sha256 файла ядра == pin(Mihomo).binary_sha256; с `CM_TEST_XRAY`: == pin(Xray).binary_sha256 (без переменных — SKIPPED)",
     ])
edge("K02", "I05.D2", "I05.D3",
     "Распаковка не выдаёт больше предела и не паникует на произвольных байтах.",
     "`tests/audit_pack_delivery.rs`: архивы строятся в тесте (`flate2` и ручная сборка zip).",
     [
         "gzip из 1000 байт, max 1000 → Ok; max 999 → TooLarge; обрезанный на 10 байт → BadArchive",
         "gzip-бомба: 64 МиБ нулей, max 1 МиБ → TooLarge, пик памяти теста не растёт до 64 МиБ (чтение через take)",
         "zip с членами `LICENSE` и `xray` (deflate): member \"xray\" → его байты; member \"nope\" → MemberMissing",
         "zip, где у `xray` заявлен размер 10, а в потоке 20 байт → BadArchive; заявлен размер > max → TooLarge",
         "zip с неверным CRC → BadArchive; с флагом шифрования → BadArchive; метод 12 (bzip2) → BadArchive",
         "member \"../xray\" и \"a/xray\" → MemberMissing; запись в архиве с именем `../xray` не выбирается по запросу \"xray\"",
         "zip без EOCD, пустой вход, 22 байта мусора → BadArchive",
         "мутация: 20000 вариантов корректного zip (xorshift, seed 1; замена, вставка, удаление байта) — без паник, результат Ok или один из кодов DeliveryError",
         "с `CM_TEST_XRAY_ZIP` (скачанный Xray-linux-64.zip): unpack_zip_member(\"xray\") даёт sha256 == pin(Xray).binary_sha256 (без переменной — SKIPPED)",
     ])
edge("K03", "I05.D3", "I05.D4",
     "Наружу выходит только бинарник, прошедший обе проверки хеша.",
     "`tests/audit_pack_delivery.rs`: подменный `Download` и тестовый `Pin`.",
     [
         "корректный архив → Ok(binary); порядок вызовов: get один раз с max_bytes == pin.max_archive",
         "архив с другим содержимым → HashMismatch, распаковка не вызывалась (архив-бомба с неверным хешем не распаковывается)",
         "верный хеш архива, но binary_sha256 другой → HashMismatch",
         "Download возвращает Fetch → Fetch; TooLarge → TooLarge",
         "TransportDownload::get(\"http://example.invalid/x\", 10) → Fetch (не https) без сетевого запроса",
     ])
edge("K04", "I05.D4", "PACK.Z",
     "Обрыв на любом шаге установки оставляет рабочий `current`; откат возвращает предыдущую версию.",
     "`tests/audit_pack_delivery.rs` в `TempDirGuard`.",
     [
         "install v1 → current указывает на versions/v1/mihomo, файл 0755, previous отсутствует",
         "install v2 → current → v2, previous → v1; файл v1 на месте",
         "probe возвращает строку без version_marker → VersionMismatch; current по-прежнему v2; каталога `.tmp` нет",
         "accept возвращает ошибку → ConfigRejected; current по-прежнему v2",
         "rollback → current → v1, previous → v2; ещё один rollback → current → v2",
         "rollback без previous → NothingToRollBack",
         "current — всегда симлинк; после каждого шага `readlink` даёт существующий исполняемый файл",
     ])
edge("K05", "I05.X1", "I05.X2",
     "Каждый поддержанный узел даёт конфиг, который принимает закреплённый Xray; неподдержанное отклоняется явно.",
     "`tests/audit_pack_xray.rs`: точное сравнение JSON и `xray run -test` при `CM_TEST_XRAY`.",
     [
         "ss {server 203.0.113.8, port 443, cipher aes-128-gcm, password x} → `{\"tag\":\"node\",\"protocol\":\"shadowsocks\",\"settings\":{\"servers\":[{\"address\":\"203.0.113.8\",\"port\":443,\"method\":\"aes-128-gcm\",\"password\":\"x\"}]}}`",
         "vless {uuid 11111111-2222-4333-8444-555555555555, tls true, servername example.invalid, network ws, ws-opts {path /p, headers {Host h.example}}} → protocol vless, users[0] {id, encryption none}, streamSettings {network ws, wsSettings {path /p, headers {Host h.example}}, security tls, tlsSettings {serverName example.invalid}}",
         "vless с reality-opts {public-key K, short-id S} → security reality, realitySettings {serverName, publicKey K, shortId S, fingerprint chrome}",
         "trojan {password x, sni example.invalid} → security tls всегда, tlsSettings.serverName example.invalid",
         "vmess без alterId и cipher → alterId 0, security auto",
         "type tuic, hysteria2, wireguard, http, socks5 → Err(Unsupported(<type>)); network h2 → Err(Unsupported(\"network\")); dialer-proxy → Err(Unsupported(\"dialer\"))",
         "порт 0 или 70000, uuid \"x\", нет server → Err(InvalidNode); текст ошибки не содержит адреса, пароля, uuid",
         "generate_config(&[], 20000) → Empty; generate_config(nodes, 0) → InvalidPort",
         "generate_config(ss, 20000): inbounds[0] == {tag cm, listen 127.0.0.1, port 20000, protocol mixed, settings {udp true}}; outbounds теги [node, direct, block]; routing.rules[0] == {type field, inboundTag [cm], outboundTag node}",
         "с `CM_TEST_XRAY`: `xray run -test` принимает конфиг для ss, vmess, vless+ws+tls, vless+reality, trojan (без переменной — SKIPPED)",
     ])
edge("K06", "I05.X2", "PACK.Z",
     "Xray проходит тот же жизненный цикл, что mihomo, а отсутствие reload видно в capabilities и в коде ошибки.",
     "`tests/audit_pack_xray.rs`: внутри `unshare -rn` с `CM_TEST_XRAY`, upstream — локальный shadowsocks не нужен: outbound `freedom` через узел-заглушку недоступен, проверяется только lifecycle.",
     [
         "capabilities: core \"xray\", reload_without_restart false, delay_probe false",
         "start → ApiReady, порт 127.0.0.1:<аренда> принимает TCP; config.json 0600 и содержит арендованный порт",
         "reload → Err(Unsupported), процесс прежний (тот же pid)",
         "restart с корректным конфигом → новый pid, порт принимает; restart с конфигом, который `-test` отвергает → Err(InvalidConfig), прежний pid жив",
         "statistics → Err(Unsupported); health после kill -9 → DOWN",
         "stop → нет процессов группы, аренды освобождены",
         "документ с двумя входящими или listen 0.0.0.0 → start → Err(InvalidConfig), аренда не удержана",
         "`audit_i04_lifecycle` (mihomo) проходит без изменений после выноса общего кода в src/core/process.rs",
     ], level="L2")
edge("K07", "I08.N1", "I08.N2",
     "Имена и адреса туннеля однозначны, не пересекаются между туннелями и укладываются в пределы ядра.",
     "`tests/audit_pack_net.rs`.",
     [
         "tunnel_net(0, 1000) == {netns cm-0, veth_host cmv0h, veth_ns cmv0n, host_addr 10.213.0.1/30, ns_addr 10.213.0.2/30, gateway 10.213.0.1, tun cmtun0, tun_addr 198.18.0.1/30, dns_addr 198.18.0.2, table 100, rule_priority 1000, mtu 1400, owner_uid 1000}",
         "tunnel_net(63, 1000): veth_host cmv63h, host_addr 10.213.63.1/30, tun cmtun63, table 163, rule_priority 1063",
         "tunnel_net(64, 1000) → Err(BadIndex)",
         "для всех index 0..63: имена интерфейсов ≤ 15 символов; все имена, адреса, таблицы и приоритеты попарно различны",
     ])
edge("K08", "I08.N2", "I08.N4",
     "Порядок команд не оставляет момента, когда трафик veth может уйти в основную таблицу.",
     "`tests/audit_pack_net.rs`: точное сравнение списков.",
     [
         "create_commands(tunnel_net(0,1000), Block) — ровно 17 команд из спецификации, в том же порядке, аргументы по одному",
         "при Ipv6Policy::Pass команды 10 нет (16 команд)",
         "индекс команды `route add blackhole default metric 200 table 100` меньше индекса `route add default dev cmtun0 table 100`, а тот меньше индекса `rule add iif cmv0h lookup 100 priority 1000`",
         "destroy_commands: [rule del iif cmv0h lookup 100 priority 1000; route flush table 100; link del cmtun0; link del cmv0h; netns del cm-0]",
         "ни один аргумент не содержит пробела, `;`, `|`, `$`",
     ])
edge("K09", "I08.N3", "I08.N4",
     "Таблица запрещает интерфейсам CM любой путь, кроме своего TUN, и не трогает чужой трафик.",
     "`tests/audit_pack_net.rs`: точный текст и `nft -c -f -` внутри `unshare -rn`.",
     [
         "render_table([0, 3]) == текст из спецификации байт в байт",
         "render_table([]) содержит оба правила `\"cmv*\" counter drop` и ни одного accept",
         "туннели переданы в порядке [3, 0] → пары всё равно в порядке 0, 3",
         "в тексте `policy accept;` и нет слова `fwd`",
         "replace_commands: одна команда Nft с args [\"-f\", \"-\"], stdin начинается с \"table inet cm\\ndelete table inet cm\\n\"",
         "внутри `unshare -rn`: `nft -c -f -` принимает stdin replace_commands([0, 3]); повторная загрузка той же таблицы проходит (замена, а не ошибка «уже существует»)",
     ], level="L2")
edge("K10", "I08.N4", "I08.N6",
     "Отказ любой команды создания сворачивает уже созданное; запреты стоят раньше интерфейсов.",
     "`tests/audit_pack_net.rs` с `RecordingExec`.",
     [
         "create: первая выполненная команда — Nft (таблица), затем 17 команд create_commands",
         "fail_at = k для каждого k из 1..=17: результат Err(CommandFailed); после отказа выполнены все команды destroy_commands",
         "destroy: все 5 команд выполняются, даже если первая вернула ошибку; последней идёт Nft с таблицей для remaining",
         "SystemExec: программа `Ip` запускается как `/usr/bin/ip`; окружение дочернего процесса пустое (проверка подставным `ip` невозможна — путь фиксирован; проверяется по исходнику: в `src/net/exec.rs` нет `Command::new(\"ip\")` без абсолютного пути)",
     ])
edge("K11", "I08.N5", "I08.N6",
     "Worker в TUN-режиме слушает только свой TUN, перехватывает DNS и не добавляет маршрутов сам.",
     "`tests/audit_pack_net.rs`: точный JSON и `mihomo -t` при `CM_TEST_MIHOMO`.",
     [
         "attach_tun(doc, tunnel_net(0,1000)).tun == {enable true, device cmtun0, stack gvisor, auto-route false, auto-redirect false, auto-detect-interface false, mtu 1400, inet4-address [198.18.0.1/30], dns-hijack [any:53, tcp://any:53]}",
         "в результате нет ключа `listeners`; `dns.enable` true, `dns.enhanced-mode` redir-host",
         "входной документ с ключом `tun` или `dns` → Err(Forbidden); с `mixed-port` → Err(Forbidden); с правилом GEOIP → Err(InvalidRule)",
         "resolv_conf(tunnel_net(0,1000)) == \"nameserver 198.18.0.2\\noptions edns0\\n\"",
         "с `CM_TEST_MIHOMO`: `mihomo -t` принимает результат attach_tun + attach_instance_controller",
     ], level="L2")
edge("K12", "I08.N1", "I08.N3",
     "Отрисовка таблицы использует только имена из TunnelNet.",
     "`tests/audit_pack_net.rs`.",
     [
         "для index 0..63 каждая строка accept содержит ровно veth_host и tun этого туннеля",
     ])
edge("K13", "I08.N1", "I08.N5",
     "TUN-конфиг и resolv.conf берут адреса из TunnelNet, а не из констант.",
     "`tests/audit_pack_net.rs`.",
     [
         "attach_tun для index 5: device cmtun5, inet4-address [198.18.5.1/30]; resolv_conf → nameserver 198.18.5.2",
     ])
edge("K14", "I08.N6", "PACK.Z",
     "Сеть туннеля создаётся и удаляется только транзакцией контроллера; после обрыва не остаётся половины сети.",
     "`tests/audit_pack_net_ops.rs`: `handle` с `RecordingExec` и подменным Authorizer; base в `TempDirGuard`.",
     [
         "net_apply(browser, gen 1) → ok, data {type net, index 0, netns cm-0}; аренда Tunnel \"0\" у держателя u<uid>-browser; файл <base>/etc-netns/cm-0/resolv.conf == \"nameserver 198.18.0.2\\noptions edns0\\n\"",
         "второй экземпляр → index 1; net_revert(browser) → аренда 0 свободна; следующий net_apply снова получает index 0",
         "Authorizer получает действие \"io.github.cm.net\"; при отказе → denied, RecordingExec пуст",
         "RecordingExec отказывает на 5-й команде create → ответ code failed; аренды нет; resolv.conf удалён; журнал: транзакция Aborted",
         "обрыв (`crash_before`) перед шагом net_create → после reconcile аренды нет и resolv.conf удалён",
         "net_revert без сети → not_running; net_apply с тем же id повторно → тот же ответ, команды не выполняются второй раз",
         "65-й туннель → code quota либо failed с освобождением всего (аренд Tunnel ровно 64)",
         "worker_start для экземпляра с сетью: фабрика получает Some(TunnelNet) с тем же index",
     ], level="L2")
edge("K15", "I09.N7", "PACK.Z",
     "Расхождение желаемого и фактического состояния сети обнаруживается и называется; чинится только безопасное.",
     "`tests/audit_pack_net.rs`: фикстуры JSON в `tests/fixtures/pack/` (сняты с `ip -j` в `unshare -rn` после create_commands).",
     [
         "фикстуры после create для index 0: audit → []",
         "из фикстуры маршрутов убран blackhole → [MissingBlackhole(0)]; repair → [ip route add blackhole default metric 200 table 100]",
         "убрано правило iif → [MissingRule(0)]; repair → [ip rule add iif cmv0h lookup 100 priority 1000]",
         "нет cmtun0 в links → [MissingTunRoute(0), MissingTun(0)] в этом порядке; repair для MissingTun → []",
         "лишний интерфейс cmv7h без желаемого туннеля → [OrphanVeth(\"cmv7h\")]; repair → [ip link del cmv7h]",
         "правило priority 1042 с iif cmv42h без туннеля → [OrphanRule(1042)], repair → `ip rule del iif cmv42h priority 1042`; правило priority 1042 без iif или с iif eth0 сиротой не считается; правило priority 32766 (main) не считается сиротой",
         "parse_rules(\"not json\") → Err(Drift)",
     ])
edge("K16", "I09.N8", "PACK.Z",
     "Автомат туннеля не имеет перехода, в котором трафик идёт при неготовом ядре.",
     "`tests/audit_pack_net.rs`: перебор всех 6 × 9 пар.",
     [
         "Stopped + StartRequested → Starting; Starting + CoreReady → Up; Up + RemoteLost → Degraded; Degraded + RemoteBack → Up",
         "Up + CoreDown → Blocked; Degraded + CoreDown → Blocked; Up + DriftFound → Blocked; Starting + DriftFound → Blocked",
         "Blocked + CoreReady → Up; Blocked + Repaired → Starting; Failed + StartRequested → Starting",
         "любое состояние + StopRequested → Stopped",
         "все пары вне таблицы оставляют состояние прежним (например Stopped + CoreReady → Stopped, Starting + NetReady → Starting)",
         "passes_traffic истинно только для Up и Degraded",
         "net_axis: Up → Verified, Degraded → Partial, Blocked → Blocked, Failed → Error, Stopped → Unknown, Starting → Unknown",
         "ни из одного состояния одним событием нельзя попасть в Up, кроме CoreReady и RemoteBack",
     ])
edge("K17", "I08.N1", "I09.N7",
     "Сверка ищет ровно те имена, таблицы и приоритеты, которые выдаёт TunnelNet.",
     "`tests/audit_pack_net.rs`.",
     [
         "для index 5: audit на пустом Observed → [MissingBlackhole(5), MissingRule(5), MissingTunRoute(5), MissingVeth(5), MissingTun(5), MissingNetns(5)]",
         "Observed, собранный из имён tunnel_net(5, 1000) (правило priority 1005 iif cmv5h table 105; маршруты blackhole и dev cmtun5 в таблице 105; интерфейсы cmv5h, cmtun5; netns cm-5) → []",
     ])
