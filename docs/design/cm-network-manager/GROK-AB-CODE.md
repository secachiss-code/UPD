# Роль 1, Grok 4.7: Пакеты A и B: туннель хоста, отказы, DNS, приложения, упаковки, регион, приватное окружение, качество — весь код за один проход

Дата: 2026-10-10. Координатор: Claude. Граф и контракты: [AB-DAG.md](AB-DAG.md). Основания: [ADR-HOST-FLOW.md](ADR-HOST-FLOW.md), [ADR-DNS.md](ADR-DNS.md), [ADR-XRAY-DATAPATH.md](ADR-XRAY-DATAPATH.md), [ADR-PACKET-FLOW.md](ADR-PACKET-FLOW.md), [ADR-CONTROLLER.md](ADR-CONTROLLER.md), [QUESTIONS.md](QUESTIONS.md).

## Задача

Реализовать все вершины ниже в порядке зависимостей. Исполнитель — Grok 4.7.

Порядок работы и сдачи — волнами, каждая волна собирается и проходит gate отдельно:
1. **Чистые модули** (не зависят от остального, могут идти первыми): I07.H1, I07.H2, I07.H3, I07.H4, I07.H6, I08.N9, I09.F2, I11.A8 (без кадра), I12.A10, I13.P1, I13.P2, I14.R4, I15.E1, I16.Q3, I16.Q4.
2. **Наблюдение и хост:** I09.F0, I07.H5, I07.H7, I07.H9.
3. **Политика, DNS, восстановление:** I10.D1, I09.F1, I09.F3.
4. **Сессии и запуск:** I12.A13, I15.E2, I14.R6, I11.A8 (кадр `app_list`), I12.A12.
5. **Команды:** I12.A11, I11.A9, I14.R5, I14.R7.

Пакет A — код P0: он решает, куда идёт трафик. Если волна 2 или 3 упирается в противоречие спецификации, остановись на ней, сдай сделанное и опиши противоречие. Обходной путь в коде сети, контроллера и сброса привилегий не придумывай.

Что уже есть и не переписывается: контроллер и его протокол (`src/controller/`), сеть приложения (`src/net/`), запуск приложений (`src/app/`), регион и качество (`src/env/`, `src/quality/`). Вершины добавляют к ним файлы и точечные правки.

Тесты пишет роль 2 по рёбрам. Поэтому точные значения из рёбер — часть спецификации, а не пожелание.

## Правила

1. **Перед сдачей — полный gate, EXIT=0:**
   `CARGO_TARGET_DIR=$HOME/.cache/cm-grok-target CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo CM_TEST_XRAY=$HOME/.cache/cm-cores/xray/xray sh tests/check_audit.sh`.
