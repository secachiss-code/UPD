"""Пакет A: туннель хоста и выход ядра (I07), восстановление и отказы (I09), DNS (I10).

Данные для ab_dag.py. Формат — как в pack_core.py. Числа, порядок команд и ожидаемые
значения взяты из лабораторий: host_flow_lab.sh, dns_probe_lab.sh (см. ADR-HOST-FLOW, ADR-DNS).
"""

DIRECTIONS = [
    ("H", "Туннель хоста и выход ядра", "CM сам ставит маршруты и фильтр туннеля хоста; ядро любого туннеля выходит наружу мимо него, без привилегий."),
    ("F", "Отказы и восстановление", "После сбоя ядра, контроллера или перезагрузки состояние сходится само, а трафик не идёт напрямую."),
    ("D", "Политика туннеля и DNS", "Политика туннеля читается из каталога владельца; DNS — резолвер ядра этого туннеля."),
]

V = []
E = []


def vertex(vid, direction, title, size, level, deps, files, spec, external=(), covers=()):
    V.append(dict(id=vid, direction=direction, title=title, size=size, level=level, deps=deps,
                  files=files, spec=spec, external=list(external), covers=list(covers)))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


# ───────────────────────── H: туннель хоста ─────────────────────────
vertex("I07.H1", "H", "Вывести имена, адреса и номера таблиц туннеля хоста.", "S", "L1", [],
       ["src/net/host_plan.rs", "src/net/mod.rs"],
       """\
Новый файл, `pub mod host_plan;` в `src/net/mod.rs`. Значения постоянные: туннель хоста один на машину.

```rust
pub const HOST_INSTANCE: &str = "host";
pub const HOST_INDEX: u8 = 64;                 // сразу за диапазоном приложений 0..=63
pub const MAX_UPLINKS: usize = 8;
pub const MAX_UPLINK_ADDRS: usize = 32;
pub const BOOTSTRAP_DNS: [&str; 2] = ["1.1.1.1", "8.8.8.8"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostNet {
    pub tun: String,            // "cmtunh"
    pub tun_addr: String,       // "198.18.64.1/30"
    pub dns_addr: String,       // "198.18.64.2"
    pub sink: String,           // "cmsink"
    pub table: u32,             // 90
    pub sink_table: u32,        // 91
    pub rule_uplink: u32,       // 1900: + номер внешнего интерфейса
    pub rule_uplink_addr: u32,  // 1950: + сквозной номер адреса
    pub rule_main: u32,         // 1990
    pub rule_tun: u32,          // 2000
    pub rule_sink: u32,         // 2001
    pub mtu: u32,               // 1400
    pub owner_uid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uplink { pub dev: String, pub addrs: Vec<String> }

pub fn host_net(owner_uid: u32) -> HostNet;
impl HostNet { pub fn as_tunnel(&self) -> TunnelNet; }
```

`as_tunnel` нужен, чтобы ядро хоста запускалось тем же путём, что ядро приложения (`CoreAdapter::set_tunnel_net`, `config::attach_tun`). Поля результата:
- `index = HOST_INDEX`, `tun`, `tun_addr`, `dns_addr`, `mtu`, `owner_uid` — из `HostNet`;
- `netns`, `veth_host`, `veth_ns`, `host_addr`, `ns_addr`, `gateway` — пустые строки;
- `table = 90`, `rule_priority = 2000`.

`net::tunnel_net(64, …)` по-прежнему отвечает `BadIndex`: номер 64 приложению не выдаётся.""",
       covers=["I07.T01.a"])

vertex("I07.H2", "H", "Определить внешние интерфейсы и их адреса по выводу `ip -j`.", "S", "L1", [],
       ["src/net/uplink.rs", "src/net/mod.rs"],
       """\
Чистые функции разбора. Команды выполняет `I09.F0`.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRoute { pub dev: String, pub metric: u32 }

/// Вывод `ip -j route show default table main` (и того же с `-6`).
pub fn parse_default_routes(json: &str) -> Result<Vec<DefaultRoute>, NetError>;
/// Вывод `ip -j -4 addr show`: имя интерфейса → адреса `addr_info[].local` со `scope == "global"`.
pub fn parse_addrs(json: &str) -> Result<BTreeMap<String, Vec<String>>, NetError>;
pub fn uplinks(v4: &[DefaultRoute], v6: &[DefaultRoute], addrs: &BTreeMap<String, Vec<String>>) -> Vec<Uplink>;
```

Логика `uplinks`:
1. Взять интерфейсы из `v4`, упорядочить по `(metric, dev)`, убрать повторы. Затем добавить интерфейсы из `v6`, которых ещё нет, в том же порядке.
2. Выбросить `lo` и все имена с префиксом `cm` (свои TUN, veth и приёмник).
3. Оставить первые `MAX_UPLINKS`.
4. `addrs` интерфейса — из карты, по возрастанию; у интерфейса только с IPv6 список пуст. Когда общее число адресов достигает `MAX_UPLINK_ADDRS`, остальные адреса отбрасываются.

Разбор: `metric` отсутствует → 0; запись без `dev` пропускается; не JSON или не массив → `NetError::CommandFailed`.""",
       covers=["I07.T02.a"])

vertex("I07.H3", "H", "Составить команды создания, удаления и перестройки туннеля хоста.", "M", "L1", ["I07.H1", "I07.H2"],
       ["src/net/host_commands.rs", "src/net/mod.rs"],
       """\
```rust
pub fn host_create_commands(host: &HostNet, uplinks: &[Uplink]) -> Vec<Cmd>;
pub fn host_destroy_commands(host: &HostNet, uplinks: &[Uplink]) -> Vec<Cmd>;
pub fn uplink_commands(host: &HostNet, old: &[Uplink], new: &[Uplink]) -> Vec<Cmd>;
```

Все команды — `Program::Ip`. **Порядок создания** (сначала приёмник и его правило, правило туннеля — последним):
```
ip link add cmsink type dummy
ip link set cmsink up
ip route add default dev cmsink table 91
ip -6 route add default dev cmsink table 91
ip tuntap add dev cmtunh mode tun user <uid>
ip addr add 198.18.64.1/30 dev cmtunh
ip link set cmtunh mtu 1400 up
ip route add default dev cmtunh table 90
<правила внешних интерфейсов>
ip -6 rule add priority 1990 lookup main suppress_prefixlength 0
ip -6 rule add priority 2001 lookup 91
ip rule add priority 1990 lookup main suppress_prefixlength 0
ip rule add priority 2001 lookup 91
ip rule add priority 2000 lookup 90
```
**Правила внешних интерфейсов**: для интерфейса с номером `k` (с нуля) — `ip rule add priority <1900+k> oif <dev> lookup main` и то же с `ip -6`; затем для каждого его адреса со сквозным номером `j` — `ip rule add priority <1950+j> from <addr> lookup main`.

**Удаление**: те же правила в обратном порядке, `add` заменён на `del`, селектор полный (`ip rule del priority 2000 lookup 90`); затем `ip route flush table 90`, `ip route flush table 91`, `ip -6 route flush table 91`, `ip link del cmtunh`, `ip link del cmsink`.

**`uplink_commands`**: `old == new` → пусто. Иначе — удалить все правила набора `old`, затем добавить все правила набора `new` (в порядке создания).

IPv6 хоста в режиме туннеля блокируется: таблицы 90 для IPv6 нет.

Почему приёмник — `dummy`, а не `blackhole`: с `blackhole` проверка обратного пути отбрасывает ответы ядрам туннелей приложений (лаборатория, шаг 9).""",
       covers=["I07.T01.a", "I07.T02.a"])

