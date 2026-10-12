"""Пакет B: приложения и группы (I11, I12), упаковки (I13), регион (I14), приватное окружение (I15), качество (I16).

Данные для ab_dag.py. Формат — как в pack_apps.py.
"""

DIRECTIONS = [
    ("A", "Приложения и туннели", "Туннель поднимается по описанию из хранилища; запуск замечает чужой экземпляр и передачу управления; у каждой сессии своё окружение."),
    ("P", "Упаковки", "Для каждого способа запуска известно, какой процесс выходит в сеть, и что CM о нём может обещать."),
    ("R", "Регион", "Страна выхода измеряется через сам туннель; часовой пояс применяется к процессу, хост не меняется."),
    ("E", "Приватное окружение", "Каталоги данных и сокеты сессии отделяются по профилю данных."),
    ("Q", "Качество", "Пробы укладываются в бюджет; узел не меняется под живыми сессиями."),
]

V = []
E = []


def vertex(vid, direction, title, size, level, deps, files, spec, external=(), covers=()):
    V.append(dict(id=vid, direction=direction, title=title, size=size, level=level, deps=deps,
                  files=files, spec=spec, external=list(external), covers=list(covers)))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


# ───────────────────────── A: приложения и туннели ─────────────────────────
vertex("I11.A8", "A", "Находить процессы в сети туннеля и экземпляры программы вне её.", "M", "L1", [],
       ["src/app/instance.rs", "src/app/mod.rs", "src/controller/protocol.rs", "src/controller/app_ops.rs", "src/controller/dispatch.rs"],
       """\
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
Контроллер: сети у экземпляра нет → `not_running`. Иначе `netns` — имя `cm-<i>`, `pids` — `holders(scan("/proc"), netns_of_path(<файл netns>), peer.uid)`, не больше 256.""",
       covers=["I11.T04.a"])

vertex("I12.A10", "A", "Разрешить назначение приложения в туннель, профиль подключения и источник.", "M", "L1", [],
       ["src/app/assign.rs", "src/app/mod.rs"],
       """\
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

У каждого приложения свой профиль подключения и свой источник: два приложения могут одновременно работать через разные подписки.""",
       covers=["I12.T01.a"])

vertex("I15.E1", "E", "Вывести каталоги данных и каталог сокетов сессии из профиля данных.", "S", "L1", [],
       ["src/env/private.rs", "src/env/mod.rs"],
       """\
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

`check_dir` (для контроллера): `lstat`; настоящий каталог, владелец `uid`, `mode & 0o077 == 0`.""",
       covers=["I15.T02.a"])

vertex("I12.A13", "A", "Дать каждой сессии приложения своё окружение.", "M", "L2", ["I15.E1"],
       ["src/app/env.rs", "src/app/mod.rs", "src/controller/protocol.rs", "src/controller/codec.rs", "src/controller/app_ops.rs"],
       """\
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

Поля `data` и `ipc` применяет `I15.E2`.""",
       covers=["I12.T03.a"])

vertex("I15.E2", "E", "Применить каталоги данных и политику сокетов сессии при запуске.", "M", "L2", ["I15.E1", "I12.A13"],
       ["src/controller/app_ops.rs"],
       """\
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

Порядок наложения переменных не меняется: сначала переменные сессии клиента, затем то, что ставит контроллер; значения контроллера перекрывают клиентские.""",
       covers=["I15.T02.a", "I12.T03.a"])