2. **Переформатирование существующих файлов запрещено.**
   - `cargo fmt` запрещён всегда.
   - `rustfmt` запрещён для `src/main.rs`, `src/lib.rs`, `src/i18n_table.rs`, `src/core/adapter.rs`, `src/core/mod.rs` и любого файла, которого нет в проверке формата в `tests/check_audit.sh`. `rustfmt src/main.rs` форматирует ещё `tui.rs` и `tui/*`, `rustfmt src/lib.rs` — весь крейт.
   - Файлы, которые gate уже проверяет на формат (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`, `src/core/mihomo/*.rs`), обязаны проходить `rustfmt --edition 2024 --check` и после правки. Форматировать — только тот файл, который правил.
   - Все новые файлы лежат в этих же каталогах и попадают в проверку сами. Список проверки в `tests/check_audit.sh` не менять.
   - Проверка перед сдачей: `git diff --stat` по файлам из первого пункта — только смысловые строки.
3. Сборка только с `--offline --locked`. Новых зависимостей нет.
4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога `--base` теста. Команды `ip`, `nft` и `sysctl` выполняются только внутри `unshare`; на хосте — никогда. Настоящий `pkcheck` не вызывается.
5. Каждый `unsafe` — с `// SAFETY:`. В `pre_exec` — только async-signal-safe вызовы.
6. **Тесты пишет роль 2 (Composer 2.5).** От тебя — код, который выполняет контракты рёбер (точные значения, коды, порядок проверок и команд), и зелёный существующий gate. Новых тестовых файлов не создавай.
7. **Разрешённые правки существующих тестов** — только чтобы они собирались и отражали намеренно изменённый контракт; ничего сверх списка:
   - `tests/pack_support/mod.rs`: новое поле `query` в конструкторе `NetCtx` (подменный `FixedQuery`);
   - `tests/audit_i06_protocol.rs`: новые варианты `Op` в `match` и поле `session: None` в литерале `AppLaunch`;
   - `tests/audit_pack_net.rs` (и `tests/pack_stand.py`, если он это проверяет): ожидание `auto-detect-interface` — `true`;
   - тест, считающий действия в файле политики polkit: на одно действие больше (`io.github.cm.host`).
   Любая другая правка теста — отказ при ревью. Если существующий тест падает по иной причине, остановись и опиши её.
8. **Никаких данных запроса в ответах и ошибках.** Ответ — код из фиксированного набора и `ReplyData`. Пути, argv, env, байты конфига и тексты ошибок ОС не попадают ни в ответ, ни в `Debug`, ни в вывод процесса.
9. **uid — только из сокета.** Место, где uid, путь или каталог владельца берётся из тела запроса, — отказ при ревью.
10. **Сеть — блокировка по построению.** Порядок команд из `I07.H3`, текст таблицы из `I07.H4`, порядок команд и таблица прошлого пакета (`net::commands`, `net::firewall`) не меняются: они проверены лабораторией.
11. Новые поля в `Deps` не добавлять: общее состояние передаётся через `Workers` (как `set_net`).
12. Новые строки интерфейса — через `t!` и `src/i18n_table.rs`; токены в квадратных скобках (`[in_use]`) не переводятся.
13. Коммит — только по поручению пользователя. Сдача — отчёт.

## Вершины по порядку

### I07.H1 — Вывести имена, адреса и номера таблиц туннеля хоста.

Файлы: `src/net/host_plan.rs`, `src/net/mod.rs`. Зависит от: —.

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

`net::tunnel_net(64, …)` по-прежнему отвечает `BadIndex`: номер 64 приложению не выдаётся.

Контракты, которые проверит роль 2:
- **H01 → I07.H3:** Номера и имена туннеля хоста постоянны и не пересекаются с туннелями приложений.
  - host_net(1000): tun "cmtunh", tun_addr "198.18.64.1/30", dns_addr "198.18.64.2", sink "cmsink", table 90, sink_table 91, rule_uplink 1900, rule_uplink_addr 1950, rule_main 1990, rule_tun 2000, rule_sink 2001, mtu 1400, owner_uid 1000
  - для всех i в 0..64: tunnel_net(i).table != 90 и != 91; rule_priority < 1900; tun_addr != "198.18.64.1/30"
  - tunnel_net(64, 0) == Err(BadIndex)
  - BOOTSTRAP_DNS == ["1.1.1.1", "8.8.8.8"]; те же два адреса стоят в `default-nameserver` результата attach_tun
- **H02 → I07.H4:** Таблица DNS хоста отрисована дословно и заменяется одной командой.
  - render_host_table(&host_net(0)) совпадает с текстом из вершины I07.H4 побайтно
  - host_firewall_commands(Some): одна команда Nft, args ["-f", "-"], stdin начинается с "table ip cm_host\ndelete table ip cm_host\n"
  - host_firewall_commands(None): stdin ровно "table ip cm_host\ndelete table ip cm_host\n"
  - `nft -c -f -` принимает оба текста (код 0)
- **H03 → I07.H5:** Ядро хоста получает TUN хоста тем же путём, что ядро приложения.
  - host_net(1000).as_tunnel(): index 64, tun "cmtunh", tun_addr "198.18.64.1/30", dns_addr "198.18.64.2", mtu 1400, owner_uid 1000, netns "", veth_host ""
  - attach_tun({"mode":"direct"}, &as_tunnel): tun.device "cmtunh", tun.inet4-address ["198.18.64.1/30"], tun.auto-route false
  - attach_tun для любого туннеля: tun.auto-detect-interface == true

### I07.H2 — Определить внешние интерфейсы и их адреса по выводу `ip -j`.

Файлы: `src/net/uplink.rs`, `src/net/mod.rs`. Зависит от: —.

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

Разбор: `metric` отсутствует → 0; запись без `dev` пропускается; не JSON или не массив → `NetError::CommandFailed`.

Контракты, которые проверит роль 2:
- **H04 → I07.H3:** Внешние интерфейсы определяются по маршрутам по умолчанию, свои интерфейсы CM в них не попадают.
  - v4 [{up0, metric 100}, {wlan0, metric 600}], addrs {up0: [192.0.2.2], wlan0: [10.0.0.5]} → [Uplink{up0, [192.0.2.2]}, Uplink{wlan0, [10.0.0.5]}]
  - v4 [{cmtunh, 0}, {lo, 0}, {up0, 100}] → только up0
  - v4 пуст, v6 [{up0, 1024}] → [Uplink{up0, []}]
  - две записи одного интерфейса с разной метрикой → один Uplink
  - 10 интерфейсов → первые 8 по (metric, dev)
  - запись маршрута без "metric" → metric 0; "не json" → Err(CommandFailed)
  - parse_addrs: адрес со scope "link" или "host" не попадает в список

### I07.H3 — Составить команды создания, удаления и перестройки туннеля хоста.

Файлы: `src/net/host_commands.rs`, `src/net/mod.rs`. Зависит от: I07.H1, I07.H2.

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

Почему приёмник — `dummy`, а не `blackhole`: с `blackhole` проверка обратного пути отбрасывает ответы ядрам туннелей приложений (лаборатория, шаг 9).

Контракты, которые проверит роль 2:
- **H05 → I07.H7:** Команды туннеля хоста идут в проверенном порядке: приёмник раньше туннеля, удаление зеркально.
  - host_create_commands(host_net(1000), [Uplink{up0, [192.0.2.2]}]) — ровно 16 команд; первая "link add cmsink type dummy", последняя "rule add priority 2000 lookup 90"
  - "rule add priority 2001 lookup 91" стоит раньше "rule add priority 2000 lookup 90"
  - есть "rule add priority 1900 oif up0 lookup main", "-6 rule add priority 1900 oif up0 lookup main", "rule add priority 1950 from 192.0.2.2 lookup main"
  - есть "-6 rule add priority 2001 lookup 91" и нет ни одной команды с "-6" и "lookup 90"
  - "tuntap add dev cmtunh mode tun user 1000"
  - два интерфейса с адресами [a], [b, c] → приоритеты oif 1900 и 1901, from 1950, 1951, 1952
  - host_destroy_commands: первая "rule del priority 2000 lookup 90", последние две "link del cmtunh", "link del cmsink"; каждой команде `rule add` создания соответствует `rule del` с тем же хвостом
  - uplink_commands(old == new) пусто; (old [up0], new [up1]) → сначала все del набора old, затем все add набора new
  - в командах нет слова "blackhole"

### I07.H4 — Отрисовать таблицу nftables, которая направляет DNS хоста в туннель.

Файлы: `src/net/host_firewall.rs`, `src/net/mod.rs`. Зависит от: I07.H1.

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
- `Some(host)` → `"table ip cm_host\ndelete table ip cm_host\n"` + текст таблицы;
- `None` → `"table ip cm_host\ndelete table ip cm_host\n"`.

Таблицу `inet cm` (`net::firewall`) эта вершина не трогает.

Контракты, которые проверит роль 2:
- **H06 → I07.H7:** Перенаправление DNS включается и снимается вместе с туннелем хоста.
  - host_apply(tunnel): в записи RecordingExec команда Nft со stdin, содержащим "table ip cm_host {", идёт после последней команды `ip rule add`
  - host_revert: первая команда — Nft со stdin ровно "table ip cm_host\ndelete table ip cm_host\n"
  - host_apply(proxy): ни одной команды Ip и Nft

### I07.H5 — Привязать исходящие сокеты ядра к внешнему интерфейсу.

Файлы: `src/core/mihomo/config.rs`. Зависит от: I07.H1.

Одна смысловая правка в `attach_tun`: `"auto-detect-interface": true` (было `false`).

Ядро само следит за маршрутом по умолчанию в `main` и привязывает исходящие сокеты к его интерфейсу. Привилегии для этого не нужны. Без привязки ядро хоста отвергает собственный трафик (`reject loopback connection`), а ядро приложения выходило бы через туннель хоста.

Для ядра хоста новой функции нет: `attach_tun(document, &host.as_tunnel())` даёт `device: "cmtunh"` и `inet4-address: ["198.18.64.1/30"]`.

Если после правки падает существующий тест, где вышестоящий сервер стоит на `127.0.0.1`, остановись и опиши это в отчёте: обходить привязку нельзя.

Контракты, которые проверит роль 2:
- **H07 → I07.H7:** Ядро экземпляра `host` запускается на TUN хоста только у владельца туннеля хоста.
  - после host_apply(tunnel) uid A: worker_start{host} → фабрика получила TunnelNet с tun "cmtunh" и index 64
  - после host_apply(proxy): фабрика получила сеть None
  - worker_start{host} без host.json → conflict; от uid B при host.json uid A → conflict; ядро не создано
  - net_apply{instance: "host"} → bad_instance

### I07.H6 — Спланировать переход между режимами хоста и проверить, что переход возможен.

Файлы: `src/net/host.rs`, `src/net/mod.rs`. Зависит от: —.

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

Для `Off` и `Proxy` — всегда `Ok(())`. `network_manager` и `firewalld` переход не блокируют: они только показываются в статусе.

Контракты, которые проверит роль 2:
- **H08 → I07.H7:** Туннель хоста не включается поверх действующего VPN хоста и там, где DNS не проверен.
  - preflight(Tunnel, ["lo", "cm-vpn"], [], чисто) == Err(LegacyCm)
  - preflight(Tunnel, ["lo"], [правило table "2022"], чисто) == Err(ForeignAutoRoute)
  - preflight(Tunnel, ["lo"], [], resolved: true) == Err(Resolved)
  - preflight(Proxy, ["cm-vpn"], [правило 2022], resolved: true) == Ok
  - network_manager и firewalld == true при прочих чистых → Ok
  - detect(каталог с NetworkManager/ и systemd/resolve/stub-resolv.conf) → network_manager true, resolved true, firewalld false
  - host_apply(tunnel) при link cm-vpn в FixedQuery → conflict, RecordingExec пуст, host.json нет
  - host_apply(tunnel) при resolved → unsupported, RecordingExec пуст
- **H10 → I07.H9:** Шесть переходов между режимами хоста раскладываются в шаги без пропусков.
  - transition(Off, Proxy) == [ApplyNet(Proxy), StartWorker]; (Off, Tunnel) == [ApplyNet(Tunnel), StartWorker]
  - transition(Proxy, Off) == transition(Tunnel, Off) == [StopWorker, RevertNet]
  - transition(Proxy, Tunnel) == [StopWorker, RevertNet, ApplyNet(Tunnel), StartWorker]; (Tunnel, Proxy) — то же с Proxy
  - transition(x, x) пусто для трёх режимов
  - в любом переходе RevertNet стоит после StopWorker, StartWorker — после ApplyNet

### I09.F0 — Снять фактическое состояние сети командами `ip -j`.

Файлы: `src/net/query.rs`, `src/net/mod.rs`, `src/controller/net_ops.rs`, `src/controller/server.rs`. Зависит от: —.

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

`NetCtx` получает поле `pub query: Arc<Mutex<dyn NetQuery + Send>>`; в `server.rs` — `SystemQuery`.

Контракты, которые проверит роль 2:
- **H09 → I07.H7:** Контроллер видит фактические правила, маршруты и интерфейсы через один источник.
  - observe(FixedQuery, [100]) задаёт вопросы в порядке: "-j rule show", "-j route show table 100", "-j link show", "netns list"
  - нет ответа на "-j route show table 100" → routes[100] пуст, ошибки нет
  - "netns list" с текстом "cm-0 (id: 1)\ncm-3\n" → netns ["cm-0", "cm-3"]
  - observe_uplinks без ответа на вопрос с "-6" → список по v4
  - SystemQuery в пустом netns: observe(…, []) → links ["lo"], rules с приоритетами 0, 32766, 32767
  - вывод длиннее 1 МиБ → Err(CommandFailed)
- **F01 → I09.F1:** Сверка при старте опирается на фактическое состояние, а не на записи.
  - записи есть (аренда 0, поколение net:app), а FixedQuery показывает только lo → cleared_nets 1; аренды, поколения и каталога resolv нет; команд `ip … del` для cmv0h не было
  - netns cm-0 есть, правила priority 1000 нет → repaired 1; выполнена `ip rule add iif cmv0h lookup 100 priority 1000`; netns не удалялся
  - veth cmv5h без аренды → orphans 1; выполнена `ip link del cmv5h`
  - чужое правило priority 1005 без iif cmv5h → не тронуто
  - при нуле туннелей последняя команда — Nft с таблицей inet cm, в которой есть `iifname "cmv*" counter drop`
  - каталог `<base>/u1000` — символическая ссылка → пропущен, owners 0
  - ошибка в журнале одного владельца не мешает сверке второго

### I07.H7 — Создавать и снимать туннель хоста операциями контроллера.

Файлы: `src/controller/host_ops.rs`, `src/controller/protocol.rs`, `src/controller/actions.rs`, `src/controller/dispatch.rs`, `src/controller/core_ops.rs`, `src/controller/mod.rs`, `src/main.rs`. Зависит от: I07.H3, I07.H4, I07.H5, I07.H6, I09.F0.

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

`net_apply` для экземпляра `host` → `bad_instance`.

Контракты, которые проверит роль 2:
- **H11 → I07.H9:** Операции хоста проходят тот же порядок проверок, что остальные, и ведут состояние в одном файле.
  - Authorizer получает "io.github.cm.host" для host_apply и host_revert; при отказе → denied, RecordingExec пуст
  - host_status использует действие статуса и отвечает Host{mode "off", generation null, uplinks []}, когда файла нет
  - host_apply{mode "vpn"} → bad_argument
  - host_apply(tunnel, gen 1) → ok Host{mode "tunnel", generation 1, uplinks ["up0"]}; host.json: uid вызывающего, mode, generation, uplinks с адресами; права 0600
  - тот же кадр повторно (тот же id) → тот же ответ, RecordingExec не вырос
  - второй host_apply (другой id) → conflict
  - host_status от uid B при host.json uid A → mode "off"
  - host_revert{gen 2} при записи gen 1 → generation_mismatch; от uid B → not_running
  - host_revert при работающем ядре host → conflict; после worker_stop → ok, host.json нет, записи `net:host` в поколениях нет
  - отказ команды на шаге HostCreate (подменный NetExec) → failed; выполнены команды host_destroy_commands; host.json нет
  - кадр host_apply с лишним полем "uid" → bad_frame
- **F02 → I09.F1:** После перезагрузки запись о туннеле хоста снимается, после перезапуска контроллера — сохраняется.
  - host.json (tunnel), cmtunh в FixedQuery нет → host.json удалён, запись `net:host` убрана, выполнена Nft с удалением таблицы cm_host
  - host.json (tunnel), cmtunh есть, внешние интерфейсы те же → команд ip rule нет
  - host.json (tunnel), default перешёл с up0 на up1 → выполнены del правил up0 и add правил up1; host.json обновлён

### I07.H9 — Дать команду `cm host` для смены режима хоста.

Файлы: `src/net/cli.rs`, `src/net/mod.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: I07.H6, I07.H7.

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

Нынешнюю команду `cm vpn` и службу VPN хоста эта вершина не меняет.

Контракты, которые проверит роль 2:
- **H12 → AB.Z:** `cm host set` выполняет переход целиком и откатывает его при отказе.
  - `cm host status` до команд: "mode=off generation=- uplinks=-", код 0
  - `cm host set tunnel --config F` → код 0; status: "mode=tunnel generation=1 uplinks=up0"
  - `cm host set tunnel --config F` повторно → код 0, поколение прежнее
  - `cm host set off` → код 0; нет cmtunh, cmsink, правил 1900–2001, таблицы ip cm_host
  - `cm host set tunnel --config <негодный конфиг>` → код 3, в stderr "[invalid_config]"; после этого status: mode=off; правил 1900–2001 нет
  - `cm host set proxy` без --config → код 2
  - контроллер не запущен → код 4
- **Z01 → AB.Z:** Туннель хоста и туннель приложения работают одновременно и не используют друг друга.
  - `cm host set tunnel`, затем запрос хоста → 200; счётчик ядра хоста вырос
  - туннель приложения поднят; запрос приложения → 200; счётчик ядра приложения вырос, счётчик ядра хоста не изменился
  - `kill -9` ядра хоста → запрос хоста: тайм-аут; запрос приложения: 200
  - `ip link del cmtunh` → запрос хоста: тайм-аут (приёмник); запрос приложения: 200
  - запрос хоста по IPv6 → блокирован; сокет, привязанный к внешнему интерфейсу, проходит
  - DNS хоста на сервер локальной сети → отвечает ядро хоста (SERVFAIL без выхода наружу — тоже его ответ); сервер локальной сети запроса не получил
  - маршрут по умолчанию переведён на второй интерфейс → не позже чем через 5 с есть правило `oif` для него; запрос приложения → 200
  - строгий rp_filter (1 во всех `conf/*/rp_filter`) → запросы хоста и приложения → 200
  - `cm host set off` → правил 1900–2001, cmtunh, cmsink и таблицы ip cm_host нет; запрос приложения → 200; запрос хоста → 200 напрямую
  - ядро хоста и ядро приложения: Uid — клиент, CapEff 0

### I08.N9 — Читать политику туннеля из каталога владельца.

Файлы: `src/net/policy.rs`, `src/net/mod.rs`, `src/controller/owner.rs`, `src/controller/net_ops.rs`. Зависит от: —.

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

`net_ops::apply`: политика читается до первого шага. `ipv6 == Pass` → `unsupported`, ни одной команды не выполнено. Иначе в `net::create` передаётся `policy.ipv6` (сейчас там постоянное `Block`).

Контракты, которые проверит роль 2:
- **D01 → I10.D1:** Политика туннеля читается с теми же проверками, что конфиг, и имеет безопасные значения по умолчанию.
  - parse(b"{}") == {ipv6: Block, fake_ip: Forbidden, lifetime: Pinned, failure: Block}
  - parse({"lifetime":"refcount","failure":"failover","fake_ip":"allowed"}) — три поля изменены, ipv6 Block
  - parse({"dns":"8.8.8.8"}) → Err(Unsupported); parse(b"[]") → Err; parse({"ipv6":"maybe"}) → Err
  - read_policy: файла нет → значения по умолчанию
  - net.json — символическая ссылка → отказ с тем же кодом, что у ссылки вместо конфига
  - net.json длиннее 1024 байт → too_large
  - net.json с лишним полем → invalid_config
  - net_apply при {"ipv6":"pass"} → unsupported, RecordingExec пуст, аренды туннеля нет
- **D02 → I09.F3:** Сторож берёт режим отказа из политики туннеля.
  - net.json {"failure":"block"}: шесть гибелей ядра подряд в одном окне → после шестой состояние failed, restarts 5
  - net.json отсутствует → поведение то же, что при block
- **X01 → I12.A11:** Клиент пишет политику туннеля в том виде, в каком её читает контроллер.
  - net.json, записанный `cm tunnel up` с флагами `--lifetime refcount --failure failover`, разбирается `net::policy::parse` в {lifetime Refcount, failure Failover}
  - профиль подключения с ipv6 pass → `cm tunnel up` получает от контроллера unsupported: код 3, "[unsupported]"; сети нет
  - `--lifetime forever` → код 2, к контроллеру обращений нет
- **X04 → I12.A12:** Срок жизни туннеля задаёт владелец в политике, а не запрос на запуск.
  - в кадрах app_launch, net_apply, worker_start нет поля срока жизни: кадр с полем "lifetime" → bad_frame
  - net.json изменён с pinned на refcount при работающем туннеле → следующий tick применяет новое значение

### I10.D1 — Настроить DNS ядра по политике туннеля и сбрасывать кэши при смене поколения.

Файлы: `src/net/dns.rs`, `src/core/mihomo/config.rs`, `src/core/mihomo/lifecycle.rs`, `src/core/adapter.rs`, `src/controller/core_ops.rs`. Зависит от: I08.N9.

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

Отдельного DNS-процесса нет ([ADR-DNS](ADR-DNS.md)).

Контракты, которые проверит роль 2:
- **D03 → AB.Z:** Конфиг DNS ядра следует политике; кэши не переживают смену поколения.
  - dns_runtime(по умолчанию, tunnel_net(3)) == {resolver "198.18.3.2", enhanced_mode "redir-host", fake_ip_range None, nameservers ["https://1.1.1.1/dns-query", "https://dns.google/dns-query"], bootstrap ["1.1.1.1", "8.8.8.8"]}
  - fake_ip Allowed, tunnel_net(3) → "fake-ip", Some("198.19.3.0/24")
  - ipv6 Pass → Err(Unsupported)
  - attach_tun_dns: dns.enhanced-mode и dns.fake-ip-range по политике; profile.store-fake-ip == false; документ с ключом profile → Err(Forbidden)
  - attach_tun(document, tunnel) == attach_tun_dns(document, tunnel, политика по умолчанию)
  - стенд, fake_ip allowed: первое имя получает адрес из 198.19.0.0/24; после worker_reload новое имя получает тот же первый адрес (таблица сброшена)
- **D04 → AB.Z:** Резолвер туннеля отвечает на все типы запросов и не выпускает DNS мимо туннеля.
  - redir-host: A → адрес сервера; AAAA → NOERROR без записей; TXT, MX, SRV, HTTPS, SVCB → запись сервера
  - redir-host: несуществующее имя → NXDOMAIN
  - усечённый ответ по UDP → клиент получает полную запись (ядро повторило по TCP)
  - запрос по TCP → ответ
  - запрос приложения на 9.9.9.9:53 → тот же ответ (перехват); на интерфейсе «интернета» нет пакетов на 9.9.9.9
  - fake-ip: A → адрес из диапазона; несуществующее имя → адрес из диапазона (а не NXDOMAIN); HTTPS → NOERROR без записей
  - UDP не на порт 53: эхо-сервер отвечает через туннель
  - соединение по IPv6-адресу из сети приложения → немедленный отказ (curl rc 7), пакетов IPv6 на «интернете» нет
  - POST /cache/fakeip/flush и /cache/dns/flush → 204
- **Z04 → AB.Z:** Существующие проверки прошлых пакетов остаются зелёными с новым кодом.
  - EXIT 0 с `CM_TEST_MIHOMO` и `CM_TEST_XRAY`; строк SKIPPED нет
  - стенд прошлого пакета `tests/pack_stand.py`: все проверки сошлись (ожидание `auto-detect-interface` в нём и в `tests/audit_pack_net.rs` — true)
  - число тестов не меньше 650 плюс новые

### I09.F1 — Сводить состояние при старте контроллера и при загрузке системы.

Файлы: `src/controller/boot.rs`, `src/controller/server.rs`, `src/controller/mod.rs`, `src/main.rs`. Зависит от: I09.F0, I07.H7.

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
```

Контракты, которые проверит роль 2:
- **F03 → I09.F3:** После гибели контроллера сиротские ядра останавливаются, сеть остаётся и блокирует трафик.
  - стенд: `kill -9` контроллера при работающем туннеле → новый контроллер при старте: процесса ядра нет, worker_status running false, cmtun0 и netns cm-0 на месте
  - тот же момент: запрос из приложения — тайм-аут; на «интернете» нет пакетов с адреса хоста от имени приложения
  - после worker_start того же поколения запрос приложения снова 200; приложение не перезапускалось
  - `cm controller reconcile --boot --base DIR` печатает одну строку JSON с шестью полями отчёта, код 0
  - без --base в тестовом режиме → код 2; не root вне тестового режима → код 4
  - в тексте установки есть юнит cm-reconcile.service с `Before=network-pre.target` и `ExecStart=… controller reconcile --boot`

### I09.F2 — Решать, что делать при отказе ядра, сервера или сети.

Файлы: `src/net/failure.rs`, `src/net/mod.rs`. Зависит от: —.

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

`GiveUp` означает состояние `Failed`: трафик заблокирован до команды пользователя.

Контракты, которые проверит роль 2:
- **F04 → I09.F3:** Перезапуски ядра ограничены окном и растущей задержкой.
  - пять CoreDown подряд (одно окно) → задержки 0, 1000, 5000, 30000, 120000; шестой → GiveUp
  - CoreDown через 600000 мс после начала окна → задержка 0, restarts 1, окно начато заново
  - RemoteLost, Block → Block; RemoteLost, Failover ×3 → Failover; четвёртый → Block
  - Drift → Repair; ReloadRejected → None; UplinkLost → None; состояние не меняется
  - свойство (10 000 случайных последовательностей отказов и времён): restarts <= 5 и switches <= 3 в любом окне

### I09.F3 — Следить за ядрами, сетью и внешними интерфейсами и чинить расхождения.

Файлы: `src/controller/watch.rs`, `src/controller/server.rs`, `src/controller/core_ops.rs`, `src/controller/protocol.rs`, `src/controller/dispatch.rs`, `src/controller/mod.rs`. Зависит от: I09.F1, I09.F2, I08.N9.

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
`state` — `stopped|starting|up|degraded|blocked|failed`. Экземпляр не запускался → `stopped`, нули. `holders` заполняет `I12.A12`; до неё — 0. `Watch` разделяется между потоком и обработчиком через `Workers` (`Workers::set_watch`, как `set_net`): в `Deps` новых полей нет.

Контракты, которые проверит роль 2:
- **F05 → AB.Z:** Сторож возвращает туннель в работу без участия клиента и не выпускает трафик напрямую.
  - tick при живом ядре: состояние up, restarts 0
  - ядро мертво: tick → blocked; следующий tick с тем же временем → ядро запущено заново с прежним поколением, restarts 1
  - вторая гибель: перезапуск не раньше чем через 1000 мс
  - шестая гибель в окне → failed; дальнейшие tick ядро не запускают; worker_stop и worker_start возвращают up
  - FixedQuery без правила priority 1000 → выполнена команда его добавления; состояние прошло blocked → starting
  - tunnel_state незапущенного экземпляра → Tunnel{state "stopped", restarts 0, holders 0}; чужой uid получает тот же ответ
  - host.json tunnel, FixedQuery сменил up0 на up1 → выполнены uplink_commands, host.json обновлён; повторный tick команд не даёт
  - стенд, матрица: `kill -9` ядра → запрос приложения сначала тайм-аут, не позже чем через 5 с снова 200, приложение то же
  - стенд: `ip link del cmtun0` → запрос тайм-аут (blackhole), прямых пакетов нет
  - стенд: `ip rule del priority 1000` → не позже чем через 5 с правило восстановлено
  - стенд: worker_reload с негодным конфигом → invalid_config; запрос приложения 200 (прежний конфиг)
  - стенд: внешний интерфейс выключен на 3 с и включён → запрос приложения снова 200 без команд клиента
  - стенд: `kill -9` клиента `cm app run` → приложение живо, запрос 200
  - во всех строках матрицы: на «интернете» 0 пакетов с адреса 10.213.0.2
- **X02 → I12.A11:** Команды `cm tunnel` опираются на состояние, которое ведёт сторож.
  - `cm tunnel status t1` до `up` → "state=stopped restarts=0 holders=0 generation=-"
  - после `kill -9` ядра и одного прохода сторожа → restarts=1, затем state=up
  - `cm tunnel up t1` при state=up → действий нет (в журнале контроллера нет новой транзакции)
- **X03 → I12.A12:** Автоматическая остановка — шаг того же прохода сторожа и идёт под блокировкой экземпляра.
  - один вызов tick выполняет по порядку: внешние интерфейсы, ядра, сеть, срок жизни (порядок команд в записях подменных исполнителей)
  - экземпляр занят операцией клиента → tick его пропускает, остальные обрабатывает
  - app_launch во время автоматической остановки → busy или not_running; процесс в снимаемой сети не появился
- **Z02 → AB.Z:** Два туннеля двух владельцев независимы: снятие и отказ одного не задевают другого.
  - uid A и uid B поднимают по туннелю: netns cm-0 и cm-1, таблицы 100 и 101; запросы обоих → 200
  - из cm-0 адрес 10.213.1.2 недоступен (drop в forward); счётчик `oifname "cmv*" counter drop` вырос
  - net_revert туннеля A → cm-1, cmv1h, cmtun1, правило 1001 на месте; запрос B → 200
  - `kill -9` ядра A → запрос B → 200
  - uid B: tunnel_state, app_list, worker_stop, net_revert для экземпляра A → not_running или stopped с нулями; состояние A не изменилось
  - после снятия обоих: в таблице inet cm остались только запреты (два в forward, один в input)

### I11.A8 — Находить процессы в сети туннеля и экземпляры программы вне её.

Файлы: `src/app/instance.rs`, `src/app/mod.rs`, `src/controller/protocol.rs`, `src/controller/app_ops.rs`, `src/controller/dispatch.rs`. Зависит от: —.

Принадлежность процесса туннелю определяется его сетевым пространством, а не тем, кто его запустил: так учитываются и фоновые потомки.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo { pub pid: u32, pub uid: u32, pub exe: Option<PathBuf>, pub netns: Option<u64> }

pub fn netns_id(link: &str) -> Option<u64>;            // "net:[4026531840]" → 4026531840
pub fn netns_of_path(path: &Path) -> Option<u64>;      // inode файла /run/netns/cm-<i>
pub fn scan(proc_root: &Path) -> Vec<ProcInfo>;
pub fn holders(procs: &[ProcInfo], netns: u64, uid: u32) -> Vec<u32>;
pub fn elsewhere(procs: &[ProcInfo], uid: u32, exe: &Path, netns: u64) -> Vec<u32>;
```

`scan`: каталоги `proc_root` с числовым именем. `uid` — владелец каталога; `exe` — цель ссылки `exe`; `netns` — `netns_id` от цели ссылки `ns/net`. Нечитаемая ссылка → `None`, запись остаётся. Результат — по возрастанию `pid`.

`holders`: процессы с этим `netns` и этим `uid`, по возрастанию.

`elsewhere`: процессы того же `uid` с тем же `exe`, у которых `netns` известен и отличается от заданного.

**Кадр**:
```rust
Op::AppList { instance: String }                       // класс Status
ReplyData::Apps { netns: String, pids: Vec<u32> }
```
Контроллер: сети у экземпляра нет → `not_running`. Иначе `netns` — имя `cm-<i>`, `pids` — `holders(scan("/proc"), netns_of_path(<файл netns>), peer.uid)`, не больше 256.

Контракты, которые проверит роль 2:
- **T01 → I12.A12:** Держатели туннеля — процессы в его сетевом пространстве, включая потомков.
  - netns_id("net:[4026531840]") == Some(4026531840); netns_id("net:[x]") == None; netns_id("mnt:[1]") == None
  - фикстура: pid 10 (ns 100, /usr/bin/app), 11 (ns 200, /usr/bin/app), 12 (ns 200, /usr/bin/other), 13 (ссылки ns нет) → scan возвращает 4 записи по возрастанию pid; у 13 netns None
  - holders(…, 200, uid) == [11, 12]; holders(…, 300, uid) пуст; holders с чужим uid пуст
  - каталог с нечисловым именем (`self`, `net`) в scan не попадает
  - app_list экземпляра без сети → not_running; класс операции Status
  - app_list от uid B для сети uid A → not_running
- **T02 → I11.A9:** Экземпляр программы вне сети туннеля обнаруживается до запуска.
  - та же фикстура: elsewhere(…, uid, "/usr/bin/app", 200) == [10]; для сети 100 == [11]
  - elsewhere для "/usr/bin/other" и сети 200 пуст
  - процесс с неизвестным netns в elsewhere не попадает
  - процесс чужого uid с тем же exe не попадает
- **T11 → I11.A9:** Контроллер отдаёт клиенту имя сети и её процессы.
  - после запуска `sleep 30` в туннеле: app_list → Apps{netns "cm-0", pids содержит pid запуска}
  - приложение, породившее фонового потомка и завершившееся: pid потомка остаётся в app_list
  - после завершения всех процессов app_list → pids пуст

### I12.A10 — Разрешить назначение приложения в туннель, профиль подключения и источник.

Файлы: `src/app/assign.rs`, `src/app/mod.rs`. Зависит от: —.

Чистые функции над `GraphSnapshot`.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTunnel {
    pub tunnel_instance_id: Id,
    pub instance: String,              // имя экземпляра для контроллера
    pub generation: u64,
    pub connection_profile_id: Id,
    pub source_id: Id,
    pub selection: NodeSelection,
    pub kernel: KernelKind,
    pub dns: DnsPolicy,
    pub owner: TunnelOwner,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedApp { pub application_id: Id, pub tunnel: ResolvedTunnel, pub shared: bool, pub preset_id: Id, pub data_profile_id: Id }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignError { UnknownApplication, UnknownGroup, UnknownTunnel, UnknownProfile, UnknownSource, HostTunnel, OwnerMismatch, BadInstanceName }

pub fn tunnel(snapshot: &GraphSnapshot, tunnel_id: &Id) -> Result<ResolvedTunnel, AssignError>;
pub fn application(snapshot: &GraphSnapshot, app_id: &Id) -> Result<ResolvedApp, AssignError>;
```

`tunnel`: экземпляр → профиль подключения → источник; отсутствие звена — своя ошибка. `instance` — текст идентификатора туннеля; если он не проходит `InstanceId::new` или равен `host` → `BadInstanceName`.

`application`:
- `OwnTunnel { tunnel_instance_id }` → `tunnel(…)`; владелец туннеля обязан быть `Application { application_id }` этого приложения, иначе `OwnerMismatch`; `shared = false`;
- `Group { group_id }` → группа → её туннель; владелец обязан быть `Group { group_id }`, иначе `OwnerMismatch`; `shared = true`;
- владелец туннеля `Host` → `HostTunnel`: приложение не назначается в туннель хоста.

У каждого приложения свой профиль подключения и свой источник: два приложения могут одновременно работать через разные подписки.

Контракты, которые проверит роль 2:
- **T03 → I12.A11:** Туннель разрешается до профиля подключения и источника; имя экземпляра годно для контроллера.
  - туннель t1 (владелец Application a1, профиль p1 → источник s1) → ResolvedTunnel{instance "t1", source_id s1, connection_profile_id p1}
  - неизвестный туннель → UnknownTunnel; профиля нет → UnknownProfile; источника нет → UnknownSource
  - идентификатор туннеля "Work_VPN" → BadInstanceName; "host" → BadInstanceName; 33 символа → BadInstanceName
  - два туннеля с разными источниками → два ResolvedTunnel с разными source_id
- **T04 → I11.A9:** Приложение попадает только в свой туннель или в туннель своей группы.
  - приложение a1 с OwnTunnel t1 (владелец Application a1) → shared false, tunnel.instance "t1"
  - приложение a2 с OwnTunnel t1 (владелец Application a1) → OwnerMismatch
  - приложение в группе g1, туннель группы принадлежит Group g1 → shared true
  - туннель группы принадлежит Group g2 → OwnerMismatch; неизвестная группа → UnknownGroup
  - назначение в туннель с владельцем Host → HostTunnel
  - неизвестное приложение → UnknownApplication

### I15.E1 — Вывести каталоги данных и каталог сокетов сессии из профиля данных.

Файлы: `src/env/private.rs`, `src/env/mod.rs`. Зависит от: —.

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataState { #[default] Existing, Isolated, Fresh }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcPolicy { #[default] Shared, Private }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateDirs { pub root: PathBuf, pub config: PathBuf, pub data: PathBuf, pub cache: PathBuf, pub state: PathBuf }

pub fn valid_name(name: &str) -> bool;   // 1..=32 символа из a-z, 0-9, '-'; не начинается с '-'
pub fn data_dirs(home: &Path, profile: &str, state: DataState, session: &str) -> Option<PrivateDirs>;
pub fn runtime_dir(runtime: &Path, session: &str) -> PathBuf;                // <runtime>/cm/<session>
pub fn env_overrides(dirs: &PrivateDirs) -> BTreeMap<String, String>;
pub fn prepare(dirs: &PrivateDirs) -> std::io::Result<()>;
pub fn prepare_runtime(path: &Path) -> std::io::Result<()>;
pub fn check_dir(path: &Path, uid: u32) -> bool;
```

`data_dirs`:
- `Existing` → `None`: приложение работает со своими обычными данными;
- `Isolated` → `root = <home>/.local/share/cm/data/<profile>`;
- `Fresh` → `root = <home>/.local/share/cm/data/<profile>/fresh-<session>`. Каталог автоматически не удаляется;
- подкаталоги `config`, `data`, `cache`, `state`.

`env_overrides`: `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`. `HOME` не меняется: программа, которая пишет мимо XDG, профилем не отделяется — это показывает `I13.P2`.

`prepare`, `prepare_runtime` выполняет клиент под своим uid: `mkdir` с правами 0700, существующий каталог допустим; ссылка или не каталог на месте любого из путей → ошибка.

`check_dir` (для контроллера): `lstat`; настоящий каталог, владелец `uid`, `mode & 0o077 == 0`.

Контракты, которые проверит роль 2:
- **T05 → I12.A13:** Имена профиля данных и сессии проходят одну проверку; каталоги выводятся из них однозначно.
  - valid_name: "work", "s0123456789ab", "a-1" → true; "", "-a", "A", "a_b", "a/b", 33 символа → false
  - data_dirs("/home/u", "work", Existing, "s1") == None
  - Isolated → root "/home/u/.local/share/cm/data/work"; config, data, cache, state — его подкаталоги
  - Fresh, сессия "s1" → root "/home/u/.local/share/cm/data/work/fresh-s1"
  - env_overrides — ровно четыре ключа XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME, XDG_STATE_HOME; HOME нет
  - runtime_dir("/run/user/1000", "s1") == "/run/user/1000/cm/s1"
  - prepare создаёт пять каталогов с правами 0700; повторный вызов — Ok; ссылка на месте root → ошибка
  - check_dir: каталог 0700 своего uid → true; 0750 → false; ссылка на такой каталог → false; файл → false; чужой uid → false
- **T08 → I15.E2:** Каталоги данных и сокетов сессии принимаются, только если их подготовил сам пользователь.
  - data {profile work, state isolated}, каталоги подготовлены → в окружении процесса XDG_CONFIG_HOME = <home>/.local/share/cm/data/work/config (и ещё три)
  - каталог data отсутствует → invalid_config, процесс не создан; контроллер каталог не создал
  - root — ссылка на чужой каталог → invalid_config
  - каталог с правами 0755 → invalid_config
  - ipc private: XDG_RUNTIME_DIR = /run/user/<uid>/cm/<session>; DBUS_SESSION_BUS_ADDRESS в окружении нет; WAYLAND_DISPLAY "wayland-1" стал "/run/user/<uid>/wayland-1"; PULSE_SERVER = unix:/run/user/<uid>/pulse/native
  - ipc private, WAYLAND_DISPLAY уже абсолютный → не изменён; PULSE_SERVER задан клиентом → не изменён
  - ipc shared: XDG_RUNTIME_DIR = /run/user/<uid>, DBUS_SESSION_BUS_ADDRESS сохранён
  - в обоих режимах в окружении нет SSH_AUTH_SOCK, GNOME_KEYRING_CONTROL, GPG_AGENT_INFO, даже если клиент их прислал (кадр с ними → bad_argument)
  - две сессии с профилями work и home: файл, созданный первой в XDG_DATA_HOME, второй не виден
- **T09 → I13.P2:** Ограничения запуска зависят от политики сокетов и состояния данных.
  - support(Native, Private, Existing) == Supported
  - support(Native, Shared, Existing) == Partial([SessionBus])
  - support(Native, Shared, Isolated) == Partial([SessionBus, DataOutsideXdg])
  - support(AppImage, Private, Existing) == Partial([FuseHelper])
  - support(AppImage, Shared, Fresh) == Partial([FuseHelper, SessionBus, DataOutsideXdg])

### I12.A13 — Дать каждой сессии приложения своё окружение.

Файлы: `src/app/env.rs`, `src/app/mod.rs`, `src/controller/protocol.rs`, `src/controller/codec.rs`, `src/controller/app_ops.rs`. Зависит от: I15.E1.

Сейчас регион берётся из `config/env.json` экземпляра: два приложения на общем туннеле получают одно окружение. Эта вершина добавляет окружение на сессию.

```rust
// src/app/env.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRef { pub profile: String, pub state: DataState }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFile {
    pub timezone: String,
    pub locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub data: Option<DataRef>,
    #[serde(default)] pub ipc: IpcPolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvError { Malformed, BadZone, BadLocale, BadProfile }
pub const MAX_SESSION_BYTES: usize = 4096;

pub fn parse_session(bytes: &[u8]) -> Result<SessionFile, EnvError>;
pub fn session_file(preset: &EnvironmentPreset, data: Option<DataRef>, ipc: IpcPolicy) -> SessionFile;
pub fn environment_profile(id: Id, session_id: Id, preset: &EnvironmentPreset) -> EnvironmentProfile;
```

`parse_session`: не JSON или лишнее поле → `Malformed`; `timezone` не проходит `identity::tzdata::valid_zone_name` → `BadZone`; `locale` — 1..=64 символа из букв, цифр, `_ . - @`, иначе `BadLocale`; `data.profile` не проходит `private::valid_name` → `BadProfile`.

`environment_profile` копирует пояс, локаль и языки из пресета в момент создания сессии: последующая правка пресета живую сессию не меняет.

**Кадр**: `Op::AppLaunch` получает поле
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
session: Option<String>,
```
Кадр без поля разбирается как раньше. Декодер: значение не проходит `private::valid_name` → `bad_argument`.

**Контроллер** (`app_ops`):
- `session` задан → читается `config/session-<session>.json` (`read_owned_file`, предел `MAX_SESSION_BYTES`) и `parse_session`; файла нет или ошибка разбора → `invalid_config`, процесс не создан; `TZ` и `LANG` — из этого файла;
- `session` не задан → как сейчас (`config/env.json`, если есть).

Поля `data` и `ipc` применяет `I15.E2`.

Контракты, которые проверит роль 2:
- **T06 → I15.E2:** Окружение сессии читается из её файла с теми же проверками, что конфиг.
  - parse_session({"timezone":"Europe/Berlin","locale":"de_DE.UTF-8"}) → data None, ipc Shared
  - лишнее поле → Malformed; timezone "../etc/passwd" → BadZone; locale "de DE" → BadLocale; data.profile "A" → BadProfile
  - session_file(preset, None, Shared) несёт пояс и локаль пресета
  - environment_profile: после правки пресета ранее созданный профиль не изменился
  - кадр app_launch без поля session разбирается; сериализация кадра без сессии не содержит ключа "session"
  - session "../x" → bad_argument
  - app_launch с session "s1", файла session-s1.json нет → invalid_config, процесс не создан
  - файл сессии — ссылка → отказ; длиннее 4096 байт → too_large
  - стенд: два запуска в одном туннеле с сессиями Europe/Berlin и Asia/Tokyo → у процессов TZ разные, LANG разные
- **T07 → I14.R6:** Процесс без переменной TZ видит пояс региона, а хост — свой.
  - запуск с сессией Europe/Berlin, программа `env -u TZ date +%Z` (вывод в файл) → CET или CEST
  - второй запуск в том же туннеле с Asia/Tokyo → JST
  - sha256 файла /etc/localtime на хосте до и после запусков совпадает; `readlink /etc/localtime` не изменился
  - session с timezone "Nowhere/City" (имя допустимо, файла нет) → invalid_config, процесс не создан
  - запуск без пояса: /etc/localtime внутри совпадает с хостом

### I15.E2 — Применить каталоги данных и политику сокетов сессии при запуске.

Файлы: `src/controller/app_ops.rs`. Зависит от: I15.E1, I12.A13.

Контроллер ничего не создаёт в каталогах пользователя: каталоги готовит клиент (`private::prepare`), контроллер только проверяет их и выставляет переменные.

Обозначения: `home` — домашний каталог uid из базы пользователей (как сейчас в `env_of`); `runtime = /run/user/<uid>`.

**`data`** задан и `state != Existing`:
1. `dirs = private::data_dirs(home, profile, state, session)`;
2. `private::check_dir(path, uid)` для `dirs.root` и четырёх подкаталогов; любой отказ → `invalid_config`, процесс не создан;
3. в окружение добавляются `private::env_overrides(dirs)`.

**`ipc == Private`**:
1. `dir = private::runtime_dir(runtime, session)`; `check_dir(dir, uid)`, отказ → `invalid_config`;
2. `XDG_RUNTIME_DIR = dir`;
3. `DBUS_SESSION_BUS_ADDRESS` из окружения удаляется;
4. `WAYLAND_DISPLAY` без `/` в начале → `<runtime>/<значение>`;
5. `PULSE_SERVER` не задан → `unix:<runtime>/pulse/native`.

**`ipc == Shared`** — как сейчас.

В обоих режимах приложению не передаются `SSH_AUTH_SOCK`, `GNOME_KEYRING_CONTROL`, `GPG_AGENT_INFO`: их нет в `SESSION_ENV_KEYS`, и эта вершина список не расширяет.

Порядок наложения переменных не меняется: сначала переменные сессии клиента, затем то, что ставит контроллер; значения контроллера перекрывают клиентские.

Контракты, которые проверит роль 2:
- **T13 → I11.A9:** Клиент сам готовит каталоги сессии, и контроллер их принимает.
  - `cm app run --data isolated --ipc private a1` → код 0; созданы каталоги данных профиля и /run/user/<uid>/cm/<сессия>, все 0700
  - в config/ владельца появился файл session-<имя>.json, имя — `s` и 12 шестнадцатеричных цифр
  - два запуска подряд дают разные имена сессий
  - `--data fresh` дважды → два разных каталога fresh-<сессия>; после завершения приложений оба на месте

### I14.R6 — Подставить процессу файл часового пояса региона.

Файлы: `src/controller/drop.rs`, `src/controller/app_ops.rs`. Зависит от: I12.A13.

Одной переменной `TZ` мало: программа без неё читает `/etc/localtime` хоста.

`drop.rs`: новая функция `drop_into_netns_region_pre_exec(command, netns, resolv, localtime: Option<&Path>, run_as, keep_fds, limits)`. Прежняя `drop_into_netns_pre_exec` остаётся и вызывает новую с `None`. В потомке, в его собственном mount namespace, сразу после привязки `resolv.conf`: `mount(<localtime>, "/etc/localtime", MS_BIND)`. Отказ `mount` — отказ запуска, как для `resolv.conf`. Путь с NUL → `bad_argument` до запуска.

`app_ops`: если у запуска есть пояс (из файла сессии или `env.json`):
- файл пояса — `<tzdir>/<timezone>`, `tzdir = /usr/share/zoneinfo`;
- это не обычный файл → `invalid_config`, процесс не создан;
- иначе путь передаётся в `drop_into_netns_region_pre_exec`.

Нет пояса — `localtime = None`, поведение прежнее.

`/etc/localtime` хоста обычно ссылка: привязка ложится на файл, на который она указывает, и видна только в mount namespace этого приложения. Хост не меняется.

Контракты, которые проверит роль 2:
- **R04 → AB.Z:** Отказ подстановки пояса останавливает запуск, а не оставляет пояс хоста.
  - drop_into_netns_pre_exec (прежняя сигнатура) собирается и ведёт себя как раньше
  - путь пояса с NUL → bad_argument, процесс не создан
  - пояс задан, а файла нет → invalid_config; в ответе нет пути и имени пояса
- **Z05 → AB.Z:** Два приложения на общем туннеле получают разный регион процесса.
  - `cm app run` двух приложений группы с пресетами Europe/Berlin (de_DE.UTF-8) и Asia/Tokyo (ja_JP.UTF-8), локали разрешены флагом → у первого TZ Europe/Berlin и `date +%Z` без TZ даёт CET/CEST, у второго — Asia/Tokyo и JST
  - LANG процессов — из своих пресетов
  - оба процесса в одном netns и выходят через один узел

### I12.A11 — Поднимать и снимать туннель по описанию из хранилища командой `cm tunnel`.

Файлы: `src/app/tunnel.rs`, `src/app/mod.rs`, `src/core/mihomo/config.rs`, `src/controller/client.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: I12.A10, I08.N9, I09.F3.

До сих пор туннель приложения поднимал только стенд. Эта вершина делает то же из хранилища.

`src/main.rs`: `if args.first() == "tunnel" { exit(cm::app::tunnel::dispatch(&args[1..])) }`.

```
cm tunnel up ID [--lifetime pinned|refcount] [--failure block|failover]
cm tunnel down ID [--keep-net] [--force]
cm tunnel status ID
```
`ID` — идентификатор `TunnelInstance`.

`config.rs`: `pub fn document_from_store(store, source_id, selection, dns, user_rules, auto_group) -> Result<Value, ConfigError>` — та часть `generate_from_store`, что идёт до слушателей; `generate_from_store` вызывает её. Поведение `generate_from_store` не меняется.

`client.rs`: `pub fn owner_config_dir(instance: &str) -> PathBuf` — `<base>/u<euid>/instances/<instance>/config`; `<base>` — `SYSTEM_BASE`, в `test_mode()` — `CM_CONTROLLER_BASE`, если задана.

**`up`**:
1. Хранилище (`sources::cli::store_root()`) → `assign::tunnel`. Ошибка → код 2, токен в `stderr`.
2. `tunnel_state` → `up` → код 0, действий нет.
3. `instance_prepare`.
4. `N` = поколение из `worker_status` + 1; ядра не было → 1.
5. Документ: `document_from_store(…, user_rules = [], auto_group = true)`, ключ `dns` удалить (DNS настраивает контроллер). Записать в `gen-<N>.json` (0600, временный файл и `rename`).
6. `net.json`: `ipv6` и `fake_ip` — из `ResolvedTunnel::dns`, `lifetime` и `failure` — из флагов (по умолчанию `pinned`, `block`).
7. `net_apply{N}`; ответ `conflict` означает, что сеть уже есть, и ошибкой не считается.
8. `worker_start{N}`.
9. Печать: `instance=<имя> generation=<N> netns=<имя netns>`. Код 0.

**`down`**:
1. `app_list`: есть процессы и нет `--force` → код 5, `[in_use]`.
2. `worker_stop`; `not_running` не ошибка.
3. Без `--keep-net` — `net_revert`. С `--keep-net` сеть остаётся: живые приложения заблокированы, а не выходят напрямую (Q12).

**`status`**: `tunnel_state` и `app_list` → `state=<…> restarts=<n> holders=<n> generation=<N|->`.

Коды выхода: 0; 2 — использование или описание; 3 — отказ контроллера (`[код]` в `stderr`); 4 — контроллер недоступен; 5 — туннель занят.

Запись состояния туннеля и сессий обратно в хранилище в эту вершину не входит.

Контракты, которые проверит роль 2:
- **T10 → I14.R7:** Туннель поднимается из хранилища одной командой и снимается без утечки.
  - `cm tunnel up t1` → код 0, строка "instance=t1 generation=1 netns=cm-0"; `cm tunnel status t1` → "state=up restarts=0 holders=0 generation=1"
  - повторный `cm tunnel up t1` → код 0, поколение прежнее, второго ядра нет
  - `cm tunnel up t2` (другой источник) → netns cm-1; запрос из cm-0 прошёл через прокси источника 1, из cm-1 — через прокси источника 2 (журналы прокси)
  - в gen-1.json владельца нет ключей dns, tun, listeners; права 0600
  - net.json: {"ipv6":"block","fake_ip":"forbidden","lifetime":"pinned","failure":"block"} при вызове без флагов
  - `cm tunnel down t1` при живом приложении → код 5, "[in_use]"; с --force → код 0
  - `cm tunnel down t2 --keep-net` → ядра нет, netns cm-1 есть, запрос из него — тайм-аут
  - `cm tunnel up unknown` → код 2; контроллер не запущен → код 4
  - после `down t1`: нет cmv0h, cmtun0, netns cm-0, правила 1000; туннель t2 не затронут

### I12.A12 — Останавливать туннель, когда его покинул последний процесс.

Файлы: `src/controller/watch.rs`, `src/controller/app_ops.rs`. Зависит от: I11.A8, I09.F3, I08.N9.

Четвёртый шаг прохода `Watch::tick` (`I09.F3`).

```rust
pub const EMPTY_GRACE_MS: i64 = 5_000;
impl Watch { pub fn note_launch(&mut self, uid: u32, instance: &str); }
```

- `app_ops::launch` после успешного запуска вызывает `note_launch`: у экземпляра появляется признак «держатели были».
- Проход: `holders` — число процессов владельца в netns экземпляра (`app::instance`). Оно попадает в `ReplyData::Tunnel.holders`.
- Политика `lifetime == Refcount`, держатели были, и сейчас их нет:
  - первый такой проход запоминает время;
  - держатели появились снова → время сбрасывается;
  - прошло `EMPTY_GRACE_MS` → контроллер сам выполняет остановку ядра и снятие сети теми же шагами, что `worker_stop` и `net_revert`. Состояние — `stopped`, признак и время сбрасываются.
- Политика `Pinned` → туннель живёт до команды.
- Экземпляр `host` этой логикой не затрагивается: туннель хоста сессиями не останавливается.

Запуск приложения во время автоматической остановки получает `busy` или `not_running`, но не попадает в полуснятую сеть: остановка идёт под той же блокировкой экземпляра, что операции клиента.

Контракты, которые проверит роль 2:
- **T12 → AB.Z:** Туннель со сроком жизни по держателям останавливается сам, закреплённый — нет.
  - refcount: запуск, держатели есть → tick ничего не снимает
  - держателей нет, прошло 4999 мс → туннель жив; прошло 5000 мс → выполнены остановка ядра и снятие сети, состояние stopped
  - держатели вернулись на 3000-й мс → отсчёт начат заново
  - refcount, запусков не было → туннель не снимается, сколько бы ни прошло
  - pinned → не снимается никогда
  - экземпляр host с refcount в net.json → не снимается
  - tunnel_state.holders равен числу процессов владельца в netns
  - стенд: группа из двух приложений на одном туннеле; первое закрыто → туннель работает, второе отвечает 200; второе закрыто → не позже чем через 10 с нет ни ядра, ни netns
  - стенд: две группы на двух туннелях; снятие первой не меняет вторую
  - стенд: `cm host set off` при работающих туннелях приложений → запросы приложений по-прежнему 200
- **Z07 → AB.Z:** Остановка по сроку жизни не оставляет приложению прямого пути.
  - после автоматического снятия сети процесс, оставшийся с открытым сокетом, не может отправить ни одного пакета: netns удалён вместе с veth
  - на «интернете» 0 пакетов с адреса 10.213.0.2

### I13.P1 — Определить способ упаковки и запуска программы.

Файлы: `src/app/packaging/mod.rs`, `src/app/packaging/detect.rs`, `src/app/mod.rs`. Зависит от: —.

Определяется способ запуска, а не продукт: имён приложений в коде нет.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packaging { Native, AppImage, Flatpak { app_id: Option<String> }, Snap, Launcher }

/// `head` — первые 16 байт файла программы, если их удалось прочитать.
pub fn detect(program: &Path, args: &[String], head: Option<&[u8]>) -> Packaging;
pub fn read_head(program: &Path) -> Option<[u8; 16]>;
```

Проверки по порядку, первая сработавшая определяет ответ. «Имя» — последний компонент пути.

| Условие | Ответ |
|---|---|
| имя `flatpak` и первый аргумент `run` | `Flatpak { app_id }`: первый аргумент после `run`, не начинающийся с `-` |
| путь начинается с `/snap/bin/`, либо имя `snap` и первый аргумент `run` | `Snap` |
| имя из списка `systemd-run`, `gtk-launch`, `gio`, `gapplication`, `dbus-send`, `busctl`, `xdg-open`, `kioclient`, `kioclient5`, `kioclient6`, `dex` | `Launcher` |
| `head` начинается с `\x7fELF`, а байты 8..11 — `41 49 02` или `41 49 01` | `AppImage` |
| иначе | `Native` |

Контракты, которые проверит роль 2:
- **U01 → I13.P2:** Способ запуска определяется по пути, аргументам и началу файла.
  - detect("/usr/bin/flatpak", ["run", "--branch=stable", "org.example.App"], None) == Flatpak{app_id Some("org.example.App")}
  - detect("/usr/bin/flatpak", ["list"], None) == Native
  - detect("/snap/bin/tool", [], None) == Snap; detect("/usr/bin/snap", ["run", "tool"], None) == Snap
  - detect("/usr/bin/systemd-run", ["--user", "app"], None) == Launcher; то же для gtk-launch, gio, xdg-open, dbus-send
  - head \x7fELF\x02\x01\x01\x00 A I \x02 … → AppImage; тот же заголовок без байтов AI → Native
  - head None, обычный путь → Native
  - файл короче 16 байт → read_head None

### I13.P2 — Сказать, что CM обещает для каждого способа запуска.

Файлы: `src/app/packaging/policy.rs`, `src/app/packaging/mod.rs`. Зависит от: I13.P1, I15.E1.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit { SessionBus, FuseHelper, DataOutsideXdg }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason { ExecutorOutsideNetns, NotVerified }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support { Supported, Partial(Vec<Limit>), Unsupported(Reason) }

pub fn support(packaging: &Packaging, ipc: IpcPolicy, data: DataState) -> Support;
pub fn limit_token(limit: Limit) -> &'static str;     // limit_session_bus, limit_fuse_helper, limit_data_outside_xdg
pub fn reason_token(reason: Reason) -> &'static str;  // unsupported_executor_outside, unsupported_not_verified
```

| Упаковка | Ответ |
|---|---|
| `Launcher` | `Unsupported(ExecutorOutsideNetns)`: программу запускает служба сессии, она в сети хоста |
| `Flatpak`, `Snap` | `Unsupported(NotVerified)`: исполнитель и его DNS на настоящих приложениях не проверены |
| `AppImage` | `Partial`: `FuseHelper` плюс ограничения из списка ниже |
| `Native` | ограничения из списка ниже; список пуст → `Supported`, иначе `Partial` |

Общие ограничения, в этом порядке:
- `ipc == Shared` → `SessionBus`: через шину сессии программа может передать запрос уже работающему экземпляру или службе вне туннеля;
- `data != Existing` → `DataOutsideXdg`: отделяются только каталоги XDG.

Порядок в `Partial`: `FuseHelper`, `SessionBus`, `DataOutsideXdg`.

Матрица — это и есть ответ на `I13.T01.a`: для каждой ячейки названы исполнитель и статус. Проверка на настоящих Flatpak и AppImage (`I13.T05.a`) в пакет не входит.

Контракты, которые проверит роль 2:
- **U02 → I11.A9:** Неподдержанный способ запуска — отказ с причиной; ограничения видны пользователю.
  - support(Launcher, любые) == Unsupported(ExecutorOutsideNetns)
  - support(Flatpak, любые) == support(Snap, любые) == Unsupported(NotVerified)
  - limit_token: limit_session_bus, limit_fuse_helper, limit_data_outside_xdg; reason_token: unsupported_executor_outside, unsupported_not_verified
  - `cm app run t1 -- /usr/bin/systemd-run --user x` → код 2, в stderr "[unsupported_executor_outside]"; к контроллеру обращений не было
  - `cm app run t1 -- /usr/bin/flatpak run org.example.App` → код 2, "[unsupported_not_verified]"

### I11.A9 — Запускать приложение с проверками упаковки, региона, чужого экземпляра и передачи управления.

Файлы: `src/app/cli.rs`, `src/i18n_table.rs`. Зависит от: I11.A8, I13.P2, I15.E2, I12.A10.

```
cm app run [--allow-running] [--allow-missing-locale] [--data existing|isolated|fresh] [--ipc shared|private] ID
cm app run ЭКЗЕМПЛЯР -- ПРОГРАММА [АРГУМЕНТЫ]
```

**Запуск по `ID`**, по порядку:
1. Хранилище → `assign::application` (вместо нынешнего разбора назначения в `stored_launch`). Ошибка → код 2.
2. `spec::check` — как сейчас.
3. Упаковка: `packaging::detect` и `packaging::support`. `Unsupported` → `[<reason_token>]`, код 2. `Partial` → по строке `[<limit_token>]` в `stderr`, запуск продолжается.
4. Регион: `env::check_preset(preset, "/usr/share/zoneinfo", <вывод locale -a>)`. `ZoneMissing` → `[zone_missing]`, код 2. `LocaleNotInstalled` без `--allow-missing-locale` → `[locale_not_installed]`, код 2. Остальные замечания запуск не останавливают.
5. `app_list{экземпляр}` → имя netns и список процессов «до».
6. Чужой экземпляр: `instance::elsewhere(scan("/proc"), euid, <канонический путь программы>, netns_of_path("/run/netns/<netns>"))` не пуст и нет `--allow-running` → `[running_elsewhere]`, код 5. Второй запуск такой программы обычно передаёт запрос первому экземпляру, а тот работает вне туннеля.
7. Сессия: имя `s` + 12 шестнадцатеричных цифр из `/dev/urandom`. `private::prepare` для каталогов данных (если `--data` не `existing`) и `prepare_runtime` (если `--ipc private`). Файл `config/session-<имя>.json` из `app::env::session_file` (0600, временный файл и `rename`).
8. `worker_status` → поколение; `app_launch{…, session}`.
9. Передача управления: через 2000 мс — `app_list` «после». Если запущенный процесс уже завершился и среди процессов «после» нет ни одного, которого не было «до» → `[handed_off]`, код 6: программа отдала запрос кому-то вне сети туннеля.
10. Иначе печать pid, код 0.

**Запуск с `--`**: шаги 3, 5, 8, 9, 10; сессии нет, чужой экземпляр не ищется.

Коды выхода: 0; 2 — использование, описание, упаковка, регион; 3 — отказ контроллера; 4 — контроллер недоступен; 5 — программа уже работает вне туннеля; 6 — управление передано.

Поиск чужого экземпляра выполняет клиент, а не контроллер: это защита пользователя от собственной ошибки, а не граница прав. По пути исполняемого файла он не различает программы на общем интерпретаторе — для них есть `--allow-running`.

Контракты, которые проверит роль 2:
- **T14 → AB.Z:** Запуск замечает чужой экземпляр и передачу управления и не молчит об этом.
  - программа-фикстура уже работает вне туннеля → `cm app run a1` → код 5, "[running_elsewhere]"; app_launch не вызывался
  - то же с --allow-running → запуск состоялся
  - фикстура single-instance (первый экземпляр слушает unix-сокет, второй передаёт ему аргументы и выходит с кодом 0), первый экземпляр запущен вне туннеля, второй — `cm app run --allow-running a1` → код 6, "[handed_off]"
  - обычная программа (`sleep 30`) → код 0, в stdout pid
  - пресет с локалью, которой нет в системе → код 2, "[locale_not_installed]"; с --allow-missing-locale → запуск
  - пресет с несуществующим поясом → код 2, "[zone_missing]"
  - несуществующий исполняемый файл → код 3; файл без права исполнения → код 3; процесс не создан
  - программа завершилась сама через 5 с → код 0 (это не передача управления: проверка идёт на 2000-й мс)
  - запуск с `--`: сессии нет, поиск чужого экземпляра не выполняется
- **Z03 → AB.Z:** Полный путь пользователя: описание в хранилище → туннель → приложение → проверка → снятие.
  - `cm tunnel up t1` → `cm app run a1` → pid; процесс: Uid клиента, CapEff 0, интерфейсы lo и cmv0n
  - запрос приложения → 200 через прокси своего источника
  - `cm tunnel verify t1 --country DE` (источники стенда) → axis=verified, код 0
  - `cm tunnel status t1` → state=up, holders >= 1
  - приложение завершено, `cm tunnel down t1` → код 0; нет cmv0h, cmtun0, cm-0, правила 1000
  - за весь сценарий на «интернете» 0 пакетов с адреса 10.213.0.2 и 0 запросов DNS не от ядра

### I14.R4 — Описать источники страны выхода и разобрать их ответы.

Файлы: `src/env/sources.rs`, `src/env/mod.rs`. Зависит от: —.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeoFormat { Trace, Plain, Json(String) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoSource { pub id: String, pub url: String, pub format: GeoFormat }

pub const MAX_GEO_BODY: usize = 4096;
pub fn default_sources() -> Vec<GeoSource>;
pub fn parse(format: &GeoFormat, body: &[u8]) -> Option<String>;
pub fn parse_source_arg(text: &str) -> Option<GeoSource>;     // "id=формат=url"; формат: trace | plain | json:<поле>
```

`default_sources` — три независимых источника:

| id | url | формат |
|---|---|---|
| `cloudflare` | `https://www.cloudflare.com/cdn-cgi/trace` | `Trace` |
| `ipinfo` | `https://ipinfo.io/country` | `Plain` |
| `countryis` | `https://api.country.is/` | `Json("country")` |

`parse`: тело длиннее `MAX_GEO_BODY` или не UTF-8 → `None`.
- `Trace`: строка вида `loc=XX`;
- `Plain`: всё тело без пробелов по краям;
- `Json(field)`: строковое поле верхнего уровня.

Ответ принимается, только если после приведения к верхнему регистру это ровно две латинские буквы. Имя узла в определении страны не участвует нигде.

`parse_source_arg`: `id` — по правилам `private::valid_name`; url начинается с `https://` (в `test_mode()` допустим `http://`); иначе `None`.

Каждый источник — третья сторона, которая видит адрес выхода: их число — цена проверки. Список передаётся параметром, значения по умолчанию можно заменить.

Контракты, которые проверит роль 2:
- **R01 → I14.R5:** Страна берётся только из ответа источника и только как две буквы.
  - parse(Trace, "fl=1\nip=203.0.113.7\nloc=DE\ntls=TLSv1.3\n") == Some("DE")
  - parse(Plain, "de\n") == Some("DE"); parse(Plain, "Germany") == None; parse(Plain, "") == None
  - parse(Json("country"), {"ip":"203.0.113.7","country":"NL"}) == Some("NL"); поле — число → None; не JSON → None
  - тело 4097 байт → None; не UTF-8 → None
  - default_sources: три записи с id cloudflare, ipinfo, countryis; все url начинаются с https://
  - parse_source_arg("a=plain=https://x.test/c") → Some; "a=xml=https://x" → None; "A=plain=https://x" → None; "a=plain=http://x" → None вне тестового режима
  - parse_source_arg("a=json:country=https://x.test/") → формат Json("country")

### I14.R5 — Собрать наблюдения страны запросами из сети самого туннеля.

Файлы: `src/env/collect.rs`, `src/env/mod.rs`, `src/main.rs`. Зависит от: I14.R4.

У ядра туннеля приложения нет порта-прокси: запрос «как у приложения» можно сделать только изнутри его сети. Поэтому пробу выполняет сам `cm`, запущенный там операцией `app_launch`.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectError { Launch, Timeout, Malformed, BadPath }
pub const MAX_RESULT_BYTES: usize = 16 << 10;
pub const PROBE_TIMEOUT_MS: u64 = 5_000;

pub fn probe_main(args: &[String]) -> i32;
pub fn read_result(path: &Path) -> Result<Vec<Observation>, CollectError>;
pub fn collect(socket: &Path, instance: &str, sources: &[GeoSource], runtime: &Path, wait_ms: u64) -> Result<Vec<Observation>, CollectError>;
```

**Внутри сети** — `cm env probe --out ФАЙЛ --source id=формат=url [--source …]` (`src/main.rs`: ветка `env` рядом с `app`):
- `ФАЙЛ` обязан лежать в `$XDG_RUNTIME_DIR/cm/` и не существовать, иначе код 2;
- по каждому источнику — один GET существующим транспортом (`sources::transport::FetchTransport`): тайм-аут `PROBE_TIMEOUT_MS`, тело не длиннее `MAX_GEO_BODY`, перенаправления — по правилам транспорта;
- удачные ответы → `Observation { source, country, at_unix_ms }`; неудачные пропускаются;
- результат — массив JSON, запись через временный файл и `rename`, права 0600. Код 0, даже если массив пуст.

**Снаружи** — `collect`:
1. файл — `<runtime>/cm/probe-<12 шестнадцатеричных цифр>.json`; каталог `<runtime>/cm` создаётся с правами 0700;
2. `worker_status` → поколение; `app_launch` с программой `std::env::current_exe()` и аргументами пробы; отказ → `Launch`;
3. ждать файл до `wait_ms`, шаг 100 мс; не появился → `Timeout`;
4. `read_result` (предел `MAX_RESULT_BYTES`, не JSON → `Malformed`), файл удалить.

Запрос идёт тем же путём, что трафик приложения: TUN ядра, его DNS, его узел.

Контракты, которые проверит роль 2:
- **R02 → I14.R7:** Наблюдения собираются запросами из сети туннеля и не теряются молча.
  - read_result: массив из двух записей → два Observation; не JSON → Malformed; файл 16 КиБ + 1 байт → Malformed
  - `cm env probe` с --out вне $XDG_RUNTIME_DIR/cm → код 2; файл уже существует → код 2
  - стенд: два источника отвечают "DE" → collect возвращает два наблюдения с country "DE"; запросы пришли на «интернет» от туннеля (журнал прокси узла), а не напрямую
  - стенд: один источник не отвечает → одно наблюдение, ошибки нет
  - стенд: ядро туннеля убито → collect возвращает Launch (контроллер ответил not_running)
  - файл результата после чтения удалён; права созданного файла 0600

### I14.R7 — Проверять регион туннеля и замечать его смену.

Файлы: `src/app/tunnel.rs`, `src/i18n_table.rs`. Зависит от: I14.R5, I12.A11.

```
cm tunnel verify ID [--country XX] [--source id=формат=url …]
```

1. `assign::tunnel` → экземпляр. Источники — из флагов, без флагов — `default_sources()`.
2. `collect(…, wait_ms = 15_000)` → `env::geo::decide(наблюдения, сейчас)`.
3. Печать одной строкой: `region=<agreed XX N|disagree XX,YY|insufficient N|stale> sources=<число опрошенных>`.
4. С `--country`: `region_axis(решение, XX)` → ` axis=<verified|blocked|partial|unknown>`.
5. Предыдущее решение лежит в `$XDG_STATE_HOME/cm/region-<экземпляр>.json` (без переменной — `~/.local/state/cm/`). Если оно есть и задан `--country`: `invalidate::on_region_change` → ` invalidation=<keep|region_unknown|region_mismatch>`. Затем файл перезаписывается текущим решением.

Коды выхода: 0 — без `--country` либо ось `verified`; 7 — ось `blocked` (страна выхода не та); 8 — ось `partial` или `unknown`; 2, 3, 4 — как у остальных команд `cm tunnel`.

Команда ничего не перезапускает: смена страны меняет только результат проверки. Вызывать её после смены узла или поколения — дело интерфейса (`I17`).

Контракты, которые проверит роль 2:
- **R03 → AB.Z:** Проверка региона называет исход и не объявляет страну подтверждённой без согласия источников.
  - два источника DE, `--country DE` → "region=agreed DE 2 sources=2 axis=verified", код 0
  - два источника DE, `--country NL` → axis=blocked, код 7
  - источники DE и NL → "region=disagree DE,NL", axis=partial, код 8
  - один источник → "region=insufficient 1", axis=unknown, код 8
  - без --country → строка без axis, код 0
  - второй вызов после смены ответа источников с DE на NL при `--country DE` → "invalidation=region_mismatch"; приложение в туннеле не перезапущено (pid тот же)
  - первый вызов (файла прошлого решения нет) → поля invalidation в строке нет
- **Z06 → AB.Z:** Смена узла меняет результат проверки региона, но не трогает приложение.
  - verify при источниках DE → verified; worker_reload на конфиг с другим узлом, источники отвечают NL → verify: axis=blocked, invalidation=region_mismatch
  - pid приложения до и после совпадает

### I16.Q3 — Считать показатели узла и держать пробы в бюджете.

Файлы: `src/quality/metrics.rs`, `src/quality/mod.rs`. Зависит от: —.

Чистые функции.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample { pub node: String, pub at_unix_ms: i64, pub ok: bool, pub delay_ms: Option<u32> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeStats {
    pub node: String, pub samples: usize, pub loss_percent: u8,
    pub median_delay_ms: Option<u32>, pub jitter_ms: Option<u32>, pub last_ok_unix_ms: Option<i64>,
}
pub const WINDOW: usize = 20;
pub fn stats(samples: &[Sample], node: &str) -> NodeStats;
pub fn to_candidate(stats: &NodeStats, country: Option<String>, protocol: &str) -> Candidate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode { Pinned, Failover, Adaptive }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions { pub metered: bool, pub on_battery: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget { pub min_interval_ms: i64, pub max_per_hour: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeDecision { Allow, TooSoon, HourlyBudget }

pub fn budget(mode: SelectionMode, conditions: Conditions) -> Budget;
pub fn allow_probe(history_unix_ms: &[i64], now_unix_ms: i64, budget: Budget) -> ProbeDecision;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceSample { pub rss_kib: u64, pub cpu_ticks: u64 }
pub fn parse_resources(stat: &str, statm: &str, page_kib: u64) -> Option<ResourceSample>;
```

`stats`: последние `WINDOW` проб узла по времени.
- `loss_percent` — доля неудачных, округление вниз; проб нет → 100;
- `median_delay_ms` — медиана задержек удачных проб; при чётном числе — нижняя из двух средних;
- `jitter_ms` — медиана модулей отклонения от медианы (то же правило для чётного числа);
- удачных проб нет → обе величины `None`.

`to_candidate`: `delay_ms = median_delay_ms`, `loss_percent`, `last_ok_unix_ms` — из `stats`.

`budget`:

| Режим | `min_interval_ms` | `max_per_hour` |
|---|---|---|
| `Pinned` | 300 000 | 12 |
| `Failover` | 60 000 | 60 |
| `Adaptive` | 30 000 | 120 |

Лимитная сеть или батарея (хотя бы одно): интервал ×4, число в час ÷4.

`allow_probe`: последняя проба моложе `min_interval_ms` → `TooSoon`; проб за последние 3 600 000 мс не меньше `max_per_hour` → `HourlyBudget`; иначе `Allow`. Проверки в этом порядке.

`parse_resources`: `/proc/<pid>/stat` — сумма полей 14 и 15 (`utime + stime`); имя процесса в скобках может содержать пробелы и скобки, разбор идёт от последней `)`. `/proc/<pid>/statm` — второе поле, умноженное на `page_kib`.

Числа бюджета — исходные значения, а не результат измерений: пилот на настоящих узлах (`I16.T03.a`) в пакет не входит.

Контракты, которые проверит роль 2:
- **Q01 → I16.Q4:** Показатели узла считаются по последним пробам; число проб ограничено бюджетом.
  - задержки удачных проб [100, 120, 110, 300], одна неудачная → samples 5, loss_percent 20, median_delay_ms 110, jitter_ms 10
  - проб нет → loss_percent 100, median None, jitter None
  - 25 проб → учитываются последние 20 по времени
  - все пробы неудачные → loss_percent 100, median None, last_ok None
  - budget(Pinned, обычные) == {300000, 12}; (Failover) == {60000, 60}; (Adaptive) == {30000, 120}
  - budget(Adaptive, metered) == {120000, 30}; (Adaptive, metered и on_battery) == {120000, 30}
  - allow_probe: последняя проба 29 999 мс назад при Adaptive → TooSoon; 30 000 мс назад → Allow
  - 120 проб за последний час при Adaptive → HourlyBudget; проба ровно 3 600 000 мс назад в счёт не идёт
  - история пуста → Allow
  - parse_resources("12 (a b) c) S 1 12 12 0 -1 4194560 100 0 0 0 7 5 0 0 20 0 1 0 100 0 0", "100 250 30 1 0 20 0", 4) → cpu_ticks 12, rss_kib 1000; строка без `)` → None
  - to_candidate переносит median_delay_ms в delay_ms и loss_percent без изменений

### I16.Q4 — Выбирать узел с учётом режима, живых сессий и неудачных переключений.

Файлы: `src/quality/select.rs`, `src/quality/mod.rs`. Зависит от: I16.Q3.

Чистая функция поверх существующих `score::evaluate` и `switch::decide`.

```rust
pub struct SelectInput<'a> {
    pub mode: SelectionMode,
    pub current: Option<&'a str>,
    pub outcome: &'a Outcome,                 // ranked + rejected из score::evaluate
    pub live_sessions: usize,
    pub last_switch_unix_ms: i64,
    pub last_failed_switch_unix_ms: Option<i64>,
    pub now_unix_ms: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepReason { Pinned, CurrentUsable, SessionsPinned, Cooldown, AlreadyBest, GainTooSmall, DwellNotElapsed }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchReason { NoCurrent, CurrentRejected, BetterNode }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason { NoCandidates, PinnedNodeRejected }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice { Keep(KeepReason), Switch { node: String, reason: SwitchReason }, Block(BlockReason) }

pub const FAILED_SWITCH_COOLDOWN_MS: i64 = 300_000;
pub fn choose(input: &SelectInput<'_>) -> Choice;
pub fn reason_token(choice: &Choice) -> &'static str;
```

«Текущий годен» — `current` есть в `outcome.ranked`. Проверки по порядку:

1. `mode == Pinned`: текущий годен → `Keep(Pinned)`; иначе → `Block(PinnedNodeRejected)`. Закреплённый узел не заменяется никогда.
2. `outcome.ranked` пуст → `Block(NoCandidates)`. Ограничения не ослабляются.
3. `current == None` → `Switch { лучший, NoCurrent }`.
4. Текущий не годен → `Switch { лучший, CurrentRejected }`.
5. `mode == Failover` → `Keep(CurrentUsable)`.
6. Дальше — `Adaptive`, текущий годен:
   - `live_sessions > 0` → `Keep(SessionsPinned)`: живые соединения на другой узел не переносятся;
   - неудачное переключение было меньше `FAILED_SWITCH_COOLDOWN_MS` назад → `Keep(Cooldown)`;
   - иначе `switch::decide(current, ranked, last_switch_unix_ms, now_unix_ms)`: `To { node }` → `Switch { node, BetterNode }`; `Stay` с причиной `AlreadyBest`, `GainTooSmall`, `DwellNotElapsed` → `Keep` с той же причиной.

`reason_token` — имя причины в `snake_case` (`sessions_pinned`, `current_rejected`, `no_candidates`, …): по нему интерфейс объясняет каждое решение.

Контракты, которые проверит роль 2:
- **Q02 → AB.Z:** Каждое решение о выборе узла имеет причину; узел не дёргается и не меняется под живыми сессиями.
  - Pinned, текущий годен → Keep(Pinned); Pinned, текущий отклонён, есть лучшие → Block(PinnedNodeRejected)
  - ranked пуст (все отклонены ограничением страны) → Block(NoCandidates): ограничение не ослаблено
  - current None → Switch{лучший, NoCurrent}
  - Failover, текущий годен, есть узел вдвое быстрее → Keep(CurrentUsable); текущий отклонён → Switch{лучший, CurrentRejected}
  - Adaptive, live_sessions 1, есть узел вдвое быстрее → Keep(SessionsPinned)
  - Adaptive, сессий нет, неудачное переключение 299 999 мс назад → Keep(Cooldown); 300 000 мс назад → решение switch::decide
  - Adaptive, выигрыш 19% → Keep(GainTooSmall); 20% и срок выдержан → Switch{…, BetterNode}
  - reason_token: sessions_pinned, current_rejected, no_candidates, pinned_node_rejected, gain_too_small
  - свойство (10 000 случайных рядов, два узла с оценками в пределах ±10% друг от друга, шаг 30 с, 1 час) → переключений 0
  - свойство (случайные ряды, любой режим): два Switch подряд разделяет не меньше MIN_DWELL_MS, кроме случая CurrentRejected
  - лимитная сеть и батарея: allow_probe по бюджету ×4 отказывает раньше; смена страны узла на запрещённую делает его отклонённым → CurrentRejected или Block
- **Z08 → AB.Z:** Проверки деградации сведены в один отчёт с причиной каждого решения.
  - узел перестал отвечать (10 неудачных проб подряд) → TooLossy → Adaptive: Switch{…, CurrentRejected}; Pinned: Block(PinnedNodeRejected)
  - узел «мерцает» (чередование удачных и неудачных проб) → loss_percent 50 → отклонён; за час переключений не больше одного
  - все узлы недоступны → Block(NoCandidates)
  - для каждого сценария reason_token не пуст и входит в перечисленный в вершине набор

## Сдача

Отчёт:
- по каждой вершине: сделано или открыто с причиной;
- `git diff --stat` по существующим файлам;
- EXIT и счётчики gate;
- список мест, где спецификация допускала толкование, и выбранное толкование.

## Что закрывает пакет в общем плане

| Вершина | Закрывает подзадачи |
|---|---|
| `I07.H1` | I07.T01.a |
| `I07.H2` | I07.T02.a |
| `I07.H3` | I07.T01.a, I07.T02.a |
| `I07.H4` | I07.T01.a, I07.T01.b |
| `I07.H5` | I07.T02.a |
| `I07.H6` | I07.T03.a, I07.T04.a |
| `I09.F0` | I09.T03.a |
| `I07.H7` | I07.T03.a, I07.T04.a |
| `I07.H9` | I07.T03.a |
| `I08.N9` | I10.T01.a |
| `I10.D1` | I10.T01.a, I10.T03.a, I10.T04.a |
| `I09.F1` | I09.T03.a |
| `I09.F2` | I09.T04.a |
| `I09.F3` | I09.T03.a, I09.T04.a, I07.T04.a, I07.T05.a |
| `I11.A8` | I11.T04.a |
| `I12.A10` | I12.T01.a |
| `I15.E1` | I15.T02.a |
| `I12.A13` | I12.T03.a |
| `I15.E2` | I15.T02.a, I12.T03.a |
| `I14.R6` | I14.T03.a |
| `I12.A11` | I12.T01.a |
| `I12.A12` | I12.T05.a |
| `I13.P1` | I13.T01.a, I13.T03.a |
| `I13.P2` | I13.T01.a, I13.T03.a, I13.T04.a |
| `I11.A9` | I11.T04.a, I13.T04.a, I14.T03.a |
| `I14.R4` | I14.T01.a |
| `I14.R5` | I14.T01.a |
| `I14.R7` | I14.T05.a, I14.T01.a |
| `I16.Q3` | I16.T02.a |
| `I16.Q4` | I16.T04.a, I16.T05.a |
| `AB.Z` | I07.T05.a, I08.T05.a, I09.T05.a, I10.T02.a, I10.T05.a, I11.T05.a, I12.T05.a, I13.T02.a, I14.T04.a, I16.T05.a |

Всего подзадач общего плана: 32. Подзадача закрывается, когда приняты все её вершины.

Остаются открытыми после пакета: `I05.T02.a` (измерено, путь данных не выбран), `I05.T02.b`, `I05.T05.a`, `I10.T02.b`, `I13.T05.a`, `I15-R.T04.a`, `I15-R.T05.a`, `I15.T01.a`, `I15.T05.a`, `I16.T03.a` — причины в разделе «Что в пакет не вошло».