vertex("I07.H4", "H", "Отрисовать таблицу nftables, которая направляет DNS хоста в туннель.", "S", "L1", ["I07.H1"],
       ["src/net/host_firewall.rs", "src/net/mod.rs"],
       """\
```rust
pub fn render_host_table(host: &HostNet) -> String;
pub fn host_firewall_commands(host: Option<&HostNet>) -> Vec<Cmd>;
```

Текст таблицы (отступы — два пробела, в конце перевод строки):
```
table ip cm_host {
  chain output {
    type nat hook output priority -100; policy accept;
    oifname != "cmtunh" oifname != "lo" ip daddr != { 1.1.1.1, 8.8.8.8 } meta l4proto { tcp, udp } th dport 53 counter dnat to 198.18.64.2
  }
}
```
Адреса исключения — `BOOTSTRAP_DNS` в порядке массива. Ими ядро находит свой сервер; его сокеты привязаны к внешнему интерфейсу и в туннель попасть не могут.

`host_firewall_commands`: одна команда `Program::Nft` с аргументами `-f -` и stdin:
- `Some(host)` → `"table ip cm_host\\ndelete table ip cm_host\\n"` + текст таблицы;
- `None` → `"table ip cm_host\\ndelete table ip cm_host\\n"`.

Таблицу `inet cm` (`net::firewall`) эта вершина не трогает.""",
       covers=["I07.T01.a", "I07.T01.b"])

vertex("I07.H5", "H", "Привязать исходящие сокеты ядра к внешнему интерфейсу.", "S", "L2", ["I07.H1"],
       ["src/core/mihomo/config.rs"],
       """\
Одна смысловая правка в `attach_tun`: `"auto-detect-interface": true` (было `false`).

Ядро само следит за маршрутом по умолчанию в `main` и привязывает исходящие сокеты к его интерфейсу. Привилегии для этого не нужны. Без привязки ядро хоста отвергает собственный трафик (`reject loopback connection`), а ядро приложения выходило бы через туннель хоста.

Для ядра хоста новой функции нет: `attach_tun(document, &host.as_tunnel())` даёт `device: "cmtunh"` и `inet4-address: ["198.18.64.1/30"]`.

Если после правки падает существующий тест, где вышестоящий сервер стоит на `127.0.0.1`, остановись и опиши это в отчёте: обходить привязку нельзя.""",
       covers=["I07.T02.a"])

vertex("I07.H6", "H", "Спланировать переход между режимами хоста и проверить, что переход возможен.", "S", "L1", [],
       ["src/net/host.rs", "src/net/mod.rs"],
       """\
Чистые функции.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostStep { StopWorker, RevertNet, ApplyNet(HostMode), StartWorker }
pub fn transition(from: HostMode, to: HostMode) -> Vec<HostStep>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostBlock { LegacyCm, ForeignAutoRoute, Resolved }
pub struct Integration { pub network_manager: bool, pub firewalld: bool, pub resolved: bool }
pub fn detect(run_root: &Path) -> Integration;
pub fn preflight(mode: HostMode, links: &[String], rules: &[ObservedRule], integration: &Integration) -> Result<(), HostBlock>;
```

`transition` (`HostMode` — из `profiles`):

| Из → в | Шаги |
|---|---|
| одинаковые | пусто |
| off → proxy, off → tunnel | `ApplyNet(to)`, `StartWorker` |
| proxy → off, tunnel → off | `StopWorker`, `RevertNet` |
| proxy → tunnel, tunnel → proxy | `StopWorker`, `RevertNet`, `ApplyNet(to)`, `StartWorker` |

`detect`: `network_manager` — существует `<run_root>/NetworkManager`; `firewalld` — `<run_root>/firewalld`; `resolved` — `<run_root>/systemd/resolve/stub-resolv.conf`. В продукте `run_root` — `/run`.

`preflight` для `HostMode::Tunnel`, проверки по порядку:
1. среди `links` есть `cm-vpn` → `LegacyCm` (работает нынешний VPN хоста);
2. есть правило с `table == "2022"` → `ForeignAutoRoute` (ядро с `auto-route` или чужой клиент на его основе);
3. `integration.resolved` → `Resolved` (перенаправление DNS для него не проверено).

Для `Off` и `Proxy` — всегда `Ok(())`. `network_manager` и `firewalld` переход не блокируют: они только показываются в статусе.""",
       covers=["I07.T03.a", "I07.T04.a"])

vertex("I09.F0", "F", "Снять фактическое состояние сети командами `ip -j`.", "M", "L2", [],
       ["src/net/query.rs", "src/net/mod.rs", "src/controller/net_ops.rs", "src/controller/server.rs"],
       """\
До сих пор `net::observe::audit` получал `Observed` только в тестах. Эта вершина даёт настоящий источник.

```rust
pub trait NetQuery { fn query(&mut self, program: Program, args: &[&str]) -> Result<String, NetError>; }
pub struct SystemQuery;                       // запуск как в SystemExec, но stdout читается
pub struct FixedQuery { pub answers: BTreeMap<String, String>, pub asked: Vec<String> }  // ключ — аргументы через пробел
pub const MAX_QUERY_BYTES: usize = 1 << 20;

pub fn observe(query: &mut dyn NetQuery, tables: &[u32]) -> Result<Observed, NetError>;
pub fn observe_uplinks(query: &mut dyn NetQuery) -> Result<Vec<Uplink>, NetError>;
```

`SystemQuery`: те же пути программ, что у `SystemExec`; `env_clear`, stdin и stderr — `/dev/null`; тайм-аут 10 с; вывод длиннее `MAX_QUERY_BYTES` или код выхода не 0 → `CommandFailed`.

`FixedQuery`: записывает ключ в `asked`; нет ответа на ключ → `CommandFailed`.

`observe` выполняет по порядку:
- `ip -j rule show` → `parse_rules`;
- для каждой таблицы из `tables` — `ip -j route show table <N>` → `parse_routes`; ошибка команды для таблицы означает пустую таблицу;
- `ip -j link show` → `parse_links`;
- `ip netns list` → имена: первое слово каждой непустой строки.

`observe_uplinks`: `ip -j route show default table main`, `ip -j -6 route show default table main`, `ip -j -4 addr show` → `uplink::uplinks`. Ошибка команды с `-6` означает пустой список.

`NetCtx` получает поле `pub query: Arc<Mutex<dyn NetQuery + Send>>`; в `server.rs` — `SystemQuery`.""",
       covers=["I09.T03.a"])