vertex("I14.R6", "R", "Подставить процессу файл часового пояса региона.", "M", "L2", ["I12.A13"],
       ["src/controller/drop.rs", "src/controller/app_ops.rs"],
       """\
Одной переменной `TZ` мало: программа без неё читает `/etc/localtime` хоста.

`drop.rs`: новая функция `drop_into_netns_region_pre_exec(command, netns, resolv, localtime: Option<&Path>, run_as, keep_fds, limits)`. Прежняя `drop_into_netns_pre_exec` остаётся и вызывает новую с `None`. В потомке, в его собственном mount namespace, сразу после привязки `resolv.conf`: `mount(<localtime>, "/etc/localtime", MS_BIND)`. Отказ `mount` — отказ запуска, как для `resolv.conf`. Путь с NUL → `bad_argument` до запуска.

`app_ops`: если у запуска есть пояс (из файла сессии или `env.json`):
- файл пояса — `<tzdir>/<timezone>`, `tzdir = /usr/share/zoneinfo`;
- это не обычный файл → `invalid_config`, процесс не создан;
- иначе путь передаётся в `drop_into_netns_region_pre_exec`.

Нет пояса — `localtime = None`, поведение прежнее.

`/etc/localtime` хоста обычно ссылка: привязка ложится на файл, на который она указывает, и видна только в mount namespace этого приложения. Хост не меняется.""",
       covers=["I14.T03.a"])

vertex("I12.A11", "A", "Поднимать и снимать туннель по описанию из хранилища командой `cm tunnel`.", "L", "L2", ["I12.A10"],
       ["src/app/tunnel.rs", "src/app/mod.rs", "src/core/mihomo/config.rs", "src/controller/client.rs", "src/main.rs", "src/i18n_table.rs"],
       """\
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

Запись состояния туннеля и сессий обратно в хранилище в эту вершину не входит.""",
       external=["I08.N9", "I09.F3"], covers=["I12.T01.a"])

vertex("I12.A12", "A", "Останавливать туннель, когда его покинул последний процесс.", "M", "L2", ["I11.A8"],
       ["src/controller/watch.rs", "src/controller/app_ops.rs"],
       """\
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

Запуск приложения во время автоматической остановки получает `busy` или `not_running`, но не попадает в полуснятую сеть: остановка идёт под той же блокировкой экземпляра, что операции клиента.""",
       external=["I09.F3", "I08.N9"], covers=["I12.T05.a"])

# ───────────────────────── P: упаковки ─────────────────────────
vertex("I13.P1", "P", "Определить способ упаковки и запуска программы.", "S", "L1", [],
       ["src/app/packaging/mod.rs", "src/app/packaging/detect.rs", "src/app/mod.rs"],
       """\
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
| `head` начинается с `\\x7fELF`, а байты 8..11 — `41 49 02` или `41 49 01` | `AppImage` |
| иначе | `Native` |""",
       covers=["I13.T01.a", "I13.T03.a"])

vertex("I13.P2", "P", "Сказать, что CM обещает для каждого способа запуска.", "S", "L1", ["I13.P1", "I15.E1"],
       ["src/app/packaging/policy.rs", "src/app/packaging/mod.rs"],
       """\
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

Матрица — это и есть ответ на `I13.T01.a`: для каждой ячейки названы исполнитель и статус. Проверка на настоящих Flatpak и AppImage (`I13.T05.a`) в пакет не входит.""",
       covers=["I13.T01.a", "I13.T03.a", "I13.T04.a"])

vertex("I11.A9", "A", "Запускать приложение с проверками упаковки, региона, чужого экземпляра и передачи управления.", "L", "L2",
       ["I11.A8", "I13.P2", "I15.E2", "I12.A10"],
       ["src/app/cli.rs", "src/i18n_table.rs"],
       """\
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

Поиск чужого экземпляра выполняет клиент, а не контроллер: это защита пользователя от собственной ошибки, а не граница прав. По пути исполняемого файла он не различает программы на общем интерпретаторе — для них есть `--allow-running`.""",
       external=["I14.R2"], covers=["I11.T04.a", "I13.T04.a", "I14.T03.a"])

# ───────────────────────── R: регион ─────────────────────────
vertex("I14.R4", "R", "Описать источники страны выхода и разобрать их ответы.", "S", "L1", [],
       ["src/env/sources.rs", "src/env/mod.rs"],
       """\
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

Каждый источник — третья сторона, которая видит адрес выхода: их число — цена проверки. Список передаётся параметром, значения по умолчанию можно заменить.""",
       covers=["I14.T01.a"])

vertex("I14.R5", "R", "Собрать наблюдения страны запросами из сети самого туннеля.", "M", "L2", ["I14.R4"],
       ["src/env/collect.rs", "src/env/mod.rs", "src/main.rs"],
       """\
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

Запрос идёт тем же путём, что трафик приложения: TUN ядра, его DNS, его узел.""",
       external=["I11.A3"], covers=["I14.T01.a"])

