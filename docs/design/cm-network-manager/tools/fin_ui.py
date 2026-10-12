"""FIN, часть 2: интерфейс (направление C) — обзор контроллера, описание туннелей и приложений командами,
живая модель состояния, страница TUI, апплет, диагностический экспорт. Этап 8 общего плана (I17).
"""

DIRECTIONS = [
    ("C", "Интерфейс и наблюдаемость", "Пользователь заводит туннели и приложения командами, видит их настоящее состояние по осям и управляет ими из TUI и апплета."),
]

V = [
    dict(id="C.O1", direction="C", title="Отдавать клиенту одним ответом состояние хоста и всех его туннелей.", size="M", level="L1",
         deps=[], files=["src/controller/protocol.rs", "src/controller/dispatch.rs", "src/controller/core_ops.rs", "src/controller/watch.rs"],
         covers=["I17.T01.a"], external=[],
         spec="""\
Сейчас клиент может спросить состояние только экземпляра с известным именем. Списка нет, поколения сети не видно.

```rust
// protocol.rs
Op::Overview                                   // класс Status, instance() → None
ReplyData::Overview { host: HostRow, tunnels: Vec<TunnelRow> }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRow {
    pub mode: String,                 // off | proxy | tunnel
    pub generation: Option<u64>,
    pub uplinks: Vec<String>,
    pub state: String,                // как в tunnel_state; при mode off — "stopped"
    pub restarts: u32,
    pub core_pid: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRow {
    pub instance: String,
    pub state: String,                // stopped | starting | up | degraded | blocked | failed
    pub restarts: u32,
    pub holders: u32,
    pub worker_generation: Option<u64>,
    pub net_generation: Option<u64>,
    pub netns: Option<String>,        // "cm-<i>", если у экземпляра есть аренда сети
    pub core_pid: Option<u32>,
}

pub const MAX_OVERVIEW_ROWS: usize = 64;
```

- Владелец — uid сокета; чужие экземпляры в ответ не попадают.
- Экземпляры — имена каталогов `<корень владельца>/instances`, которые проходят `InstanceId::new`, кроме `host`; по возрастанию имени; первые `MAX_OVERVIEW_ROWS`.
- `state`, `restarts`, `holders` — из сторожа, как в `tunnel_state` (нет записи → `stopped`, 0, 0).
- `worker_generation` — запись поколения экземпляра; `net_generation` — запись `net:<экземпляр>`.
- `core_pid` — pid ядра, если запись ядра есть и процесс жив.
- `host`: режим, поколение и внешние интерфейсы — как в `host_status` (чужой владелец видит `off`); `state`, `restarts`, `core_pid` — экземпляра `host` этого владельца.
- Проверка прав — как у `worker_status`."""),

    dict(id="C.S1", direction="C", title="Держать хранилище туннелей приложений у пользователя, а не в системном каталоге.", size="S", level="L1",
         deps=[], files=["src/sources/cli.rs", "src/i18n_table.rs"], covers=["I12.T01.a"], external=[],
         spec="""\
Сейчас путь хранилища один — `/var/lib/cm/profiles`, каталог с правами 0700 того, кто его создал (root). Команды `cm tunnel`, `cm app`, TUI и апплет работают от пользователя и открыть его не могут.

```rust
pub fn store_root_for(euid: u32, env: &dyn Fn(&str) -> Option<String>, state_dir: &str) -> PathBuf;
pub fn store_root() -> PathBuf;                       // store_root_for с настоящими значениями
pub fn store_writes_allowed(euid: u32, system_flag: bool) -> bool;
```

`store_root_for`, первое подходящее:
1. `CM_PROFILE_STORE` не пуст → он;
2. `euid == 0` → `<state_dir>/profiles` (как сейчас);
3. `XDG_DATA_HOME` не пуст → `<он>/cm/profiles`;
4. `HOME` не пуст → `<он>/.local/share/cm/profiles`;
5. иначе `.local/share/cm/profiles`.

`store_writes_allowed`: `euid != 0` → да (пользователь пишет в своё хранилище); `euid == 0` → значение системного признака `vpn_store_source` (как сейчас).

Следствия:
- `cm source import` без `--dry-run` от пользователя пишет источник в его хранилище; `--from-legacy` остаётся только для root;
- прежний VPN хоста (root) по-прежнему читает системное хранилище;
- туннель хоста новой схемы (`cm host set tunnel --stored`) берёт профиль из хранилища пользователя, который его включает: ядро хоста работает под его uid.

Хранилище создаётся при первой записи теми же функциями (`Store::initialize`): права 0700, владелец — пользователь."""),

    dict(id="C.E1", direction="C", title="Заводить и удалять туннели, приложения, группы и профиль хоста командами.", size="L", level="L1",
         deps=["C.S1"], files=["src/profiles/edit.rs", "src/profiles/mod.rs", "src/app/define.rs", "src/app/mod.rs", "src/app/cli.rs",
                         "src/app/tunnel.rs", "src/net/cli.rs", "src/main.rs", "src/i18n_table.rs"],
         covers=["I12.T01.a", "I11.T01.a"], external=[],
         spec="""\
Сейчас туннель, приложение, группу и профиль подключения в хранилище создают только тесты. Команды `cm tunnel up ID` и `cm app run ID` пользователю не на чем выполнить.

```
cm group define ID --source SRC [--node УЗЕЛ] [--fake-ip]
cm group remove ID
cm group list
cm app define ID --exec /ПУТЬ [--arg A]... [--cwd КАТАЛОГ] [--env ИМЯ=ЗНАЧЕНИЕ]...
                 (--source SRC [--node УЗЕЛ] [--fake-ip] | --group GID)
                 [--zone ПОЯС --locale ЛОКАЛЬ [--lang ТЕГ]...]
cm app remove ID
cm app list
cm host define --source SRC [--node УЗЕЛ]
```

Идентификаторы. `ID` приложения или группы — 1–30 знаков `a-z0-9-`. Производные сущности:

| Сущность | Идентификатор |
|---|---|
| туннель приложения или группы | `t-<ID>` (он же имя экземпляра контроллера) |
| профиль подключения | `p-<ID>`; для хоста — `p-host` |
| профиль данных | `d-<ID>` |
| пресет окружения | `e-<ID>` |
| политика хоста | `host-policy` |

```rust
// src/profiles/edit.rs — чистые функции: снимок → кандидат снимка
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError { BadId, Exists, UnknownSource, UnknownNode, UnknownGroup, UnknownEntity, InUse, BadValue, Model }

pub struct LinkSpec { pub source_id: Id, pub node_id: Option<Id>, pub fake_ip: bool }
pub struct AppSpec {
    pub id: Id, pub executable: String, pub argv: Vec<String>, pub cwd: Option<String>,
    pub environment: Vec<(String, String)>, pub target: AppTarget,
    pub timezone: String, pub locale: String, pub languages: Vec<String>,
}
pub enum AppTarget { Own(LinkSpec), Group(Id) }

pub fn define_group(snapshot: &GraphSnapshot, id: &Id, link: &LinkSpec) -> Result<GraphSnapshot, EditError>;
pub fn define_app(snapshot: &GraphSnapshot, spec: &AppSpec) -> Result<GraphSnapshot, EditError>;
pub fn define_host(snapshot: &GraphSnapshot, link: &LinkSpec) -> Result<GraphSnapshot, EditError>;
pub fn remove_app(snapshot: &GraphSnapshot, id: &Id) -> Result<GraphSnapshot, EditError>;
pub fn remove_group(snapshot: &GraphSnapshot, id: &Id) -> Result<GraphSnapshot, EditError>;
pub fn token(error: EditError) -> &'static str;   // bad_id, exists, unknown_source, unknown_node, unknown_group, unknown_entity, in_use, bad_value, invalid_model
```

Правила:
- Выбор узла: `--node УЗЕЛ` → `NodeSelection::Pinned` на узел с этим идентификатором, принадлежащий источнику (иначе `UnknownNode`); без флага → `NodeSelection::Policy { name: "auto" }` — все текущие узлы источника с автоматическим выбором ядра.
- DNS профиля: IPv6 — `Block`; fake-ip — `Allowed` при `--fake-ip`, иначе `Forbidden`. Ядро — `Mihomo`.
- `define_group`: группа `ID`, туннель `t-<ID>` с владельцем-группой, профиль `p-<ID>`. Одним кандидатом.
- `define_app` с `Own`: приложение, туннель `t-<ID>` с владельцем-приложением, профиль `p-<ID>`, профиль данных `d-<ID>`, пресет `e-<ID>`. С `Group`: приложение, `d-<ID>`, `e-<ID>`; назначение — группа (нет группы → `UnknownGroup`).
- Пресет: `--zone` и `--locale` задаются вместе; без них — пояс `UTC`, локаль `C.UTF-8`, языки пусты. Значения проверяются теми же функциями, что при запуске (`valid_zone_name`, проверки пресета); отказ → `BadValue`.
- Описание запуска проверяется `spec::check` до записи; отказ → `BadValue`.
- Любой из создаваемых идентификаторов уже выдавался (в том числе удалённой сущности) → `Exists`: реестр выданных идентификаторов повторного использования не допускает.
- `define_host`: профиль `p-host` и политика `host-policy` с режимом `Off`, ссылкой на профиль, без туннеля, `autostart = false`. Повторный вызов заменяет источник и выбор узла в `p-host` (единственная команда, которая меняет существующую сущность).
- `remove_app`: удаляет приложение и всё, что создано вместе с ним (`t-`, `p-`, `d-`, `e-`). `remove_group`: группа с приложениями → `InUse`.
- Кандидат обязан проходить `GraphSnapshot::validate` и проверку перехода; отказ → `Model`.
- Команда: читает снимок, строит кандидата, `Store::commit(ревизия, кандидат, [])`. Конфликт ревизии → одна повторная попытка с новым снимком.
- `remove` перед записью спрашивает `overview`: у туннеля задано любое из двух поколений → `[in_use]`, код 5 (сначала `cm tunnel down`). Контроллер недоступен → удаление разрешено.

Вывод:
- `define` и `remove` → одна строка `ok <ID>`; отказ → `[<token>]`, код 2;
- `cm app list` → по строке `id=<ID> tunnel=<t-…> shared=<yes|no> exec=<путь>`;
- `cm group list` → `id=<ID> tunnel=<t-…> apps=<n>`.

Секретов в выводе нет: источник называется идентификатором."""),

    dict(id="C.M2", direction="C", title="Считать нагрузку процессора и память ядра по `/proc`.", size="S", level="L1",
         deps=[], files=["src/status/resources.rs", "src/status.rs"], covers=["I17.T04.a"], external=[],
         spec="""\
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample { pub ticks: u64, pub rss_kb: u64, pub at_ms: i64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage { pub cpu_pct: u32, pub rss_kb: u64 }

pub fn parse_stat(text: &str) -> Option<u64>;                 // utime + stime
pub fn parse_statm(text: &str, page_kb: u64) -> Option<u64>;  // второе поле × page_kb
pub fn sample(proc_root: &Path, pid: u32, now_ms: i64) -> Option<Sample>;
pub fn usage(previous: &Sample, current: &Sample, ticks_per_s: u64) -> Usage;
pub fn usage_text(usage: &Usage) -> String;                   // "cpu 3% · 41 МБ"
```

- `parse_stat`: поля считаются после последней `)` в строке (имя процесса может содержать пробелы и скобки); `utime` и `stime` — 12-е и 13-е поля после неё.
- `usage`: `cpu_pct = Δticks × 100 000 / (ticks_per_s × Δмс)` с округлением вниз; `Δмс ≤ 0` или счётчик уменьшился → 0. Значение может превышать 100 на многоядерной машине.
- `usage_text`: память в мегабайтах с округлением вниз, не меньше 1.
- Процесса нет, файл не читается, формат не тот → `None`. Трафик здесь не считается и между туннелями не суммируется."""),

    dict(id="C.M1", direction="C", title="Собирать живое состояние туннелей по четырём осям из хранилища, контроллера и проверок.", size="L", level="L1",
         deps=["C.O1", "C.M2", "C.E1"],
         files=["src/status/live.rs", "src/status.rs", "src/status/tunnels.rs", "src/env/record.rs", "src/env/mod.rs", "src/app/tunnel.rs",
                "docs/design/cm-network-manager/I17-UI-CONTRACT.md"],
         covers=["I17.T01.a", "I17.T02.a"], external=[],
         spec="""\
```rust
// src/env/record.rs — файл region-<экземпляр>.json
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRecord { pub expected: Option<String>, pub generation: u64, pub decision: RegionDecision, pub at_unix_ms: i64 }

// src/status/live.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerState { Ok, Unavailable, Denied }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action { Up, Down, Verify, Run }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLive { pub mode: String, pub state: String, pub uplinks: Vec<String>, pub defined: bool, pub legacy_vpn: bool, pub core_pid: Option<u32>, pub usage: Option<Usage> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRow {
    pub view: TunnelView,            // существующий тип: оси, возраст, условие, причина
    pub id: String,                  // идентификатор туннеля в хранилище; "" — туннеля в хранилище нет
    pub instance: String,
    pub state: String,
    pub restarts: u32,
    pub generation: Option<u64>,
    pub country: Option<String>,     // страна последней проверки региона
    pub apps: Vec<String>,           // идентификаторы приложений этого туннеля
    pub core_pid: Option<u32>,
    pub usage: Option<Usage>,
    pub actions: Vec<Action>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveView { pub at_unix_ms: i64, pub controller: ControllerState, pub host: HostLive, pub tunnels: Vec<LiveRow> }

pub struct Inputs<'a> {
    pub snapshot: &'a GraphSnapshot,
    pub overview: Option<(&'a HostRow, &'a [TunnelRow])>,
    pub controller: ControllerState,
    pub regions: &'a BTreeMap<String, RegionRecord>,      // по имени экземпляра
    pub sessions: &'a BTreeMap<String, bool>,             // экземпляр → есть файл сессии с отделёнными данными или сокетами
    pub usage: &'a BTreeMap<u32, Usage>,                  // по pid ядра
    pub legacy_vpn: bool,
    pub now_unix_ms: i64,
}
pub fn build(inputs: &Inputs) -> LiveView;   // чистая функция
pub fn gather(previous: Option<&Gathered>) -> Gathered;   // ввод-вывод; Gathered = LiveView + выборки для следующего расчёта нагрузки
```

Строки: все туннели хранилища, кроме туннеля хоста, по возрастанию идентификатора; затем экземпляры из обзора, которых нет в хранилище (`id = ""`). Имя экземпляра туннеля хранилища — его идентификатор.

Оси (`view.axes`, значения `unknown | partial | verified | blocked | error`) и возраст (`view.age_s`):

| Ось | Значение | Возраст |
|---|---|---|
| `net` | `state::net_axis` по состоянию из обзора: up → verified, degraded → partial, blocked → blocked, failed → error, остальное → unknown | 0, если обзор получен; иначе нет |
| `region` | запись региона есть, её `generation` равно `worker_generation`, `expected` задан → `geo::region_axis(decision, expected)`; иначе unknown | `now − at_unix_ms` в секундах; нет записи → нет |
| `state` | есть держатели и запись сессии с отделёнными данными или сокетами → partial; иначе unknown | нет |
| `app` | есть держатели → partial; иначе unknown | 0 при держателях; иначе нет |

Оси `state` и `app` значение `verified` не получают: связь «процесс ↔ сессия» и отсутствие обхода через шину сессии на настоящих приложениях не проверены (решение D-S1).

Контроллер недоступен или отказал → все оси unknown, возраста нет, `actions` пуст, `state = "unknown"`.

Остальные поля `view`:
- `name` — идентификатор туннеля (или имя экземпляра), `host` — режим хоста, `apps` — `shared` для владельца-группы, иначе `separate`, `sessions` — число держателей;
- `condition` — как сейчас в `status::tunnels::build` (эта часть выносится в общую функцию и используется обеими);
- `failure`: состояние failed → `core-failed`; blocked → `core-restarting`; ось региона blocked → `region-mismatch`; иначе нет. Первое подходящее.

`actions`: stopped, failed → `[Up]`; up, degraded → `[Down, Verify, Run]`; starting, blocked → `[Down]`. Для строки без туннеля в хранилище — только `Down` при состояниях, отличных от stopped.

`country` — страна из `RegionDecision::Agreed` записи региона текущего поколения.

`host`: `mode`, `state`, `uplinks` — из обзора; `defined` — в хранилище есть политика хоста с профилем; `legacy_vpn` — активна служба VPN хоста.

`gather`: хранилище (нет хранилища → пустой снимок), `overview`, файлы региона, файлы `config/session-*.json` экземпляров, выборки `/proc` для pid ядер. Ни одна ошибка не прерывает сбор: недоступная часть даёт unknown.

`cm tunnel verify` пишет `RegionRecord` вместо голого решения: `expected` — значение `--country`, `generation` — поколение ядра на момент проверки. Старый файл без этих полей читается как отсутствующий.

`I17-UI-CONTRACT.md` — таблица «поле интерфейса → источник → когда unknown» по этой спецификации; предварительных полей нет."""),

    dict(id="C.C1", direction="C", title="Печатать живое состояние командой для человека и для программ.", size="S", level="L2",
         deps=["C.M1"], files=["src/app/tunnel.rs", "src/i18n_table.rs"], covers=["I17.T02.a"], external=[],
         spec="""\
```
cm tunnel list [--json]
```

Текст, первая строка — хост, далее по строке на туннель:

```
host mode=<m> state=<s> uplinks=<a,b|-> defined=<yes|no> legacy_vpn=<yes|no> core_pid=<n|->
id=<ID|-> instance=<имя> state=<s> net=<v> region=<v> data=<v> app=<v> holders=<n> restarts=<n> generation=<g|-> country=<CC|-> core_pid=<n|->
```

`data` — ось `state` (слово `state` занято состоянием туннеля). `--json` — `LiveView` одной строкой.

Код выхода: 0; 4 — контроллер недоступен (вывод всё равно печатается: туннели хранилища с unknown). Токены не переводятся."""),

    dict(id="C.T1", direction="C", title="Показать туннели в TUI по осям и управлять ими без блокировки интерфейса.", size="L", level="L2",
         deps=["C.M1", "I.S1"], files=["src/status/page.rs", "src/status.rs", "src/tui.rs", "src/tui/tunnels.rs", "src/i18n_table.rs"],
         covers=["I17.T02.a", "I17.T05.a"], external=[],
         spec="""\
Логика и отрисовка страницы — в библиотеке (`src/status/page.rs`), чтобы роль 2 проверяла их без терминала. `src/tui/tunnels.rs` — только фоновый сбор и связь с экраном. Вход — клавиша `m` на экране VPN (сейчас она работает только с макетом): при `CM_TUI_MOCK_TUNNELS=1` открывается макет (`mock_tunnels.rs` остаётся), иначе — настоящая страница. Клавиша добавляется в подсказку экрана VPN.

```rust
// src/status/page.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm { Down { id: String }, HostOn, HostOff }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome { Stay, Back, Message(String), Run(Vec<String>) }

pub struct Page { pub view: LiveView, pub index: usize, pub confirm: Option<Confirm> }
impl Page {
    pub fn new(view: LiveView) -> Self;
    pub fn replace(&mut self, view: LiveView);          // новые данные; выбранная строка сохраняется по instance
    pub fn key(&mut self, key: KeyCode) -> Outcome;
    pub fn keys(&self) -> Vec<(&'static str, &'static str)>;
}
pub fn draw(frame: &mut Frame, area: Rect, page: &Page);

pub struct Poller { /* поток и приёмник канала */ }
impl Poller {
    pub fn start(period_ms: u64, gather: impl FnMut() -> LiveView + Send + 'static) -> Self;
    pub fn latest(&mut self) -> Option<LiveView>;       // последнее готовое значение, без ожидания
}
```

Клавиши вне вопроса:

| Клавиша | Действие |
|---|---|
| `↑` `↓` `k` `j` | выбор строки, без перехода через край |
| `u`, `Enter` | есть `Action::Up` → `Run(["tunnel","up",id])`; иначе `Message` с причиной |
| `x` | есть `Action::Down` → вопрос `Confirm::Down` |
| `v` | есть `Action::Verify` → `Run(["tunnel","verify",id])`, с `"--country", CC`, если `country` известна |
| `h` | режим хоста off и `host.defined` → вопрос `HostOn`; режим tunnel или proxy → вопрос `HostOff`; off без профиля → `Message` «сначала cm host define» |
| `e` | `Run(["diag","export"])` |
| `q`, `Esc`, `Backspace` | `Back` |

Вопрос открыт: `y`, `Enter` — подтверждение; `n`, `Esc` — отмена с сообщением; любая другая клавиша вопрос не закрывает.
- `Down` → `Run(["tunnel","down",id])`. Текст вопроса называет последствие: приложения этого туннеля останутся без сети, VPN хоста этой клавишей не меняется.
- `HostOn` → `Run(["host","set","tunnel","--stored"])`. Текст: весь трафик хоста пойдёт через туннель; при `host.legacy_vpn` — отказ с сообщением «сначала остановите VPN хоста» вместо вопроса.
- `HostOff` → `Run(["host","set","off"])`. Текст: хост выйдет в сеть напрямую, туннели приложений не затронуты.

`Run` исполняется существующим экраном процесса (как команды экрана VPN); после завершения — возврат на страницу.

Отрисовка:
- первая строка — хост: режим, состояние, внешние интерфейсы;
- таблица: имя, состояние, четыре оси (символ и слово; при ширине меньше 80 — только символ), возраст каждой оси, число процессов, нагрузка ядра;
- возраст старше 3600 с помечается `⚠`;
- под таблицей — причина (`failure`) и приложения выбранной строки;
- общего «зелёного» статуса нет: ни одна ячейка не сводит оси в одно слово;
- контроллер недоступен → строка «контроллер недоступен» и туннели хранилища с unknown.
Символы и подписи значений — те же, что в макете; общие функции переносятся в `page.rs`, макет их использует оттуда.

Фоновый сбор — `Poller`: поток вызывает `gather` сразу и далее каждые `period_ms`, кладёт результат в канал ёмкостью 1 (`try_send`) и завершается, когда `Poller` уничтожен (ожидание периода — отрезками не длиннее 100 мс). `latest` забирает всё накопленное и возвращает последнее. `src/tui/tunnels.rs` создаёт `Poller::start(2000, …)` с `live::gather` при входе на страницу и на каждом такте интерфейса передаёт `latest()` в `Page::replace`. Нажатие клавиши никогда не ждёт сбора."""),

    dict(id="C.D1", direction="C", title="Выгружать диагностику без секретов, путей пользователя и аргументов программ.", size="M", level="L1",
         deps=["C.M1"], files=["src/diag/mod.rs", "src/diag/export.rs", "src/diag/redact.rs", "src/lib.rs", "src/main.rs", "src/i18n_table.rs"],
         covers=["I17.T03.a"], external=[],
         spec="""\
```
cm diag export [--out ФАЙЛ|-]
```

```rust
#[derive(Clone, Debug, Serialize)]
pub struct Export {
    pub version: String, pub at_unix_ms: i64, pub live: LiveView,
    pub instances: Vec<InstanceDiag>, pub units: BTreeMap<String, String>,
    pub kernel: BTreeMap<String, String>, pub checks: Vec<String>, pub notes: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct InstanceDiag {
    pub instance: String, pub generations: Vec<u64>, pub policy: Option<TunnelPolicy>,
    pub region: Option<RegionRecord>, pub shape: Option<Shape>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Shape { pub proxies: usize, pub groups: usize, pub rules: usize, pub protocols: BTreeMap<String, usize> }

pub fn shape(document: &serde_json::Value) -> Shape;
pub fn scrub(text: &str, home: &str, user: &str) -> String;
pub fn find_secrets(text: &str) -> Vec<&'static str>;     // имена сработавших правил; пусто — чисто
```

Что входит:
- `live` — как в `cm tunnel list --json`;
- по каждому экземпляру владельца: номера файлов `gen-N.json`, политика, запись региона, **форма** последнего конфига (число узлов, групп, правил, узлов по протоколам). Содержимое конфига не выгружается;
- `units` — состояние служб `cm-controller.socket`, `cm-controller.service`, `cm-reconcile.service` и службы VPN хоста;
- `kernel` — вывод `ip -j rule`, `ip -j -6 rule`, `ip -j route show table 90`, `ip -j route show table 91`, `ip -br link`; запросы без привилегий через `SystemQuery`; ошибка запроса → строка `error`;
- `checks` — строки `cm host check`;
- аргументы, окружение и пути исполняемых файлов процессов не выгружаются — только их число.

`scrub`, правила по порядку:

| Правило | Что заменяется | На что |
|---|---|---|
| `home_path` | префикс `home` | `~` |
| `user_name` | имя пользователя отдельным словом или частью пути | `<user>` |
| `url` | `схема://…` до пробела или кавычки | `<схема>://<redacted>` |
| `key_value` | значение после ключа `password`, `passwd`, `uuid`, `token`, `secret`, `private-key`, `private_key`, `psk`, `auth`, `authorization`, `cookie` (без учёта регистра; разделители `=`, `:`, `": "`) | `<redacted>` |
| `uuid` | 8-4-4-4-12 шестнадцатеричных знаков | `<redacted>` |
| `long_token` | 32 и более знаков из `A-Za-z0-9+/=_-` подряд | `<redacted>` |
| `public_ip` | IPv4 вне `10/8`, `127/8`, `169.254/16`, `172.16/12`, `192.168/16`, `198.18/15`, `100.64/10`; IPv6 вне `fe80::/10`, `fc00::/7`, `::1` | `<ip>` |

`find_secrets` проверяет те же классы, кроме `user_name` и `home_path` при пустых значениях. Разбор — ручной, без новых зависимостей.

Порядок команды: сбор → сериализация → `scrub` → `find_secrets`. Непустой результат → `[export_not_clean]`, код 3, файл не пишется. Иначе файл с правами 0600; по умолчанию `$XDG_STATE_HOME/cm/diag-<секунды>.json` (нет переменной — `~/.local/state`); путь печатается в stdout. `--out -` — документ в stdout.

Коды: 0; 2 — использование или ошибка записи; 3 — экспорт не чист."""),

    dict(id="C.A1", direction="C", title="Показать туннели и нагрузку ядер в апплете.", size="M", level="L3",
         deps=["C.M1"], files=["src/status/panel.rs", "src/status.rs", "cosmic/src/tunnels.rs", "cosmic/src/main.rs", "cosmic/src/applet.rs",
                              "cosmic/src/panel.rs", "cosmic/src/jobs.rs", "cosmic/src/model.rs"],
         covers=["I17.T04.a"], external=[],
         spec="""\
Содержимое строк — в библиотеке, отрисовка — в `cosmic/`.

```rust
// src/status/panel.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRow { pub title: String, pub axes: String, pub detail: String, pub problem: bool }
pub fn rows(view: &LiveView) -> Vec<PanelRow>;
pub fn headline(view: &LiveView) -> String;
```

- `headline`: «туннели: N работают, M с ошибкой» (N — состояния up и degraded; M — blocked и failed); контроллер недоступен → «контроллер недоступен»; туннелей нет → «туннелей нет».
- `rows`: первая — хост (`title` «Хост», `axes` — режим и состояние, `detail` — внешние интерфейсы и нагрузка ядра); далее по строке на туннель:
  - `title` — идентификатор;
  - `axes` — четыре оси символами в порядке net, region, state, app, через пробел;
  - `detail` — «процессов N · cpu X% · Y МБ», причина ошибки в конце, если есть;
  - `problem` — состояние blocked или failed либо любая ось blocked или error.
- Трафик туннелей не складывается и общий итог не показывается.

`cosmic/`:
- вид задания `Kind::Tunnels` в существующей очереди `Jobs`: `cm::status::live::gather` выполняется вне потока интерфейса, период 5 с, пока открыта панель; ошибки — в существующем списке ошибок панели;
- раздел «Туннели» в панели: `headline`, строки, кнопка «Открыть в консоли» — существующий запуск TUI;
- значок панели — существующая функция `tunnel_icons::badge_for` по `view` строк.

Действий, меняющих состояние, в апплете нет: управление — в TUI."""),
]