vertex("I07.H7", "H", "Создавать и снимать туннель хоста операциями контроллера.", "L", "L2",
       ["I07.H3", "I07.H4", "I07.H5", "I07.H6", "I09.F0"],
       ["src/controller/host_ops.rs", "src/controller/protocol.rs", "src/controller/actions.rs", "src/controller/dispatch.rs",
        "src/controller/core_ops.rs", "src/controller/mod.rs", "src/main.rs"],
       """\
**Кадры** (`protocol.rs`):
```rust
Op::HostApply { generation: u64, mode: String }   // "proxy" | "tunnel"
Op::HostRevert { generation: u64 }
Op::HostStatus
OpClass::Host
ReplyData::Host { mode: String, generation: Option<u64>, uplinks: Vec<String> }
```
- `Op::class`: `HostStatus` → `Status`; `HostApply`, `HostRevert` → `Host`.
- `Op::instance`: для трёх операций — `Some(HOST_INSTANCE)`.
- `actions.rs`: `ACTION_HOST = "io.github.cm.host"`. В файле политики polkit (`src/main.rs`) — ещё одно действие: значение `auth_admin_keep`, как у `io.github.cm.net`; сообщение — на тех же языках, что у соседних действий.

**Состояние**: файл `<base>/host.json`, владелец — контроллер, 0600, запись через временный файл и `rename`:
```rust
#[serde(deny_unknown_fields)]
pub struct HostState { pub uid: u32, pub mode: String, pub generation: u64, pub uplinks: Vec<Uplink> }
```

**`host_apply`**, проверки по порядку:
1. `mode` не `proxy` и не `tunnel` → `bad_argument`.
2. Повтор того же кадра (тот же `id`) → сохранённый ответ из журнала, как у остальных операций.
3. `host.json` существует → `conflict` (чужой uid или свой: сначала `host_revert`).
4. Только для `tunnel`: `net::observe` и `host::detect("/run")` → `host::preflight`. `LegacyCm`, `ForeignAutoRoute` → `conflict`; `Resolved` → `unsupported`.
5. Только для `tunnel`: `observe_uplinks`. Пустой список не ошибка: правила появятся, когда появится сеть (`I09.F3`).

Шаги транзакции (журнал и компенсация — как в `net_ops`):

| Шаг | Вперёд | Компенсация |
|---|---|---|
| `WriteHostState` | записать `host.json` | удалить файл |
| `HostCreate` (только `tunnel`) | `host_create_commands` | `host_destroy_commands`, ошибки отдельных команд не останавливают |
| `HostFirewall` (только `tunnel`) | `host_firewall_commands(Some)` | `host_firewall_commands(None)` |
| `RecordHost` | поколение под ключом `net:host` в `generations.json` владельца (ключ `host` занят поколением ядра) | убрать запись |

Ответ: `Host { mode, generation: Some(g), uplinks: <имена> }`.

**`host_revert`**:
1. нет `host.json` или в нём другой uid → `not_running`;
2. поколение не совпало → `generation_mismatch`;
3. ядро экземпляра `host` запущено → `conflict` (сначала `worker_stop`);
4. шаги: `host_firewall_commands(None)` → `host_destroy_commands` → удалить `host.json` → убрать запись `net:host`. Удаление идемпотентно: отсутствие правила или устройства не ошибка.

**`host_status`**: нет файла → `Host { mode: "off", generation: None, uplinks: [] }`. Файл чужого uid → тот же ответ `off`: чужое состояние не раскрывается.

**Ядро хоста** — обычные `worker_start` и `worker_stop` с `instance: "host"`. В `core_ops`, там, где для экземпляра определяется `TunnelNet`:
- экземпляр `host` и `host.json` принадлежит вызывающему, режим `tunnel` → `host_net(uid).as_tunnel()`;
- режим `proxy` → сети нет, ядро слушает арендованный порт (как сейчас у экземпляра без сети);
- экземпляр `host`, а `host.json` нет или он чужой → `conflict`. Имя `host` зарезервировано.

`net_apply` для экземпляра `host` → `bad_instance`.""",
       external=["I06.W2", "I08.N6"], covers=["I07.T03.a", "I07.T04.a"])

vertex("I07.H9", "H", "Дать команду `cm host` для смены режима хоста.", "M", "L2", ["I07.H6", "I07.H7"],
       ["src/net/cli.rs", "src/net/mod.rs", "src/main.rs", "src/i18n_table.rs"],
       """\
`src/main.rs`: `if args.first() == "host" { exit(cm::net::cli::dispatch(&args[1..])) }` рядом с `app` и `controller`.

```
cm host status
cm host set off|proxy|tunnel --config ФАЙЛ
```

`status`: `host_status` → одна строка `mode=<режим> generation=<N|-> uplinks=<имена через запятую|->`.

`set`:
1. Текущий режим — из `host_status`. `host::transition(текущий, новый)` даёт шаги; пустой список → код 0 без действий.
2. Шаги выполняются по порядку вызовами контроллера:
   - `StopWorker` → `worker_stop{instance: "host", generation: <из worker_status>}`; ответ `not_running` не ошибка;
   - `RevertNet` → `host_revert{generation}`;
   - `ApplyNet(mode)` → `instance_prepare{host}`; копия `ФАЙЛ` в `config/gen-<N>.json` владельца (0600); `host_apply{generation: N, mode}`;
   - `StartWorker` → `worker_start{instance: "host", generation: N}`.
3. `N` — поколение из `worker_status` плюс один; если ядра не было — 1.
4. Отказ шага после `ApplyNet` (например, `worker_start` → `invalid_config`): выполнить `host_revert{N}` и выйти с кодом 3. Режим остаётся `off`: при `tunnel` это означает, что правил CM нет и трафик хоста идёт как до команды.

`--config` обязателен для `proxy` и `tunnel`. Это готовый конфиг ядра без `tun`, `dns` и слушателей (те же требования, что у `attach_tun`); собирать его из хранилища — задача `I12.A11`.

Каталог `config/` владельца: `<base>/u<uid>/instances/host/config/`. `<base>` — `controller::owner::SYSTEM_BASE`; в `test_mode()` — переменная `CM_CONTROLLER_BASE`, если задана.

Коды выхода: 0 — готово; 2 — использование; 3 — отказ контроллера (код ответа печатается в `stderr` в квадратных скобках); 4 — контроллер недоступен.

Нынешнюю команду `cm vpn` и службу VPN хоста эта вершина не меняет.""",
       covers=["I07.T03.a"])