vertex("I14.R7", "R", "Проверять регион туннеля и замечать его смену.", "M", "L2", ["I14.R5", "I12.A11"],
       ["src/app/tunnel.rs", "src/i18n_table.rs"],
       """\
```
cm tunnel verify ID [--country XX] [--source id=формат=url …]
```

1. `assign::tunnel` → экземпляр. Источники — из флагов, без флагов — `default_sources()`.
2. `collect(…, wait_ms = 15_000)` → `env::geo::decide(наблюдения, сейчас)`.
3. Печать одной строкой: `region=<agreed XX N|disagree XX,YY|insufficient N|stale> sources=<число опрошенных>`.
4. С `--country`: `region_axis(решение, XX)` → ` axis=<verified|blocked|partial|unknown>`.
5. Предыдущее решение лежит в `$XDG_STATE_HOME/cm/region-<экземпляр>.json` (без переменной — `~/.local/state/cm/`). Если оно есть и задан `--country`: `invalidate::on_region_change` → ` invalidation=<keep|region_unknown|region_mismatch>`. Затем файл перезаписывается текущим решением.

Коды выхода: 0 — без `--country` либо ось `verified`; 7 — ось `blocked` (страна выхода не та); 8 — ось `partial` или `unknown`; 2, 3, 4 — как у остальных команд `cm tunnel`.

Команда ничего не перезапускает: смена страны меняет только результат проверки. Вызывать её после смены узла или поколения — дело интерфейса (`I17`).""",
       covers=["I14.T05.a", "I14.T01.a"])

# ───────────────────────── Q: качество ─────────────────────────
vertex("I16.Q3", "Q", "Считать показатели узла и держать пробы в бюджете.", "M", "L1", [],
       ["src/quality/metrics.rs", "src/quality/mod.rs"],
       """\
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

Числа бюджета — исходные значения, а не результат измерений: пилот на настоящих узлах (`I16.T03.a`) в пакет не входит.""",
       covers=["I16.T02.a"])

vertex("I16.Q4", "Q", "Выбирать узел с учётом режима, живых сессий и неудачных переключений.", "M", "L1", ["I16.Q3"],
       ["src/quality/select.rs", "src/quality/mod.rs"],
       """\
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

`reason_token` — имя причины в `snake_case` (`sessions_pinned`, `current_rejected`, `no_candidates`, …): по нему интерфейс объясняет каждое решение.""",
       covers=["I16.T04.a", "I16.T05.a"])

# ───────────────────────── рёбра ─────────────────────────
edge("T01", "I11.A8", "I12.A12",
     "Держатели туннеля — процессы в его сетевом пространстве, включая потомков.",
     "`tests/audit_ab_instance.rs`: дерево-фикстура с каталогами `<pid>/exe` и `<pid>/ns/net` (ссылки).",
     ["netns_id(\"net:[4026531840]\") == Some(4026531840); netns_id(\"net:[x]\") == None; netns_id(\"mnt:[1]\") == None",
      "фикстура: pid 10 (ns 100, /usr/bin/app), 11 (ns 200, /usr/bin/app), 12 (ns 200, /usr/bin/other), 13 (ссылки ns нет) → scan возвращает 4 записи по возрастанию pid; у 13 netns None",
      "holders(…, 200, uid) == [11, 12]; holders(…, 300, uid) пуст; holders с чужим uid пуст",
      "каталог с нечисловым именем (`self`, `net`) в scan не попадает",
      "app_list экземпляра без сети → not_running; класс операции Status",
      "app_list от uid B для сети uid A → not_running"])
edge("T02", "I11.A8", "I11.A9",
     "Экземпляр программы вне сети туннеля обнаруживается до запуска.",
     "`tests/audit_ab_instance.rs`.",
     ["та же фикстура: elsewhere(…, uid, \"/usr/bin/app\", 200) == [10]; для сети 100 == [11]",
      "elsewhere для \"/usr/bin/other\" и сети 200 пуст",
      "процесс с неизвестным netns в elsewhere не попадает",
      "процесс чужого uid с тем же exe не попадает"])