E = [
    dict(id="CO02", source="C.O1", target="C.M1", level="L1",
         contract="Обзор перечисляет только экземпляры владельца, с состоянием сторожа и без данных запроса.",
         check="`tests/audit_fin_overview.rs`: контроллер с подменными исполнителями.",
         values=["владелец с экземплярами `b`, `a` и `host` → `tunnels` из двух строк в порядке `a`, `b`; `host` в списке нет",
                 "экземпляры другого uid в ответ не попадают; чужой владелец туннеля хоста видит `host.mode = off`",
                 "каталог с именем, не проходящим `InstanceId::new`, пропускается; при 70 экземплярах в ответе 64 строки",
                 "экземпляр без запусков → state stopped, restarts 0, holders 0, оба поколения и netns пусты",
                 "класс операции — Status; подменный `Authorizer` с отказом → `denied`"]),
    dict(id="CS01", source="C.S1", target="C.E1", level="L1",
         contract="Путь хранилища зависит только от uid и окружения, и пользователь не пишет в системное хранилище.",
         check="`tests/audit_fin_define.rs`.",
         values=["`store_root_for(1000, …)` при `HOME=/home/u` без других переменных → `/home/u/.local/share/cm/profiles`",
                 "при `XDG_DATA_HOME=/d` → `/d/cm/profiles`; при `CM_PROFILE_STORE=/x` → `/x` для любого uid",
                 "`store_root_for(0, …)` при `state_dir=/var/lib/cm` и заданном `HOME` → `/var/lib/cm/profiles`",
                 "`store_writes_allowed(1000, false)` → true; `(0, false)` → false; `(0, true)` → true",
                 "`cm app define` во временном `HOME` без `CM_PROFILE_STORE` создаёт `<HOME>/.local/share/cm/profiles` с правами 0700"]),
    dict(id="CE01", source="C.E1", target="C.M1", level="L1",
         contract="Команды описания создают граф, который проходит проверку модели и разрешается назначением.",
         check="`tests/audit_fin_define.rs`: чистые функции `profiles::edit` на снимках теста.",
         values=["`define_app` с собственным туннелем на снимке с источником `s1` → в кандидате приложение `web`, туннель `t-web`, профиль `p-web`, профиль данных `d-web`, пресет `e-web`; `validate()` → Ok; `assign::application` → instance `t-web`, shared false",
                 "`define_group g1`, затем два `define_app` с `Group(g1)` → `assign::application` обоих даёт instance `t-g1`, shared true",
                 "без `--node` → выбор `Policy{name:\"auto\"}`; с узлом другого источника → `Err(UnknownNode)`",
                 "идентификатор длиной 31 → `Err(BadId)`; повтор `define_app web` → `Err(Exists)`; `define_app` после `remove_app` с тем же ID → `Err(Exists)`",
                 "`remove_group` при живом приложении группы → `Err(InUse)`; `remove_app` убирает `t-`, `p-`, `d-`, `e-` и оставляет источник",
                 "`--exec relative/path` → `Err(BadValue)`; пояс `../x` → `Err(BadValue)`; снимок не изменён",
                 "`define_host` дважды с разными источниками → одна политика `host-policy`, профиль `p-host` указывает на второй источник"]),
    dict(id="CE02", source="C.E1", target="I.S1", level="L2",
         contract="Команды пишут хранилище одной записью и не трогают работающий туннель.",
         check="`tests/audit_fin_define.rs` (команды во временном хранилище), `tests/fin_stand.py`, раздел `define`.",
         values=["`cm app define web --exec /usr/bin/true --source s1` → `ok web`, код 0; ревизия хранилища выросла ровно на 1",
                 "`cm app list` → строка `id=web tunnel=t-web shared=no exec=/usr/bin/true`",
                 "неизвестный источник → `[unknown_source]`, код 2; ревизия не изменилась",
                 "стенд: `cm tunnel up t-web`, затем `cm app remove web` → `[in_use]`, код 5; после `cm tunnel down t-web` → `ok web`",
                 "в выводе команд нет адресов узлов и URL источника"]),
    dict(id="CM01", source="C.M2", target="C.M1", level="L1",
         contract="Нагрузка считается по двум выборкам и не падает на необычных строках `/proc`.",
         check="`tests/audit_fin_live.rs`.",
         values=["строка stat с именем процесса `(a b) c)` → поля берутся после последней `)`; utime 150, stime 50 → 200",
                 "выборки (ticks 100, 0 мс) и (ticks 300, 2000 мс) при 100 тиках в секунду → cpu_pct 100",
                 "Δмс = 0 или ticks уменьшились → cpu_pct 0",
                 "statm `1000 2560 …` при странице 4 КБ → rss_kb 10240; `usage_text` → `cpu 0% · 10 МБ`",
                 "нет каталога процесса → `sample` возвращает None"]),
    dict(id="CM02", source="C.M1", target="C.C1", level="L1",
         contract="Каждая ось получает значение только из своего источника; без источника — unknown.",
         check="`tests/audit_fin_live.rs`: чистая функция `live::build`.",
         values=["state up, записи региона нет, держателей нет → net verified, region unknown, state unknown, app unknown; actions [Down, Verify, Run]",
                 "запись региона Agreed DE с expected DE и поколением, равным поколению ядра → region verified, country DE; возраст = (now − at)/1000",
                 "та же запись с другим поколением → region unknown, country пусто",
                 "expected US при Agreed DE → region blocked, failure region-mismatch, condition blocked",
                 "state failed → net error, failure core-failed, actions [Up]; state blocked → failure core-restarting, actions [Down]",
                 "2 держателя и запись сессии → state partial, app partial, sessions 2; ни в одном наборе входов `state` и `app` не равны verified",
                 "контроллер Unavailable → все оси unknown, возраста нет, actions пуст, строки туннелей хранилища присутствуют",
                 "экземпляр из обзора без туннеля в хранилище → строка с id \"\" после строк хранилища"]),
    dict(id="CM03", source="C.M1", target="C.T1", level="L1",
         contract="Клавиши страницы дают только разрешённые действия; остановка и режим хоста — через вопрос.",
         check="`tests/audit_fin_page.rs`: `status::page::Page` на собранных `LiveView`.",
         values=["строка stopped: `u` → `Run([\"tunnel\",\"up\",\"t-web\"])`; `x` → `Message`, вопрос не открыт",
                 "строка up: `x` → открыт `Confirm::Down`; `Tab` → вопрос остаётся; `n` → `Message`, вопрос закрыт; `x`, `y` → `Run([\"tunnel\",\"down\",\"t-web\"])`",
                 "строка up со страной DE: `v` → `Run([\"tunnel\",\"verify\",\"t-web\",\"--country\",\"DE\"])`",
                 "хост off без профиля: `h` → `Message`; с профилем и `legacy_vpn` → `Message`, вопрос не открыт; с профилем без него → `HostOn`, `Enter` → `Run([\"host\",\"set\",\"tunnel\",\"--stored\"])`",
                 "хост tunnel: `h`, `y` → `Run([\"host\",\"set\",\"off\"])`",
                 "`replace` с новым порядком строк сохраняет выбор по instance; `↓` на последней строке выбор не меняет",
                 "`q`, `Esc`, `Backspace` вне вопроса → `Back`; `Esc` в вопросе → отмена, не `Back`"]),
    dict(id="CM04", source="C.M1", target="C.D1", level="L1",
         contract="Очистка убирает каждый класс секретов, а выгрузка с остатком не пишется.",
         check="`tests/audit_fin_diag.rs`.",
         values=["`scrub` строки с `/home/alice/x`, `alice`, `vless://id@host:443?x=1`, `password=abc`, uuid, 40-символьным токеном, `8.8.4.4` при home `/home/alice` и user `alice` → `find_secrets` результата пуст; исходных значений в результате нет",
                 "адреса `10.213.0.1`, `198.18.64.2`, `192.168.1.1`, `fe80::1` остаются как есть",
                 "`find_secrets` на исходной строке возвращает `url`, `key_value`, `uuid`, `long_token`, `public_ip`",
                 "`shape` документа с 3 узлами (2 vless, 1 trojan), 1 группой, 5 правилами → `{proxies:3, groups:1, rules:5, protocols:{trojan:1, vless:2}}`",
                 "свойство, 10 000 случаев: для случайной строки из алфавита секретов `find_secrets(scrub(s))` пуст"]),
    dict(id="CM05", source="C.M1", target="C.A1", level="L1",
         contract="Строки апплета повторяют оси живой модели и не складывают туннели в один статус.",
         check="`tests/audit_fin_page.rs`: `status::panel`.",
         values=["два туннеля up и один failed → headline «туннели: 2 работают, 1 с ошибкой»",
                 "контроллер Unavailable → headline «контроллер недоступен»; пустой список → «туннелей нет»",
                 "строка туннеля: axes — четыре символа через пробел в порядке net, region, state, app; problem true при failed и при оси region blocked",
                 "первая строка — хост; при режиме off её axes содержит «выкл»"]),
    dict(id="CC01", source="C.C1", target="X.Q4", level="L2",
         contract="Вывод `cm tunnel list` разбирается построчно и одинаков по содержанию с JSON.",
         check="`tests/fin_stand.py`, раздел `list`.",
         values=["до запуска: строка `host mode=off state=stopped uplinks=- defined=no legacy_vpn=no core_pid=-`; строка туннеля оканчивается на `state=stopped net=unknown region=unknown data=unknown app=unknown holders=0 restarts=0 generation=- country=- core_pid=-`",
                 "после `cm tunnel up t-web`: `state=up net=verified`, `generation=1`, `core_pid` — pid живого процесса ядра",
                 "после `cm tunnel verify t-web --country ZZ` с источниками стенда, отвечающими DE → `region=blocked`",
                 "`--json` разбирается в объект с ключами `at_unix_ms`, `controller`, `host`, `tunnels`; число туннелей равно числу строк текста минус одна",
                 "контроллер остановлен → код 4, строки туннелей хранилища напечатаны с unknown"]),
    dict(id="CC02", source="C.C1", target="FIN.Z", level="L2",
         contract="Состояние в выводе меняется вслед за событиями стенда не позже одного прохода сторожа.",
         check="`tests/fin_stand.py`, раздел `list`.",
         values=["`kill -9` ядра → не позже чем через 3 с `state=blocked net=blocked`; не позже чем через 8 с снова `state=up`, `restarts=1`",
                 "процесс в сети туннеля запущен → не позже чем через 3 с `holders=1 app=partial`; завершён → `holders=0 app=unknown`",
                 "`cm host set tunnel --stored` → строка хоста `mode=tunnel state=up`"]),
    dict(id="CT01", source="C.T1", target="FIN.Z", level="L2",
         contract="Страница рисуется на узком и широком терминале, на двух языках, и нигде не объявляет общий «зелёный» статус.",
         check="`tests/audit_fin_page.rs`: `ratatui::backend::TestBackend`, состояния из `tests/fixtures/i17/states.json` и из `live::build`.",
         values=["размеры 60×20, 80×24, 120×40: отрисовка без паники; при 60 колонках оси — одним символом, при 120 — символ и слово",
                 "в тексте экрана есть четыре заголовка осей; нет слов «защищено», «безопасно», «анонимно», «ok» как итога строки",
                 "возраст 3601 с → в ячейке есть `⚠`; возраст 59 с → нет",
                 "языки ru и en: подписи значений осей разные, символы одинаковые; токены команд не переведены",
                 "открытый вопрос остановки содержит имя туннеля и слова о том, что VPN хоста не меняется",
                 "состояние `контроллер недоступен` — отдельная строка; строки туннелей при этом есть",
                 "`Poller` с функцией сбора, которая спит 3 с: 100 вызовов `latest` и 100 вызовов `Page::key` подряд завершаются быстрее 100 мс; первое значение приходит не позже чем через 4 с; после уничтожения `Poller` функция сбора больше не вызывается (счётчик вызовов не растёт 3 с)"]),
    dict(id="CD01", source="C.D1", target="X.Q0", level="L2",
         contract="Выгрузка содержит состояние, достаточное для разбора, и ни одного значения из конфигов и запросов.",
         check="`tests/fin_stand.py`, раздел `diag`: источник стенда с узлами, у которых известны пароль и uuid.",
         values=["`cm diag export --out -` → код 0; JSON с ключами `version`, `live`, `instances`, `units`, `kernel`, `checks`",
                 "в выводе нет пароля, uuid и адреса узла стенда, нет пути домашнего каталога и имени пользователя",
                 "`instances[].shape.proxies` равно числу узлов источника; `generations` перечисляет файлы поколений",
                 "файл по умолчанию создан с правами 0600, путь напечатан",
                 "`kernel` содержит ключи для пяти запросов; при включённом туннеле хоста в `ip -j rule` есть приоритет 2000"]),
    dict(id="CD02", source="C.D1", target="FIN.Z", level="L1",
         contract="Выгрузка, в которой после очистки найден секрет, не записывается.",
         check="`tests/audit_fin_diag.rs`.",
         values=["экспорт, в чьё поле `notes` тест подставил строку с uuid в обход `scrub` → запись отклонена, результат `export_not_clean`, файла нет",
                 "`cm diag export --out /нет/каталога/x` → код 2",
                 "в структуре `Export` нет полей с аргументами и окружением процессов (сериализованный документ не содержит ключей `argv`, `args`, `env`, `cmdline`)"]),
    dict(id="CA01", source="C.A1", target="FIN.Z", level="L3",
         contract="Апплет собирается, не блокирует панель сбором и не содержит действий над туннелями.",
         check="gate (сборка и тесты `cosmic/`); ручная проверка — шаг `hq-apps`, раздел `applet`.",
         values=["`cargo build` и `cargo test` в `cosmic/` с `--locked --offline` → EXIT 0",
                 "в `cosmic/src/tunnels.rs` нет вызовов `cm host set`, `cm tunnel up|down`, `controller::client::call` с операциями, кроме `Overview` (проверка поиском по исходнику)",
                 "`live::gather` вызывается только из задания `Kind::Tunnels` (поиск по `cosmic/src`: одно место вызова)"]),
]