# ───────────────────────── D: политика туннеля и DNS ─────────────────────────
vertex("I08.N9", "D", "Читать политику туннеля из каталога владельца.", "S", "L1", [],
       ["src/net/policy.rs", "src/net/mod.rs", "src/controller/owner.rs", "src/controller/net_ops.rs"],
       """\
Файл `config/net.json` пишет владелец рядом со своими конфигами. Его нет → политика по умолчанию.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime { Pinned, Refcount }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode { Block, Failover }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TunnelPolicy {
    pub ipv6: Ipv6Policy,        // по умолчанию Block
    pub fake_ip: FakeIpPolicy,   // по умолчанию Forbidden
    pub lifetime: Lifetime,      // по умолчанию Pinned
    pub failure: FailureMode,    // по умолчанию Block
}
pub const MAX_POLICY_BYTES: usize = 1024;
pub fn parse(bytes: &[u8]) -> Result<TunnelPolicy, NetError>;   // любая ошибка разбора → NetError::Unsupported
```

`controller::owner::read_policy(owned) -> Result<TunnelPolicy, ControlError>`:
- файла нет → `TunnelPolicy::default()`;
- есть → `read_owned_file(owned, path, MAX_POLICY_BYTES)` (те же проверки владельца и ссылок, что у конфига), затем `parse`; ошибка разбора → `invalid_config`.

`net_ops::apply`: политика читается до первого шага. `ipv6 == Pass` → `unsupported`, ни одной команды не выполнено. Иначе в `net::create` передаётся `policy.ipv6` (сейчас там постоянное `Block`).""",
       covers=["I10.T01.a"])

vertex("I10.D1", "D", "Настроить DNS ядра по политике туннеля и сбрасывать кэши при смене поколения.", "M", "L2", ["I08.N9"],
       ["src/net/dns.rs", "src/core/mihomo/config.rs", "src/core/mihomo/lifecycle.rs", "src/core/adapter.rs", "src/controller/core_ops.rs"],
       """\
```rust
// src/net/dns.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsRuntime {
    pub resolver: String,              // tunnel.dns_addr
    pub enhanced_mode: &'static str,   // "redir-host" | "fake-ip"
    pub fake_ip_range: Option<String>, // "198.19.<index>.0/24"
    pub nameservers: Vec<String>,      // ["https://1.1.1.1/dns-query", "https://dns.google/dns-query"]
    pub bootstrap: Vec<String>,        // BOOTSTRAP_DNS
}
pub fn dns_runtime(policy: &TunnelPolicy, tunnel: &TunnelNet) -> Result<DnsRuntime, NetError>;
```
- `policy.ipv6 == Pass` → `Err(Unsupported)`;
- `fake_ip == Forbidden` → `redir-host`, диапазона нет; `Allowed` → `fake-ip`, диапазон `198.19.<tunnel.index>.0/24`.

`config.rs`: `pub fn attach_tun_dns(document, tunnel, dns: &DnsRuntime) -> Result<Vec<u8>, ConfigError>`. `attach_tun(document, tunnel)` остаётся и вызывает её с политикой по умолчанию. Отличия результата от нынешнего `attach_tun`:
- раздел `dns`: `enable: true`, `ipv6: false`, `enhanced-mode`, `nameserver`, `default-nameserver` — из `DnsRuntime`; при `fake-ip` добавляется `fake-ip-range`;
- всегда добавляется `"profile": {"store-fake-ip": false}`. Если `profile` во входном документе уже есть — `ConfigError::Forbidden`.

`CoreAdapter`: метод с пустой реализацией по умолчанию `fn set_dns_runtime(&mut self, _dns: DnsRuntime) {}`. `MihomoWorker` сохраняет значение и использует его там, где сейчас вызывает `attach_tun`.

`core_ops`: при запуске и перезагрузке ядра с сетью — `read_policy` → `dns_runtime` → `set_dns_runtime`. Ошибка `dns_runtime` → `unsupported`.

**Смена поколения.** В `MihomoWorker::reload`, после успешного `PUT /configs`: `POST /cache/fakeip/flush` и `POST /cache/dns/flush`. Ошибка сброса не делает перезагрузку неуспешной: новый конфиг уже действует.

Отдельного DNS-процесса нет ([ADR-DNS](ADR-DNS.md)).""",
       covers=["I10.T01.a", "I10.T03.a", "I10.T04.a"])

# ───────────────────────── F: отказы и восстановление ─────────────────────────
vertex("I09.F1", "F", "Сводить состояние при старте контроллера и при загрузке системы.", "L", "L2", ["I09.F0", "I07.H7"],
       ["src/controller/boot.rs", "src/controller/server.rs", "src/controller/mod.rs", "src/main.rs"],
       """\
```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BootReport { pub owners: u32, pub compensated: u32, pub cleared_nets: u32, pub repaired: u32, pub killed_cores: u32, pub orphans: u32 }
pub fn reconcile_all(base: &Path, workers: &Workers, nets: &Arc<NetCtx>, now: i64) -> Result<BootReport, ControlError>;
```

Вызывается в `serve` до приёма первого соединения и командой `cm controller reconcile --boot [--base DIR]` (вне `test_mode()` — только root и без `--base`). Команда печатает отчёт одной строкой JSON; код 0, при ошибке — 4.

Шаги по порядку:
1. **Владельцы.** Каталоги `<base>/u<число>`: настоящий каталог, не ссылка. `owners` — их число.
2. **Журналы.** Для каждого владельца — существующая сверка журнала (`Workers::reconcile`): незавершённые транзакции компенсируются. `compensated` — сумма.
3. **Ядра.** Процессы ядер после гибели контроллера остаются жить, но управлять ими уже некому. Для каждого экземпляра с `core.pid`: остановить группу процессов (существующая остановка по pid-файлу с проверкой владельца) и убрать поколение ядра. `killed_cores` — число остановленных. TUN при этом остаётся: трафик приложений блокируется, а не идёт напрямую.
4. **Сети.** `net::observe`. Для каждой записи сети (`net:<экземпляр>` в поколениях, номер — из аренды):
   - нет ни netns `cm-<i>`, ни `cmv<i>h` (после перезагрузки) → снять запись: аренда, каталог `resolv.conf`, поколение. `cleared_nets += 1`;
   - netns есть, но `audit` нашёл расхождение → `repair_commands` по каждому расхождению. `repaired += 1`. Сеть не пересоздаётся: в netns могут жить приложения.
5. **Сироты.** `Drift::OrphanVeth` и `Drift::OrphanRule` → `repair_commands`. `orphans` — их число.
6. **Хост.** `host.json` есть, а `cmtunh` нет → `host_firewall_commands(None)`, `host_destroy_commands`, удалить `host.json` и запись `net:host`. Устройство на месте → пересчитать внешние интерфейсы и выполнить `uplink_commands`.
7. **Таблица.** `net::firewall::replace_commands` по оставшимся арендам. Таблица `inet cm` с запретами `cmv*` загружается и при нуле туннелей.

Ошибка одного владельца не останавливает остальных; итог команды — ошибка, только если не удался шаг 7.

**Юнит** `cm-reconcile.service` в установке (`src/main.rs`, рядом с юнитами контроллера), включается при установке:
```
[Unit]
Description=CM network state reconcile
DefaultDependencies=no
After=local-fs.target
Before=network-pre.target
Wants=network-pre.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart={bin} controller reconcile --boot

[Install]
WantedBy=multi-user.target
```""",
       covers=["I09.T03.a"])