edge("T03", "I12.A10", "I12.A11",
     "Туннель разрешается до профиля подключения и источника; имя экземпляра годно для контроллера.",
     "`tests/audit_ab_assign.rs`: снимки строит тест (как `tests/audit_i02_store.rs`).",
     ["туннель t1 (владелец Application a1, профиль p1 → источник s1) → ResolvedTunnel{instance \"t1\", source_id s1, connection_profile_id p1}",
      "неизвестный туннель → UnknownTunnel; профиля нет → UnknownProfile; источника нет → UnknownSource",
      "идентификатор туннеля \"Work_VPN\" → BadInstanceName; \"host\" → BadInstanceName; 33 символа → BadInstanceName",
      "два туннеля с разными источниками → два ResolvedTunnel с разными source_id"])
edge("T04", "I12.A10", "I11.A9",
     "Приложение попадает только в свой туннель или в туннель своей группы.",
     "`tests/audit_ab_assign.rs`.",
     ["приложение a1 с OwnTunnel t1 (владелец Application a1) → shared false, tunnel.instance \"t1\"",
      "приложение a2 с OwnTunnel t1 (владелец Application a1) → OwnerMismatch",
      "приложение в группе g1, туннель группы принадлежит Group g1 → shared true",
      "туннель группы принадлежит Group g2 → OwnerMismatch; неизвестная группа → UnknownGroup",
      "назначение в туннель с владельцем Host → HostTunnel",
      "неизвестное приложение → UnknownApplication"])
edge("T05", "I15.E1", "I12.A13",
     "Имена профиля данных и сессии проходят одну проверку; каталоги выводятся из них однозначно.",
     "`tests/audit_ab_private.rs`.",
     ["valid_name: \"work\", \"s0123456789ab\", \"a-1\" → true; \"\", \"-a\", \"A\", \"a_b\", \"a/b\", 33 символа → false",
      "data_dirs(\"/home/u\", \"work\", Existing, \"s1\") == None",
      "Isolated → root \"/home/u/.local/share/cm/data/work\"; config, data, cache, state — его подкаталоги",
      "Fresh, сессия \"s1\" → root \"/home/u/.local/share/cm/data/work/fresh-s1\"",
      "env_overrides — ровно четыре ключа XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME, XDG_STATE_HOME; HOME нет",
      "runtime_dir(\"/run/user/1000\", \"s1\") == \"/run/user/1000/cm/s1\"",
      "prepare создаёт пять каталогов с правами 0700; повторный вызов — Ok; ссылка на месте root → ошибка",
      "check_dir: каталог 0700 своего uid → true; 0750 → false; ссылка на такой каталог → false; файл → false; чужой uid → false"])
edge("T06", "I12.A13", "I15.E2",
     "Окружение сессии читается из её файла с теми же проверками, что конфиг.",
     "`tests/audit_ab_private.rs`, `tests/audit_ab_session.rs`.",
     ["parse_session({\"timezone\":\"Europe/Berlin\",\"locale\":\"de_DE.UTF-8\"}) → data None, ipc Shared",
      "лишнее поле → Malformed; timezone \"../etc/passwd\" → BadZone; locale \"de DE\" → BadLocale; data.profile \"A\" → BadProfile",
      "session_file(preset, None, Shared) несёт пояс и локаль пресета",
      "environment_profile: после правки пресета ранее созданный профиль не изменился",
      "кадр app_launch без поля session разбирается; сериализация кадра без сессии не содержит ключа \"session\"",
      "session \"../x\" → bad_argument",
      "app_launch с session \"s1\", файла session-s1.json нет → invalid_config, процесс не создан",
      "файл сессии — ссылка → отказ; длиннее 4096 байт → too_large",
      "стенд: два запуска в одном туннеле с сессиями Europe/Berlin и Asia/Tokyo → у процессов TZ разные, LANG разные"], level="L2")