vertex("I09.F2", "F", "Решать, что делать при отказе ядра, сервера или сети.", "M", "L1", [],
       ["src/net/failure.rs", "src/net/mod.rs"],
       """\
Чистая функция. Перенос открытых соединений на другой узел не обещается (Q13).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { CoreDown, RemoteLost, Drift, ReloadRejected, UplinkLost }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { None, Block, RestartCore { delay_ms: u64 }, Failover, Repair, GiveUp }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureState { pub window_start_ms: i64, pub restarts: u8, pub switches: u8 }

pub const WINDOW_MS: i64 = 600_000;
pub const MAX_RESTARTS: u8 = 5;
pub const MAX_SWITCHES: u8 = 3;
pub const RESTART_DELAYS_MS: [u64; 5] = [0, 1_000, 5_000, 30_000, 120_000];

pub fn decide(failure: Failure, mode: FailureMode, state: &FailureState, now_ms: i64) -> (Action, FailureState);
```

Сначала окно: если `now_ms - state.window_start_ms >= WINDOW_MS`, счётчики обнуляются, окно начинается с `now_ms`.

| Отказ | Решение |
|---|---|
| `CoreDown` | `restarts < MAX_RESTARTS` → `RestartCore { delay_ms: RESTART_DELAYS_MS[restarts] }`, `restarts += 1`; иначе `GiveUp` |
| `RemoteLost` | `mode == Failover` и `switches < MAX_SWITCHES` → `Failover`, `switches += 1`; иначе `Block` |
| `Drift` | `Repair` |
| `ReloadRejected` | `None`: действует прежний конфиг |
| `UplinkLost` | `None`: сеть вернётся сама, блокировку держат маршруты |

`GiveUp` означает состояние `Failed`: трафик заблокирован до команды пользователя.""",
       covers=["I09.T04.a"])

vertex("I09.F3", "F", "Следить за ядрами, сетью и внешними интерфейсами и чинить расхождения.", "L", "L2", ["I09.F1", "I09.F2", "I08.N9"],
       ["src/controller/watch.rs", "src/controller/server.rs", "src/controller/core_ops.rs", "src/controller/protocol.rs",
        "src/controller/dispatch.rs", "src/controller/mod.rs"],
       """\
Поток контроллера. Один проход — функция, которую тест вызывает напрямую.

```rust
pub const WATCH_INTERVAL_MS: u64 = 2_000;
pub struct Watch { /* по экземпляру: TunnelState, FailureState, restart_at_ms, had_holders, empty_since_ms */ }
impl Watch {
    pub fn new() -> Self;
    pub fn tick(&mut self, base: &Path, workers: &Workers, nets: &Arc<NetCtx>, now_ms: i64);
    pub fn state(&self, uid: u32, instance: &str) -> Option<(TunnelState, u8)>;   // состояние и число перезапусков
}
```

Один проход `tick`, по порядку:
1. **Внешние интерфейсы.** `host.json` в режиме `tunnel`: `observe_uplinks`; набор изменился → `uplink_commands(old, new)`, затем перезаписать `host.json`.
2. **Ядра.** Для каждого запущенного экземпляра (нужен перебор в `Workers`, например `Workers::running() -> Vec<(u32, String)>`):
   - ядро живо и API отвечает → событие `CoreReady`;
   - ядро мертво → событие `CoreDown`, затем `failure::decide(CoreDown, policy.failure, …)`:
     - `RestartCore { delay_ms }` → запомнить `restart_at_ms = now_ms + delay_ms`. Сам перезапуск выполняет один из следующих проходов — первый, на котором `now_ms >= restart_at_ms`; на проходе, обнаружившем гибель, ядро не запускается. Запуск — с тем же поколением и тем же конфигом (`Workers::restart(owned)`: тот же путь, что `worker_start`, но без записи нового поколения);
     - `GiveUp` → состояние `Failed`, перезапусков больше нет до `worker_stop` и нового `worker_start`.
3. **Сеть.** `net::observe` → `audit` по арендам. Каждое расхождение — событие `DriftFound` и `repair_commands`. После успешной починки — `Repaired`.
4. **Срок жизни** — вершина `I12.A12`.

Состояние экземпляра ведётся автоматом `net::state::next`. Проход сериализуется с операциями клиента тем же способом, каким сериализуются сами операции; занятый экземпляр пропускается до следующего прохода.

Поток запускается в `serve` после `reconcile_all` и останавливается по тому же флагу, что цикл приёма соединений.

**Кадр статуса**:
```rust
Op::TunnelState { instance: String }                 // класс Status
ReplyData::Tunnel { state: String, restarts: u32, holders: u32 }
```
`state` — `stopped|starting|up|degraded|blocked|failed`. Экземпляр не запускался → `stopped`, нули. `holders` заполняет `I12.A12`; до неё — 0. `Watch` разделяется между потоком и обработчиком через `Workers` (`Workers::set_watch`, как `set_net`): в `Deps` новых полей нет.""",
       covers=["I09.T03.a", "I09.T04.a", "I07.T04.a", "I07.T05.a"])

# ───────────────────────── рёбра ─────────────────────────
edge("H01", "I07.H1", "I07.H3",
     "Номера и имена туннеля хоста постоянны и не пересекаются с туннелями приложений.",
     "`tests/audit_ab_host.rs`.",
     ["host_net(1000): tun \"cmtunh\", tun_addr \"198.18.64.1/30\", dns_addr \"198.18.64.2\", sink \"cmsink\", table 90, sink_table 91, rule_uplink 1900, rule_uplink_addr 1950, rule_main 1990, rule_tun 2000, rule_sink 2001, mtu 1400, owner_uid 1000",
      "для всех i в 0..64: tunnel_net(i).table != 90 и != 91; rule_priority < 1900; tun_addr != \"198.18.64.1/30\"",
      "tunnel_net(64, 0) == Err(BadIndex)",
      "BOOTSTRAP_DNS == [\"1.1.1.1\", \"8.8.8.8\"]; те же два адреса стоят в `default-nameserver` результата attach_tun"])
edge("H02", "I07.H1", "I07.H4",
     "Таблица DNS хоста отрисована дословно и заменяется одной командой.",
     "`tests/audit_ab_host.rs`; загрузка — `nft -c -f -` в `unshare -rn`.",
     ["render_host_table(&host_net(0)) совпадает с текстом из вершины I07.H4 побайтно",
      "host_firewall_commands(Some): одна команда Nft, args [\"-f\", \"-\"], stdin начинается с \"table ip cm_host\\ndelete table ip cm_host\\n\"",
      "host_firewall_commands(None): stdin ровно \"table ip cm_host\\ndelete table ip cm_host\\n\"",
      "`nft -c -f -` принимает оба текста (код 0)"], level="L2")
edge("H03", "I07.H1", "I07.H5",
     "Ядро хоста получает TUN хоста тем же путём, что ядро приложения.",
     "`tests/audit_ab_host.rs`.",
     ["host_net(1000).as_tunnel(): index 64, tun \"cmtunh\", tun_addr \"198.18.64.1/30\", dns_addr \"198.18.64.2\", mtu 1400, owner_uid 1000, netns \"\", veth_host \"\"",
      "attach_tun({\"mode\":\"direct\"}, &as_tunnel): tun.device \"cmtunh\", tun.inet4-address [\"198.18.64.1/30\"], tun.auto-route false",
      "attach_tun для любого туннеля: tun.auto-detect-interface == true"])
edge("H04", "I07.H2", "I07.H3",
     "Внешние интерфейсы определяются по маршрутам по умолчанию, свои интерфейсы CM в них не попадают.",
     "`tests/audit_ab_host.rs`, фикстуры `tests/fixtures/ab/routes-default.json`, `addrs.json`.",
     ["v4 [{up0, metric 100}, {wlan0, metric 600}], addrs {up0: [192.0.2.2], wlan0: [10.0.0.5]} → [Uplink{up0, [192.0.2.2]}, Uplink{wlan0, [10.0.0.5]}]",
      "v4 [{cmtunh, 0}, {lo, 0}, {up0, 100}] → только up0",
      "v4 пуст, v6 [{up0, 1024}] → [Uplink{up0, []}]",
      "две записи одного интерфейса с разной метрикой → один Uplink",
      "10 интерфейсов → первые 8 по (metric, dev)",
      "запись маршрута без \"metric\" → metric 0; \"не json\" → Err(CommandFailed)",
      "parse_addrs: адрес со scope \"link\" или \"host\" не попадает в список"])
edge("H05", "I07.H3", "I07.H7",
     "Команды туннеля хоста идут в проверенном порядке: приёмник раньше туннеля, удаление зеркально.",
     "`tests/audit_ab_host.rs`.",
     ["host_create_commands(host_net(1000), [Uplink{up0, [192.0.2.2]}]) — ровно 16 команд; первая \"link add cmsink type dummy\", последняя \"rule add priority 2000 lookup 90\"",
      "\"rule add priority 2001 lookup 91\" стоит раньше \"rule add priority 2000 lookup 90\"",
      "есть \"rule add priority 1900 oif up0 lookup main\", \"-6 rule add priority 1900 oif up0 lookup main\", \"rule add priority 1950 from 192.0.2.2 lookup main\"",
      "есть \"-6 rule add priority 2001 lookup 91\" и нет ни одной команды с \"-6\" и \"lookup 90\"",
      "\"tuntap add dev cmtunh mode tun user 1000\"",
      "два интерфейса с адресами [a], [b, c] → приоритеты oif 1900 и 1901, from 1950, 1951, 1952",
      "host_destroy_commands: первая \"rule del priority 2000 lookup 90\", последние две \"link del cmtunh\", \"link del cmsink\"; каждой команде `rule add` создания соответствует `rule del` с тем же хвостом",
      "uplink_commands(old == new) пусто; (old [up0], new [up1]) → сначала все del набора old, затем все add набора new",
      "в командах нет слова \"blackhole\""])
edge("H06", "I07.H4", "I07.H7",
     "Перенаправление DNS включается и снимается вместе с туннелем хоста.",
     "`tests/audit_ab_host_ops.rs` (подменные `NetExec` и `NetQuery`).",
     ["host_apply(tunnel): в записи RecordingExec команда Nft со stdin, содержащим \"table ip cm_host {\", идёт после последней команды `ip rule add`",
      "host_revert: первая команда — Nft со stdin ровно \"table ip cm_host\\ndelete table ip cm_host\\n\"",
      "host_apply(proxy): ни одной команды Ip и Nft"])
edge("H07", "I07.H5", "I07.H7",
     "Ядро экземпляра `host` запускается на TUN хоста только у владельца туннеля хоста.",
     "`tests/audit_ab_host_ops.rs`.",
     ["после host_apply(tunnel) uid A: worker_start{host} → фабрика получила TunnelNet с tun \"cmtunh\" и index 64",
      "после host_apply(proxy): фабрика получила сеть None",
      "worker_start{host} без host.json → conflict; от uid B при host.json uid A → conflict; ядро не создано",
      "net_apply{instance: \"host\"} → bad_instance"])
edge("H08", "I07.H6", "I07.H7",
     "Туннель хоста не включается поверх действующего VPN хоста и там, где DNS не проверен.",
     "`tests/audit_ab_host.rs`, `tests/audit_ab_host_ops.rs`.",
     ["preflight(Tunnel, [\"lo\", \"cm-vpn\"], [], чисто) == Err(LegacyCm)",
      "preflight(Tunnel, [\"lo\"], [правило table \"2022\"], чисто) == Err(ForeignAutoRoute)",
      "preflight(Tunnel, [\"lo\"], [], resolved: true) == Err(Resolved)",
      "preflight(Proxy, [\"cm-vpn\"], [правило 2022], resolved: true) == Ok",
      "network_manager и firewalld == true при прочих чистых → Ok",
      "detect(каталог с NetworkManager/ и systemd/resolve/stub-resolv.conf) → network_manager true, resolved true, firewalld false",
      "host_apply(tunnel) при link cm-vpn в FixedQuery → conflict, RecordingExec пуст, host.json нет",
      "host_apply(tunnel) при resolved → unsupported, RecordingExec пуст"])