edge("T07", "I12.A13", "I14.R6",
     "Процесс без переменной TZ видит пояс региона, а хост — свой.",
     "`tests/ab_stand.py`, раздел `region`.",
     ["запуск с сессией Europe/Berlin, программа `env -u TZ date +%Z` (вывод в файл) → CET или CEST",
      "второй запуск в том же туннеле с Asia/Tokyo → JST",
      "sha256 файла /etc/localtime на хосте до и после запусков совпадает; `readlink /etc/localtime` не изменился",
      "session с timezone \"Nowhere/City\" (имя допустимо, файла нет) → invalid_config, процесс не создан",
      "запуск без пояса: /etc/localtime внутри совпадает с хостом"], level="L2")
edge("T08", "I15.E1", "I15.E2",
     "Каталоги данных и сокетов сессии принимаются, только если их подготовил сам пользователь.",
     "`tests/audit_ab_session.rs`; запуск — `tests/ab_stand.py`, раздел `private`.",
     ["data {profile work, state isolated}, каталоги подготовлены → в окружении процесса XDG_CONFIG_HOME = <home>/.local/share/cm/data/work/config (и ещё три)",
      "каталог data отсутствует → invalid_config, процесс не создан; контроллер каталог не создал",
      "root — ссылка на чужой каталог → invalid_config",
      "каталог с правами 0755 → invalid_config",
      "ipc private: XDG_RUNTIME_DIR = /run/user/<uid>/cm/<session>; DBUS_SESSION_BUS_ADDRESS в окружении нет; WAYLAND_DISPLAY \"wayland-1\" стал \"/run/user/<uid>/wayland-1\"; PULSE_SERVER = unix:/run/user/<uid>/pulse/native",
      "ipc private, WAYLAND_DISPLAY уже абсолютный → не изменён; PULSE_SERVER задан клиентом → не изменён",
      "ipc shared: XDG_RUNTIME_DIR = /run/user/<uid>, DBUS_SESSION_BUS_ADDRESS сохранён",
      "в обоих режимах в окружении нет SSH_AUTH_SOCK, GNOME_KEYRING_CONTROL, GPG_AGENT_INFO, даже если клиент их прислал (кадр с ними → bad_argument)",
      "две сессии с профилями work и home: файл, созданный первой в XDG_DATA_HOME, второй не виден"], level="L2")
edge("T09", "I15.E1", "I13.P2",
     "Ограничения запуска зависят от политики сокетов и состояния данных.",
     "`tests/audit_ab_packaging.rs`.",
     ["support(Native, Private, Existing) == Supported",
      "support(Native, Shared, Existing) == Partial([SessionBus])",
      "support(Native, Shared, Isolated) == Partial([SessionBus, DataOutsideXdg])",
      "support(AppImage, Private, Existing) == Partial([FuseHelper])",
      "support(AppImage, Shared, Fresh) == Partial([FuseHelper, SessionBus, DataOutsideXdg])"])
edge("T10", "I12.A11", "I14.R7",
     "Туннель поднимается из хранилища одной командой и снимается без утечки.",
     "`tests/ab_stand.py`, раздел `tunnel`: хранилище с двумя источниками, узлы — подставные HTTP-прокси в «интернете».",
     ["`cm tunnel up t1` → код 0, строка \"instance=t1 generation=1 netns=cm-0\"; `cm tunnel status t1` → \"state=up restarts=0 holders=0 generation=1\"",
      "повторный `cm tunnel up t1` → код 0, поколение прежнее, второго ядра нет",
      "`cm tunnel up t2` (другой источник) → netns cm-1; запрос из cm-0 прошёл через прокси источника 1, из cm-1 — через прокси источника 2 (журналы прокси)",
      "в gen-1.json владельца нет ключей dns, tun, listeners; права 0600",
      "net.json: {\"ipv6\":\"block\",\"fake_ip\":\"forbidden\",\"lifetime\":\"pinned\",\"failure\":\"block\"} при вызове без флагов",
      "`cm tunnel down t1` при живом приложении → код 5, \"[in_use]\"; с --force → код 0",
      "`cm tunnel down t2 --keep-net` → ядра нет, netns cm-1 есть, запрос из него — тайм-аут",
      "`cm tunnel up unknown` → код 2; контроллер не запущен → код 4",
      "после `down t1`: нет cmv0h, cmtun0, netns cm-0, правила 1000; туннель t2 не затронут"], level="L2")