edge("H09", "I09.F0", "I07.H7",
     "Контроллер видит фактические правила, маршруты и интерфейсы через один источник.",
     "`tests/audit_ab_boot.rs`: `FixedQuery`; настоящий `SystemQuery` — в `unshare -rn`.",
     ["observe(FixedQuery, [100]) задаёт вопросы в порядке: \"-j rule show\", \"-j route show table 100\", \"-j link show\", \"netns list\"",
      "нет ответа на \"-j route show table 100\" → routes[100] пуст, ошибки нет",
      "\"netns list\" с текстом \"cm-0 (id: 1)\\ncm-3\\n\" → netns [\"cm-0\", \"cm-3\"]",
      "observe_uplinks без ответа на вопрос с \"-6\" → список по v4",
      "SystemQuery в пустом netns: observe(…, []) → links [\"lo\"], rules с приоритетами 0, 32766, 32767",
      "вывод длиннее 1 МиБ → Err(CommandFailed)"], level="L2")
edge("H10", "I07.H6", "I07.H9",
     "Шесть переходов между режимами хоста раскладываются в шаги без пропусков.",
     "`tests/audit_ab_host.rs`.",
     ["transition(Off, Proxy) == [ApplyNet(Proxy), StartWorker]; (Off, Tunnel) == [ApplyNet(Tunnel), StartWorker]",
      "transition(Proxy, Off) == transition(Tunnel, Off) == [StopWorker, RevertNet]",
      "transition(Proxy, Tunnel) == [StopWorker, RevertNet, ApplyNet(Tunnel), StartWorker]; (Tunnel, Proxy) — то же с Proxy",
      "transition(x, x) пусто для трёх режимов",
      "в любом переходе RevertNet стоит после StopWorker, StartWorker — после ApplyNet"])
edge("H11", "I07.H7", "I07.H9",
     "Операции хоста проходят тот же порядок проверок, что остальные, и ведут состояние в одном файле.",
     "`tests/audit_ab_host_ops.rs`.",
     ["Authorizer получает \"io.github.cm.host\" для host_apply и host_revert; при отказе → denied, RecordingExec пуст",
      "host_status использует действие статуса и отвечает Host{mode \"off\", generation null, uplinks []}, когда файла нет",
      "host_apply{mode \"vpn\"} → bad_argument",
      "host_apply(tunnel, gen 1) → ok Host{mode \"tunnel\", generation 1, uplinks [\"up0\"]}; host.json: uid вызывающего, mode, generation, uplinks с адресами; права 0600",
      "тот же кадр повторно (тот же id) → тот же ответ, RecordingExec не вырос",
      "второй host_apply (другой id) → conflict",
      "host_status от uid B при host.json uid A → mode \"off\"",
      "host_revert{gen 2} при записи gen 1 → generation_mismatch; от uid B → not_running",
      "host_revert при работающем ядре host → conflict; после worker_stop → ok, host.json нет, записи `net:host` в поколениях нет",
      "отказ команды на шаге HostCreate (подменный NetExec) → failed; выполнены команды host_destroy_commands; host.json нет",
      "кадр host_apply с лишним полем \"uid\" → bad_frame"])
edge("H12", "I07.H9", "AB.Z",
     "`cm host set` выполняет переход целиком и откатывает его при отказе.",
     "`tests/ab_stand.py`, раздел `host`.",
     ["`cm host status` до команд: \"mode=off generation=- uplinks=-\", код 0",
      "`cm host set tunnel --config F` → код 0; status: \"mode=tunnel generation=1 uplinks=up0\"",
      "`cm host set tunnel --config F` повторно → код 0, поколение прежнее",
      "`cm host set off` → код 0; нет cmtunh, cmsink, правил 1900–2001, таблицы ip cm_host",
      "`cm host set tunnel --config <негодный конфиг>` → код 3, в stderr \"[invalid_config]\"; после этого status: mode=off; правил 1900–2001 нет",
      "`cm host set proxy` без --config → код 2",
      "контроллер не запущен → код 4"], level="L2")

edge("D01", "I08.N9", "I10.D1",
     "Политика туннеля читается с теми же проверками, что конфиг, и имеет безопасные значения по умолчанию.",
     "`tests/audit_ab_policy.rs`.",
     ["parse(b\"{}\") == {ipv6: Block, fake_ip: Forbidden, lifetime: Pinned, failure: Block}",
      "parse({\"lifetime\":\"refcount\",\"failure\":\"failover\",\"fake_ip\":\"allowed\"}) — три поля изменены, ipv6 Block",
      "parse({\"dns\":\"8.8.8.8\"}) → Err(Unsupported); parse(b\"[]\") → Err; parse({\"ipv6\":\"maybe\"}) → Err",
      "read_policy: файла нет → значения по умолчанию",
      "net.json — символическая ссылка → отказ с тем же кодом, что у ссылки вместо конфига",
      "net.json длиннее 1024 байт → too_large",
      "net.json с лишним полем → invalid_config",
      "net_apply при {\"ipv6\":\"pass\"} → unsupported, RecordingExec пуст, аренды туннеля нет"])
edge("D02", "I08.N9", "I09.F3",
     "Сторож берёт режим отказа из политики туннеля.",
     "`tests/audit_ab_watch.rs`.",
     ["net.json {\"failure\":\"block\"}: шесть гибелей ядра подряд в одном окне → после шестой состояние failed, restarts 5",
      "net.json отсутствует → поведение то же, что при block"])
edge("D03", "I10.D1", "AB.Z",
     "Конфиг DNS ядра следует политике; кэши не переживают смену поколения.",
     "`tests/audit_ab_policy.rs`; с ядром — `tests/ab_stand.py`, раздел `dns`.",
     ["dns_runtime(по умолчанию, tunnel_net(3)) == {resolver \"198.18.3.2\", enhanced_mode \"redir-host\", fake_ip_range None, nameservers [\"https://1.1.1.1/dns-query\", \"https://dns.google/dns-query\"], bootstrap [\"1.1.1.1\", \"8.8.8.8\"]}",
      "fake_ip Allowed, tunnel_net(3) → \"fake-ip\", Some(\"198.19.3.0/24\")",
      "ipv6 Pass → Err(Unsupported)",
      "attach_tun_dns: dns.enhanced-mode и dns.fake-ip-range по политике; profile.store-fake-ip == false; документ с ключом profile → Err(Forbidden)",
      "attach_tun(document, tunnel) == attach_tun_dns(document, tunnel, политика по умолчанию)",
      "стенд, fake_ip allowed: первое имя получает адрес из 198.19.0.0/24; после worker_reload новое имя получает тот же первый адрес (таблица сброшена)"], level="L2")