edge("T11", "I11.A8", "I11.A9",
     "Контроллер отдаёт клиенту имя сети и её процессы.",
     "`tests/ab_stand.py`, раздел `apps`.",
     ["после запуска `sleep 30` в туннеле: app_list → Apps{netns \"cm-0\", pids содержит pid запуска}",
      "приложение, породившее фонового потомка и завершившееся: pid потомка остаётся в app_list",
      "после завершения всех процессов app_list → pids пуст"], level="L2")
edge("T12", "I12.A12", "AB.Z",
     "Туннель со сроком жизни по держателям останавливается сам, закреплённый — нет.",
     "`tests/audit_ab_watch.rs` (время задаёт тест); `tests/ab_stand.py`, раздел `groups`.",
     ["refcount: запуск, держатели есть → tick ничего не снимает",
      "держателей нет, прошло 4999 мс → туннель жив; прошло 5000 мс → выполнены остановка ядра и снятие сети, состояние stopped",
      "держатели вернулись на 3000-й мс → отсчёт начат заново",
      "refcount, запусков не было → туннель не снимается, сколько бы ни прошло",
      "pinned → не снимается никогда",
      "экземпляр host с refcount в net.json → не снимается",
      "tunnel_state.holders равен числу процессов владельца в netns",
      "стенд: группа из двух приложений на одном туннеле; первое закрыто → туннель работает, второе отвечает 200; второе закрыто → не позже чем через 10 с нет ни ядра, ни netns",
      "стенд: две группы на двух туннелях; снятие первой не меняет вторую",
      "стенд: `cm host set off` при работающих туннелях приложений → запросы приложений по-прежнему 200"], level="L2")

edge("U01", "I13.P1", "I13.P2",
     "Способ запуска определяется по пути, аргументам и началу файла.",
     "`tests/audit_ab_packaging.rs`.",
     ["detect(\"/usr/bin/flatpak\", [\"run\", \"--branch=stable\", \"org.example.App\"], None) == Flatpak{app_id Some(\"org.example.App\")}",
      "detect(\"/usr/bin/flatpak\", [\"list\"], None) == Native",
      "detect(\"/snap/bin/tool\", [], None) == Snap; detect(\"/usr/bin/snap\", [\"run\", \"tool\"], None) == Snap",
      "detect(\"/usr/bin/systemd-run\", [\"--user\", \"app\"], None) == Launcher; то же для gtk-launch, gio, xdg-open, dbus-send",
      "head \\x7fELF\\x02\\x01\\x01\\x00 A I \\x02 … → AppImage; тот же заголовок без байтов AI → Native",
      "head None, обычный путь → Native",
      "файл короче 16 байт → read_head None"])
edge("U02", "I13.P2", "I11.A9",
     "Неподдержанный способ запуска — отказ с причиной; ограничения видны пользователю.",
     "`tests/audit_ab_packaging.rs`; команда — `tests/audit_ab_cli.rs`.",
     ["support(Launcher, любые) == Unsupported(ExecutorOutsideNetns)",
      "support(Flatpak, любые) == support(Snap, любые) == Unsupported(NotVerified)",
      "limit_token: limit_session_bus, limit_fuse_helper, limit_data_outside_xdg; reason_token: unsupported_executor_outside, unsupported_not_verified",
      "`cm app run t1 -- /usr/bin/systemd-run --user x` → код 2, в stderr \"[unsupported_executor_outside]\"; к контроллеру обращений не было",
      "`cm app run t1 -- /usr/bin/flatpak run org.example.App` → код 2, \"[unsupported_not_verified]\""], level="L2")
edge("T13", "I15.E2", "I11.A9",
     "Клиент сам готовит каталоги сессии, и контроллер их принимает.",
     "`tests/ab_stand.py`, раздел `private`.",
     ["`cm app run --data isolated --ipc private a1` → код 0; созданы каталоги данных профиля и /run/user/<uid>/cm/<сессия>, все 0700",
      "в config/ владельца появился файл session-<имя>.json, имя — `s` и 12 шестнадцатеричных цифр",
      "два запуска подряд дают разные имена сессий",
      "`--data fresh` дважды → два разных каталога fresh-<сессия>; после завершения приложений оба на месте"], level="L2")
edge("T14", "I11.A9", "AB.Z",
     "Запуск замечает чужой экземпляр и передачу управления и не молчит об этом.",
     "`tests/ab_stand.py`, раздел `apps`; программы-фикстуры — в `tests/brokers/`.",
     ["программа-фикстура уже работает вне туннеля → `cm app run a1` → код 5, \"[running_elsewhere]\"; app_launch не вызывался",
      "то же с --allow-running → запуск состоялся",
      "фикстура single-instance (первый экземпляр слушает unix-сокет, второй передаёт ему аргументы и выходит с кодом 0), первый экземпляр запущен вне туннеля, второй — `cm app run --allow-running a1` → код 6, \"[handed_off]\"",
      "обычная программа (`sleep 30`) → код 0, в stdout pid",
      "пресет с локалью, которой нет в системе → код 2, \"[locale_not_installed]\"; с --allow-missing-locale → запуск",
      "пресет с несуществующим поясом → код 2, \"[zone_missing]\"",
      "несуществующий исполняемый файл → код 3; файл без права исполнения → код 3; процесс не создан",
      "программа завершилась сама через 5 с → код 0 (это не передача управления: проверка идёт на 2000-й мс)",
      "запуск с `--`: сессии нет, поиск чужого экземпляра не выполняется"], level="L2")

edge("R01", "I14.R4", "I14.R5",
     "Страна берётся только из ответа источника и только как две буквы.",
     "`tests/audit_ab_region.rs`, фикстуры `tests/fixtures/ab/geo/`.",
     ["parse(Trace, \"fl=1\\nip=203.0.113.7\\nloc=DE\\ntls=TLSv1.3\\n\") == Some(\"DE\")",
      "parse(Plain, \"de\\n\") == Some(\"DE\"); parse(Plain, \"Germany\") == None; parse(Plain, \"\") == None",
      "parse(Json(\"country\"), {\"ip\":\"203.0.113.7\",\"country\":\"NL\"}) == Some(\"NL\"); поле — число → None; не JSON → None",
      "тело 4097 байт → None; не UTF-8 → None",
      "default_sources: три записи с id cloudflare, ipinfo, countryis; все url начинаются с https://",
      "parse_source_arg(\"a=plain=https://x.test/c\") → Some; \"a=xml=https://x\" → None; \"A=plain=https://x\" → None; \"a=plain=http://x\" → None вне тестового режима",
      "parse_source_arg(\"a=json:country=https://x.test/\") → формат Json(\"country\")"])
edge("R02", "I14.R5", "I14.R7",
     "Наблюдения собираются запросами из сети туннеля и не теряются молча.",
     "`tests/audit_ab_region.rs` (файл результата); `tests/ab_stand.py`, раздел `region` (источники — HTTP в «интернете»).",
     ["read_result: массив из двух записей → два Observation; не JSON → Malformed; файл 16 КиБ + 1 байт → Malformed",
      "`cm env probe` с --out вне $XDG_RUNTIME_DIR/cm → код 2; файл уже существует → код 2",
      "стенд: два источника отвечают \"DE\" → collect возвращает два наблюдения с country \"DE\"; запросы пришли на «интернет» от туннеля (журнал прокси узла), а не напрямую",
      "стенд: один источник не отвечает → одно наблюдение, ошибки нет",
      "стенд: ядро туннеля убито → collect возвращает Launch (контроллер ответил not_running)",
      "файл результата после чтения удалён; права созданного файла 0600"], level="L2")