edge("D04", "I10.D1", "AB.Z",
     "Резолвер туннеля отвечает на все типы запросов и не выпускает DNS мимо туннеля.",
     "`tests/ab_stand.py`, раздел `dns`: подставной сервер по образцу `tools/dns_probe_lab_srv.py`; ядро настраивается на него напрямую (без контроллера), потому что продуктовый конфиг ходит по DoH.",
     ["redir-host: A → адрес сервера; AAAA → NOERROR без записей; TXT, MX, SRV, HTTPS, SVCB → запись сервера",
      "redir-host: несуществующее имя → NXDOMAIN",
      "усечённый ответ по UDP → клиент получает полную запись (ядро повторило по TCP)",
      "запрос по TCP → ответ",
      "запрос приложения на 9.9.9.9:53 → тот же ответ (перехват); на интерфейсе «интернета» нет пакетов на 9.9.9.9",
      "fake-ip: A → адрес из диапазона; несуществующее имя → адрес из диапазона (а не NXDOMAIN); HTTPS → NOERROR без записей",
      "UDP не на порт 53: эхо-сервер отвечает через туннель",
      "соединение по IPv6-адресу из сети приложения → немедленный отказ (curl rc 7), пакетов IPv6 на «интернете» нет",
      "POST /cache/fakeip/flush и /cache/dns/flush → 204"], level="L2")

edge("F01", "I09.F0", "I09.F1",
     "Сверка при старте опирается на фактическое состояние, а не на записи.",
     "`tests/audit_ab_boot.rs` (подменные `NetExec` и `NetQuery`, каталог `--base` во временном каталоге).",
     ["записи есть (аренда 0, поколение net:app), а FixedQuery показывает только lo → cleared_nets 1; аренды, поколения и каталога resolv нет; команд `ip … del` для cmv0h не было",
      "netns cm-0 есть, правила priority 1000 нет → repaired 1; выполнена `ip rule add iif cmv0h lookup 100 priority 1000`; netns не удалялся",
      "veth cmv5h без аренды → orphans 1; выполнена `ip link del cmv5h`",
      "чужое правило priority 1005 без iif cmv5h → не тронуто",
      "при нуле туннелей последняя команда — Nft с таблицей inet cm, в которой есть `iifname \"cmv*\" counter drop`",
      "каталог `<base>/u1000` — символическая ссылка → пропущен, owners 0",
      "ошибка в журнале одного владельца не мешает сверке второго"])
edge("F02", "I07.H7", "I09.F1",
     "После перезагрузки запись о туннеле хоста снимается, после перезапуска контроллера — сохраняется.",
     "`tests/audit_ab_boot.rs`.",
     ["host.json (tunnel), cmtunh в FixedQuery нет → host.json удалён, запись `net:host` убрана, выполнена Nft с удалением таблицы cm_host",
      "host.json (tunnel), cmtunh есть, внешние интерфейсы те же → команд ip rule нет",
      "host.json (tunnel), default перешёл с up0 на up1 → выполнены del правил up0 и add правил up1; host.json обновлён"])
edge("F03", "I09.F1", "I09.F3",
     "После гибели контроллера сиротские ядра останавливаются, сеть остаётся и блокирует трафик.",
     "`tests/ab_stand.py`, раздел `faults`; команда `cm controller reconcile --boot` — `tests/audit_ab_boot.rs`.",
     ["стенд: `kill -9` контроллера при работающем туннеле → новый контроллер при старте: процесса ядра нет, worker_status running false, cmtun0 и netns cm-0 на месте",
      "тот же момент: запрос из приложения — тайм-аут; на «интернете» нет пакетов с адреса хоста от имени приложения",
      "после worker_start того же поколения запрос приложения снова 200; приложение не перезапускалось",
      "`cm controller reconcile --boot --base DIR` печатает одну строку JSON с шестью полями отчёта, код 0",
      "без --base в тестовом режиме → код 2; не root вне тестового режима → код 4",
      "в тексте установки есть юнит cm-reconcile.service с `Before=network-pre.target` и `ExecStart=… controller reconcile --boot`"], level="L2")
edge("F04", "I09.F2", "I09.F3",
     "Перезапуски ядра ограничены окном и растущей задержкой.",
     "`tests/audit_ab_failure.rs`.",
     ["пять CoreDown подряд (одно окно) → задержки 0, 1000, 5000, 30000, 120000; шестой → GiveUp",
      "CoreDown через 600000 мс после начала окна → задержка 0, restarts 1, окно начато заново",
      "RemoteLost, Block → Block; RemoteLost, Failover ×3 → Failover; четвёртый → Block",
      "Drift → Repair; ReloadRejected → None; UplinkLost → None; состояние не меняется",
      "свойство (10 000 случайных последовательностей отказов и времён): restarts <= 5 и switches <= 3 в любом окне"])
edge("F05", "I09.F3", "AB.Z",
     "Сторож возвращает туннель в работу без участия клиента и не выпускает трафик напрямую.",
     "`tests/audit_ab_watch.rs` (подменные ядро, `NetExec`, `NetQuery`; время задаёт тест); матрица отказов — `tests/ab_stand.py`, раздел `faults`.",
     ["tick при живом ядре: состояние up, restarts 0",
      "ядро мертво: tick → blocked; следующий tick с тем же временем → ядро запущено заново с прежним поколением, restarts 1",
      "вторая гибель: перезапуск не раньше чем через 1000 мс",
      "шестая гибель в окне → failed; дальнейшие tick ядро не запускают; worker_stop и worker_start возвращают up",
      "FixedQuery без правила priority 1000 → выполнена команда его добавления; состояние прошло blocked → starting",
      "tunnel_state незапущенного экземпляра → Tunnel{state \"stopped\", restarts 0, holders 0}; чужой uid получает тот же ответ",
      "host.json tunnel, FixedQuery сменил up0 на up1 → выполнены uplink_commands, host.json обновлён; повторный tick команд не даёт",
      "стенд, матрица: `kill -9` ядра → запрос приложения сначала тайм-аут, не позже чем через 5 с снова 200, приложение то же",
      "стенд: `ip link del cmtun0` → запрос тайм-аут (blackhole), прямых пакетов нет",
      "стенд: `ip rule del priority 1000` → не позже чем через 5 с правило восстановлено",
      "стенд: worker_reload с негодным конфигом → invalid_config; запрос приложения 200 (прежний конфиг)",
      "стенд: внешний интерфейс выключен на 3 с и включён → запрос приложения снова 200 без команд клиента",
      "стенд: `kill -9` клиента `cm app run` → приложение живо, запрос 200",
      "во всех строках матрицы: на «интернете» 0 пакетов с адреса 10.213.0.2"], level="L2")