edge("R03", "I14.R7", "AB.Z",
     "Проверка региона называет исход и не объявляет страну подтверждённой без согласия источников.",
     "`tests/ab_stand.py`, раздел `region`.",
     ["два источника DE, `--country DE` → \"region=agreed DE 2 sources=2 axis=verified\", код 0",
      "два источника DE, `--country NL` → axis=blocked, код 7",
      "источники DE и NL → \"region=disagree DE,NL\", axis=partial, код 8",
      "один источник → \"region=insufficient 1\", axis=unknown, код 8",
      "без --country → строка без axis, код 0",
      "второй вызов после смены ответа источников с DE на NL при `--country DE` → \"invalidation=region_mismatch\"; приложение в туннеле не перезапущено (pid тот же)",
      "первый вызов (файла прошлого решения нет) → поля invalidation в строке нет"], level="L2")
edge("R04", "I14.R6", "AB.Z",
     "Отказ подстановки пояса останавливает запуск, а не оставляет пояс хоста.",
     "`tests/audit_ab_session.rs`.",
     ["drop_into_netns_pre_exec (прежняя сигнатура) собирается и ведёт себя как раньше",
      "путь пояса с NUL → bad_argument, процесс не создан",
      "пояс задан, а файла нет → invalid_config; в ответе нет пути и имени пояса"], level="L2")

edge("Q01", "I16.Q3", "I16.Q4",
     "Показатели узла считаются по последним пробам; число проб ограничено бюджетом.",
     "`tests/audit_ab_quality.rs`.",
     ["задержки удачных проб [100, 120, 110, 300], одна неудачная → samples 5, loss_percent 20, median_delay_ms 110, jitter_ms 10",
      "проб нет → loss_percent 100, median None, jitter None",
      "25 проб → учитываются последние 20 по времени",
      "все пробы неудачные → loss_percent 100, median None, last_ok None",
      "budget(Pinned, обычные) == {300000, 12}; (Failover) == {60000, 60}; (Adaptive) == {30000, 120}",
      "budget(Adaptive, metered) == {120000, 30}; (Adaptive, metered и on_battery) == {120000, 30}",
      "allow_probe: последняя проба 29 999 мс назад при Adaptive → TooSoon; 30 000 мс назад → Allow",
      "120 проб за последний час при Adaptive → HourlyBudget; проба ровно 3 600 000 мс назад в счёт не идёт",
      "история пуста → Allow",
      "parse_resources(\"12 (a b) c) S 1 12 12 0 -1 4194560 100 0 0 0 7 5 0 0 20 0 1 0 100 0 0\", \"100 250 30 1 0 20 0\", 4) → cpu_ticks 12, rss_kib 1000; строка без `)` → None",
      "to_candidate переносит median_delay_ms в delay_ms и loss_percent без изменений"])
edge("Q02", "I16.Q4", "AB.Z",
     "Каждое решение о выборе узла имеет причину; узел не дёргается и не меняется под живыми сессиями.",
     "`tests/audit_ab_quality.rs`.",
     ["Pinned, текущий годен → Keep(Pinned); Pinned, текущий отклонён, есть лучшие → Block(PinnedNodeRejected)",
      "ranked пуст (все отклонены ограничением страны) → Block(NoCandidates): ограничение не ослаблено",
      "current None → Switch{лучший, NoCurrent}",
      "Failover, текущий годен, есть узел вдвое быстрее → Keep(CurrentUsable); текущий отклонён → Switch{лучший, CurrentRejected}",
      "Adaptive, live_sessions 1, есть узел вдвое быстрее → Keep(SessionsPinned)",
      "Adaptive, сессий нет, неудачное переключение 299 999 мс назад → Keep(Cooldown); 300 000 мс назад → решение switch::decide",
      "Adaptive, выигрыш 19% → Keep(GainTooSmall); 20% и срок выдержан → Switch{…, BetterNode}",
      "reason_token: sessions_pinned, current_rejected, no_candidates, pinned_node_rejected, gain_too_small",
      "свойство (10 000 случайных рядов, два узла с оценками в пределах ±10% друг от друга, шаг 30 с, 1 час) → переключений 0",
      "свойство (случайные ряды, любой режим): два Switch подряд разделяет не меньше MIN_DWELL_MS, кроме случая CurrentRejected",
      "лимитная сеть и батарея: allow_probe по бюджету ×4 отказывает раньше; смена страны узла на запрещённую делает его отклонённым → CurrentRejected или Block"])
