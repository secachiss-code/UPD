#!/usr/bin/env python3
"""Рубеж I06 «Привилегированный контроллер»: вершины и рёбра-контракты.

Единственный источник для I06-DAG.md/.json, GROK-I06-CODE.md и TESTS-I06.md.
remaining_dag.py берёт отсюда вершины как подзадачи рубежа I06.

Запуск: python3 docs/design/cm-network-manager/tools/i06_dag.py [--check]
"""

import sys
from pathlib import Path

import direction_kit as kit

HERE = Path(__file__).resolve().parent
OUT = HERE.parent
FINAL = "I06.T05.a"

DIRECTIONS = [
    ("P", "Протокол", "Типизированные кадры с версией, жёсткий декодер, отдельные действия polkit по классам операций."),
    ("I", "Идентификация", "Кто просит: только из сокета и ядра, никогда из тела запроса; чьё это: путь и имя из uid peer."),
    ("H", "Ужесточение", "Дочерний процесс получает только то, что ему положено: uid вызывающего, без capabilities и чужих дескрипторов."),
    ("T", "Транзакции", "Журнал владения, компенсация при частичном отказе, поколения и идемпотентность."),
    ("W", "Работа с ядром", "Жизненный цикл worker только через контроллер и CoreAdapter."),
    ("S", "Сервис", "Вход `cm controller serve` и связка всех частей."),
    ("Z", "Приёмка", "Отрицательные проверки: чужой uid, подмена PID, инъекции, устаревший запрос, обрыв транзакции."),
]

V = []
E = []


def vertex(vid, direction, title, size, level, deps, files, spec, external=()):
    V.append(dict(id=vid, direction=direction, title=title, size=size, level=level, deps=deps,
                  files=files, spec=spec, external=list(external)))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


# ───────────────────────────── вершины ─────────────────────────────
vertex("I06.P1", "P", "Описать кадры запроса и ответа контроллера.", "S", "L1", [],
       ["src/controller/mod.rs", "src/controller/protocol.rs", "src/lib.rs"],
       """\
`pub mod controller;` в `src/lib.rs`. Подмодули: `protocol, codec, actions, peer, owner, harden, drop, journal, txn, registry, core_ops, dispatch, server`.

```rust
pub const CONTROL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 64 << 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request { pub v: u32, pub id: String, pub op: Op }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    WorkerStart { instance: String, generation: u64 },
    WorkerReload { instance: String, generation: u64, next_generation: u64 },
    WorkerStop { instance: String, generation: u64 },
    WorkerStatus { instance: String },
    NetApply { instance: String, generation: u64 },     // содержание — I07
    NetRevert { instance: String, generation: u64 },    // содержание — I07
    AppLaunch { instance: String, generation: u64, program: String, args: Vec<String> }, // I11
    Reconcile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpClass { Status, Worker, Net, App }

impl Op { pub fn class(&self) -> OpClass; pub fn instance(&self) -> Option<&str>; }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply { pub v: u32, pub id: String, pub ok: bool, pub code: String, pub data: Option<ReplyData> }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplyData {
    Started { generation: u64 },
    Status { running: bool, generation: Option<u64>, api: String, route: String, remote: String },
    Reconciled { compensated: u32 },
}
```

Классы: `WorkerStatus` → `Status`; `WorkerStart/Reload/Stop`, `Reconcile` → `Worker`; `NetApply/NetRevert` → `Net`; `AppLaunch` → `App`.

**В кадре нет uid, пути и байтов конфига.** Поля с такими именами не добавлять: uid берётся из сокета (I06.I1), путь вычисляется (I06.I2), конфиг читается из каталога владельца.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    BadFrame, TooLarge, UnsupportedVersion, BadId, BadInstance, BadArgument,
    Denied, PeerChanged, GenerationMismatch, Busy, Quota, NotRunning, Conflict,
    InvalidConfig, Timeout, Unsupported, Crashed, Failed,
}
impl ControlError { pub fn code(self) -> &'static str }  // snake_case имени: "bad_frame", "too_large", …
```
`Reply::ok(id, data)` → `ok: true, code: "ok"`; `Reply::error(id, error)` → `ok: false, code: error.code(), data: None`. Других текстов в ответе нет.""",
       external=["I06.T01.a", "H.09"])

vertex("I06.P2", "P", "Декодировать и кодировать кадры с жёсткими пределами.", "M", "L1", ["I06.P1"],
       ["src/controller/codec.rs"],
       """\
`pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, ControlError>`
- Читает до `\\n`, не больше `MAX_FRAME_BYTES + 1` байт. Длиннее → `TooLarge`; хвост до `\\n` при этом вычитывается порциями без накопления, чтобы следующее чтение началось с нового кадра.
- EOF без данных → `Ok(None)`; EOF посреди кадра → `BadFrame`.
- Возвращает кадр без завершающего `\\n`.

`pub fn decode(frame: &[u8]) -> Result<Request, ControlError>` — порядок проверок:
1. длина > `MAX_FRAME_BYTES` → `TooLarge`;
2. не UTF-8, не JSON, неизвестное поле или неизвестный `type` → `BadFrame`;
3. `v != CONTROL_VERSION` → `UnsupportedVersion`;
4. `id` не `^[a-z0-9-]{8,64}$` → `BadId`;
5. `instance` не проходит `core::InstanceId::new` → `BadInstance`;
6. `WorkerReload`: `next_generation <= generation` → `BadArgument`;
7. `AppLaunch`: `program` не абсолютный путь, содержит NUL или `..`-сегмент; `args` больше 64 штук, аргумент длиннее 4096 байт или содержит NUL → `BadArgument`.

`pub fn encode(reply: &Reply) -> Vec<u8>` — одна строка JSON и `\\n`; внутри строки нет `\\n`.

`pub fn request_digest(request: &Request) -> String` — sha256 (hex) канонического JSON поля `op` (сериализация serde_json без пробелов); нужен идемпотентности (I06.T3).""")

vertex("I06.P3", "P", "Сопоставить классы операций с действиями polkit и проверять права.", "M", "L1", ["I06.P1"],
       ["src/controller/actions.rs", "src/helper/auth.rs", "src/main.rs", "src/i18n_table.rs"],
       """\
```rust
pub const ACTION_WORKER: &str = "io.github.cm.worker"; // жизненный цикл своего worker
pub const ACTION_NET: &str = "io.github.cm.net";       // маршруты и firewall
pub const ACTION_APP: &str = "io.github.cm.app";       // запуск приложения в своём окружении
pub fn action_for(class: OpClass) -> &'static str      // Status → helper::ACTION_STATUS
pub trait Authorizer: Send + Sync { fn authorize(&self, peer: &PeerIdentity, action: &str) -> Result<(), ControlError>; }
pub struct PolkitAuthorizer;
pub fn pkcheck(pid: i32, start_time: u64, uid: u32, action: &str) -> PkResult // Allowed | Dismissed | NoAgent | Denied | Failed
```
- `PolkitAuthorizer`: uid 0 → Ok; `test_mode()` и `CM_HELPER_ALLOW=1` → Ok; иначе `pkcheck` с субъектом `pid,start_time,uid` из `PeerIdentity`. Любой результат кроме `Allowed` → `ControlError::Denied`.
- **Одна реализация вызова pkcheck.** `src/helper/auth.rs::authorize` вызывает `controller::actions::pkcheck` и переводит `PkResult` в свои прежние сообщения. Собственный запуск `pkcheck` из helper удаляется. Поведение и тексты helper не меняются.
- **Политика.** В список действий генератора политики в `src/main.rs` (рядом с `helper::ACTION_MANAGE`) добавить три действия с сообщениями на 6 языках:
  - `ACTION_WORKER` — `yes` для активного сеанса;
  - `ACTION_NET` — `auth_admin_keep`;
  - `ACTION_APP` — `yes`.
  В существующих строках — только добавление элементов списка.""")

vertex("I06.I1", "I", "Установить личность процесса на другом конце сокета.", "M", "L1", [],
       ["src/controller/peer.rs"],
       """\
```rust
pub struct PeerIdentity {
    pub uid: u32, pub gid: u32, pub pid: i32,
    pub start_time: u64,         // поле 22 /proc/<pid>/stat
    pub cgroup: String,          // путь из строки "0::" /proc/<pid>/cgroup
    pub session: Option<String>, // N из сегмента "session-N.scope"
    pub netns_inode: u64,        // st_ino /proc/<pid>/ns/net
    pidfd: OwnedFd,
}
pub fn capture(stream: &UnixStream) -> Result<PeerIdentity, ControlError>
impl PeerIdentity { pub fn alive(&self) -> bool; pub fn verify(&self) -> Result<(), ControlError>; }
pub fn parse_start_time(stat: &str) -> Option<u64>
pub fn parse_cgroup(text: &str) -> Option<String>
pub fn session_of(cgroup: &str) -> Option<String>
```
`capture` — строго в этом порядке:
1. `SO_PEERCRED` (uid, gid, pid); pid ≤ 0 → `PeerChanged`;
2. `pidfd_open(pid, 0)` через `libc::syscall(libc::SYS_pidfd_open, …)`; ошибка → `PeerChanged`;
3. только после этого чтение `/proc/<pid>/stat`, `cgroup`, `ns/net`;
4. `alive()` ещё раз: если процесс уже завершился, прочитанное могло относиться к другому процессу → `PeerChanged`.

`alive()` — `poll` на pidfd с нулевым тайм-аутом: `POLLIN` означает, что процесс завершился.
`verify()` — `alive()` и `start_time` из `/proc/<pid>/stat` совпадает с сохранённым; иначе `PeerChanged`.

`parse_start_time` берёт часть после последней `)`, поле 20 в ней: имя процесса может содержать пробелы и скобки.

`Debug` для `PeerIdentity` не печатает `cgroup` целиком (только uid, pid, session). Чтение uid из любого другого источника запрещено.""")

vertex("I06.I2", "I", "Вычислить каталог, файл конфига и имя юнита владельца из uid peer.", "M", "L1", ["I06.I1"],
       ["src/controller/owner.rs", "src/core/unit.rs"],
       """\
```rust
pub const SYSTEM_BASE: &str = "/var/lib/cm/users";
pub struct Owned { pub uid: u32, pub instance: InstanceId, pub root: PathBuf, pub unit: String }
pub fn owned(base: &Path, uid: u32, instance: &str) -> Result<Owned, ControlError>
impl Owned {
    pub fn config_path(&self, generation: u64) -> PathBuf; // <root>/instances/<instance>/config/gen-<generation>.json
    pub fn journal_path(&self) -> PathBuf;                 // <root>/journal.jsonl
    pub fn generations_path(&self) -> PathBuf;             // <root>/generations.json
}
pub fn read_owned_config(owned: &Owned, generation: u64) -> Result<CoreConfig, ControlError>
```
- `root = <base>/u<uid>`; `unit = "cm-core-u<uid>-<instance>.service"`. `uid` — только аргумент функции, который вызывающий берёт из `PeerIdentity`.
- `instance` не проходит `InstanceId::new` → `BadInstance`.
- `read_owned_config`: файл открывается с `O_NOFOLLOW|O_CLOEXEC`; затем `fstat` по открытому дескриптору. Отказ `InvalidConfig`, если это не обычный файл, владелец ≠ `owned.uid`, права содержат запись для группы или остальных (`mode & 0o022 != 0`), размер > `core::adapter::MAX_CONFIG_BYTES`, либо файла нет. Каталог `config` — симлинк → тоже `InvalidConfig`.

`src/core/unit.rs`: новая `pub fn render_user_core_unit(owned_root: &Path, uid: u32, id: &InstanceId, privileges: NetPrivileges) -> String` — тот же текст, что `render_core_unit`, но:
- `User=<uid>` в секции `[Service]`;
- пути под `<owned_root>/instances/<id>`;
- `RuntimeDirectory=cm-core-u<uid>-<id>`.

`render_core_unit` не менять.""")

vertex("I06.H1", "H", "Подготовить дочерний процесс: дескрипторы, пределы, capabilities, секреты через fd.", "M", "L1", [],
       ["src/controller/harden.rs"],
       """\
```rust
pub struct ChildLimits { pub nofile: u64, pub core: u64 }  // по умолчанию nofile 4096, core 0
pub fn harden_pre_exec(command: &mut Command, keep_fds: &[RawFd], limits: ChildLimits)
pub fn secret_fd(bytes: &[u8]) -> Result<OwnedFd, ControlError>
```
`harden_pre_exec` ставит `pre_exec` (с `// SAFETY:`), который в дочернем процессе до exec:
1. закрывает все дескрипторы ≥ 3, кроме `keep_fds` — `close_range` по промежуткам между сохраняемыми номерами (`libc::syscall(libc::SYS_close_range, …)`);
2. `setrlimit(RLIMIT_NOFILE)` и `setrlimit(RLIMIT_CORE)`;
3. `prctl(PR_SET_NO_NEW_PRIVS, 1)`;
4. сбрасывает ambient-набор (`PR_CAP_AMBIENT_CLEAR_ALL`) и bounding-набор (`PR_CAPBSET_DROP` для 0..=CAP_LAST_CAP, ошибки EINVAL для несуществующих номеров игнорировать).

В `pre_exec` — только async-signal-safe вызовы, без выделения памяти.

`secret_fd`: `memfd_create("cm-secret", MFD_CLOEXEC | MFD_ALLOW_SEALING)`, запись всех байтов, `lseek` в начало, печати `F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL`. Секрет не попадает ни в argv, ни в env.""")

vertex("I06.I3", "I", "Запускать процесс от имени вызывающего пользователя, а не root.", "M", "L2", ["I06.H1"],
       ["src/controller/drop.rs", "src/core/mihomo/lifecycle.rs"],
       """\
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAs { pub uid: u32, pub gid: u32, pub groups: Vec<u32> }
pub fn run_as_for(uid: u32, gid: u32) -> Result<RunAs, ControlError>  // группы через getgrouplist по имени из getpwuid_r
pub fn drop_pre_exec(command: &mut Command, run_as: RunAs, keep_fds: &[RawFd], limits: ChildLimits)
pub fn spawn_as(run_as: RunAs, program: &Path, args: &[String], env: &BTreeMap<String, String>) -> Result<Child, ControlError>
```
`drop_pre_exec` — один `pre_exec`, порядок обязателен:
1. `setgroups(groups)`;
2. `setresgid(gid, gid, gid)`;
3. `setresuid(uid, uid, uid)`;
4. проверка, что вернуть root нельзя: `setresuid(0, 0, 0)` обязан завершиться ошибкой, если целевой uid ≠ 0; иначе `_exit(126)`;
5. шаги `harden_pre_exec` (дескрипторы, пределы, no_new_privs, capabilities).

Группы вычисляются в родителе, до fork.

`spawn_as`: `env_clear()` и только переданный `env`; stdin null; `program` обязан быть абсолютным (`BadArgument`).

`src/core/mihomo/lifecycle.rs`: у `MihomoWorker` новый метод `pub fn set_run_as(&mut self, run_as: RunAs)`. Если он задан, `spawn` вызывает `drop_pre_exec` вместо нынешнего `pre_exec` с одним `setsid` (сам `setsid` остаётся первым шагом). Без `set_run_as` поведение прежнее. В diff `lifecycle.rs` — только эти строки.""")

vertex("I06.T1", "T", "Вести журнал владения с дозаписью и восстановлением после обрыва.", "M", "L1", [],
       ["src/controller/journal.rs"],
       """\
Файл — JSON-строки, права 0600, каждая запись дописывается и синхронизируется (`sync_data`).
```rust
#[derive(Serialize, Deserialize)]
pub struct Record { pub txn: String, pub event: Event, pub instance: String, pub generation: u64,
                    pub digest: String, pub step: Option<String>, pub reply: Option<String>, pub at: i64 }
#[serde(rename_all = "snake_case")]
pub enum Event { Begin, Step, Commit, Abort, Compensated, CompensationFailed }
pub enum TxnStatus { Pending, Committed, Aborted, Dirty }
pub struct TxnState { pub txn: String, pub instance: String, pub generation: u64, pub digest: String,
                      pub steps_done: Vec<String>, pub status: TxnStatus, pub reply: Option<String> }
pub struct Journal { /* путь и открытый файл */ }
impl Journal {
    pub fn open(path: &Path) -> Result<Self, ControlError>;      // создаёт 0600; симлинк → Failed
    pub fn append(&mut self, record: &Record) -> Result<(), ControlError>;
    pub fn replay(&self) -> Result<Vec<TxnState>, ControlError>; // в порядке Begin
    pub fn find(&self, txn: &str) -> Result<Option<TxnState>, ControlError>;
    pub fn compact(&mut self) -> Result<(), ControlError>;
}
```
- `Commit` несёт `reply` — строку ответа, которую получит повторный запрос с тем же `id`.
- Статусы: `Commit` → `Committed`; `Abort` или `Compensated` → `Aborted`; `CompensationFailed` → `Dirty`; иначе `Pending`.
- **Оборванная последняя строка** (нет `\\n` или не JSON) при `replay` пропускается; при `open` файл усекается до конца последней целой строки.
- `compact`: если файл больше 1 МиБ — переписать атомарно (`.tmp`, fsync, rename), оставив все `Pending` и `Dirty` целиком и последние 256 `Committed`. Вызывается из `append` после записи `Commit`.""",
       external=["I04.T02.b"])

vertex("I06.T2", "T", "Выполнять шаги транзакции с компенсацией и восстановлением.", "L", "L1", ["I06.T1"],
       ["src/controller/txn.rs"],
       """\
```rust
pub trait Step { fn name(&self) -> &'static str;
                 fn apply(&mut self) -> Result<(), ControlError>;
                 fn compensate(&mut self) -> Result<(), ControlError>; }
pub struct TxnMeta<'a> { pub txn: &'a str, pub instance: &'a str, pub generation: u64, pub digest: &'a str }
pub fn run(journal: &mut Journal, meta: &TxnMeta, steps: &mut [Box<dyn Step + '_>], reply: &str,
           crash_before: &dyn Fn(&str) -> bool, now: i64) -> Result<(), ControlError>
pub fn reconcile(journal: &mut Journal, steps_for: &mut dyn FnMut(&TxnState) -> Vec<Box<dyn Step>>, now: i64) -> Result<u32, ControlError>
```
`run`:
1. запись `Begin`;
2. для каждого шага по порядку: если `crash_before(name)` → вернуть `Crashed` **без компенсации и без записи** (так тест воспроизводит падение процесса); `apply`; при успехе запись `Step`;
3. `apply` вернул ошибку → `compensate` уже выполненных шагов в обратном порядке, запись `Abort`, вернуть эту ошибку;
4. компенсация вернула ошибку → запись `CompensationFailed`, вернуть `Failed` (остальные компенсации всё равно выполняются);
5. после последнего шага: `crash_before("commit")` → `Crashed`; иначе запись `Commit` с `reply`.

`reconcile`: для каждой `Pending` транзакции `steps_for(state)` даёт шаги по именам из `steps_done`; `compensate` в обратном порядке; запись `Compensated` (или `CompensationFailed`). Возвращает число транзакций, получивших `Compensated`. `Dirty` и завершённые не трогает. Повторный вызов ничего не делает.

Шаг обязан быть идемпотентным в `compensate`: после обрыва неизвестно, успел ли `apply` шага, перед которым упал процесс.""")

vertex("I06.T3", "T", "Хранить поколения и отвечать на повторный запрос сохранённым ответом.", "M", "L1", ["I06.T1"],
       ["src/controller/registry.rs"],
       """\
```rust
pub struct Generations { /* <root>/generations.json: {"<instance>": <u64>} , 0600, атомарная запись */ }
impl Generations {
    pub fn load(path: &Path) -> Result<Self, ControlError>;
    pub fn current(&self, instance: &str) -> Option<u64>;
    pub fn set(&mut self, instance: &str, generation: u64) -> Result<(), ControlError>;
    pub fn clear(&mut self, instance: &str) -> Result<(), ControlError>;
}
pub enum Replay { Fresh, Stored(String /* строка ответа */) }
pub fn replay_or_fresh(journal: &Journal, txn: &str, digest: &str) -> Result<Replay, ControlError>
pub fn check_start(current: Option<u64>, requested: u64) -> Result<(), ControlError>
pub fn check_running(current: Option<u64>, requested: u64) -> Result<(), ControlError>
```
- `check_start`: `requested < current` → `GenerationMismatch` (устаревший запрос); иначе Ok.
- `check_running` (reload, stop): `current == None` → `NotRunning`; `requested != current` → `GenerationMismatch`.
- `replay_or_fresh`:
  - транзакции с таким `txn` нет → `Fresh`;
  - `Committed` и `digest` совпал → `Stored(reply)`;
  - `digest` другой → `Conflict` (тот же ключ для другой операции);
  - `Pending`, `Dirty` или `Aborted` с тем же digest → `Conflict` (сначала нужен `reconcile`).""")

vertex("I06.W1", "W", "Управлять жизненным циклом worker через CoreAdapter как транзакциями.", "L", "L2",
       ["I06.T2", "I06.T3", "I06.I2", "I06.I3"],
       ["src/controller/core_ops.rs"],
       """\
```rust
pub const MAX_INSTANCES_PER_UID: usize = 16;
pub trait AdapterFactory: Send + Sync { fn make(&self, owned: &Owned, run_as: &RunAs) -> Box<dyn CoreAdapter + Send>; }
pub struct Workers { /* Mutex<HashMap<(u32, String), Running>>; Running { adapter, generation } */ }
impl Workers {
    pub fn new(factory: Arc<dyn AdapterFactory>) -> Self;
    pub fn start(&self, owned: &Owned, run_as: &RunAs, txn: &str, digest: &str, generation: u64, now: i64) -> Result<Reply, ControlError>;
    pub fn reload(&self, owned: &Owned, txn: &str, digest: &str, generation: u64, next: u64, now: i64) -> Result<Reply, ControlError>;
    pub fn stop(&self, owned: &Owned, txn: &str, digest: &str, generation: u64, now: i64) -> Result<Reply, ControlError>;
    pub fn status(&self, owned: &Owned, txn: &str) -> Reply;
    pub fn reconcile(&self, owned: &Owned, txn: &str, now: i64) -> Result<Reply, ControlError>;
    pub fn stop_all(&self);
}
pub struct MihomoFactory { pub binary: PathBuf, pub lease_dir: PathBuf }
```
Общее для `start`, `reload`, `stop`: открыть `Journal` и `Generations` владельца; `replay_or_fresh` — `Stored` возвращается сразу, без выполнения.

**start:**
- `check_start`;
- экземпляр уже запущен → `Conflict`;
- у uid уже `MAX_INSTANCES_PER_UID` запущенных → `Quota`;
- `read_owned_config(owned, generation)`;
- транзакция из шагов:
  - `adapter_start` — apply: `factory.make` и `adapter.start(config)`; compensate: `adapter.stop()`, ошибку `NotRunning` считать успехом;
  - `record_generation` — apply: `Generations::set`; compensate: вернуть прежнее значение или `clear`;
- при успехе экземпляр попадает в карту; ответ `Started { generation }`.

**reload:** `check_running(current, generation)`; `read_owned_config(owned, next)`; шаги `adapter_reload` (compensate: `reload` прежним конфигом поколения `generation`) и `record_generation(next)`. Ошибка адаптера оставляет прежнее поколение.

**stop:** `check_running`; шаги `adapter_stop` (compensate — ничего: остановку не откатываем) и `clear_generation`; экземпляр убирается из карты.

**status:** без журнала и без проверки поколения; для не запущенного — `Status { running: false, generation: None, api: "down", route: "down", remote: "unknown" }`. Значения осей — snake_case имён вариантов `ApiState`/`RouteState`/`RemoteState`.

**reconcile:** `txn::reconcile` по журналу владельца; для шага `adapter_start` компенсация — остановить экземпляр, если он есть в карте; `Reconciled { compensated }`.

Ошибки `CoreError` → `ControlError`: `InvalidConfig→InvalidConfig, Unsupported→Unsupported, NotRunning→NotRunning, Busy→Busy, Timeout→Timeout, Conflict→Conflict`, остальные → `Failed`.

`MihomoFactory::make`: `MihomoWorker::new(id, InstanceRoot::new(&owned.root), lease_dir, binary)` и `set_run_as(run_as)`.

Карта экземпляров разделена по uid: экземпляр другого uid с тем же именем не виден и не считается в квоте.

**Точка обрыва для проверок.** `crash_before` для `txn::run` в продукте всегда возвращает `false`. Только при `test_mode()` и заданной переменной `CM_CONTROLLER_CRASH_BEFORE=<имя шага или commit>` на этом шаге процесс завершается через `std::process::abort()`. Вне `test_mode()` переменная не читается.""",
       external=["I04.T05.a"])

vertex("I06.W2", "W", "Провести кадр через проверки и вернуть ответ без утечек.", "M", "L1",
       ["I06.P2", "I06.P3", "I06.I1", "I06.W1"],
       ["src/controller/dispatch.rs"],
       """\
```rust
pub const MAX_CONCURRENT_OPS: usize = 4;
pub struct Deps { pub base: PathBuf, pub authorizer: Arc<dyn Authorizer>, pub workers: Arc<Workers>,
                  pub slots: Arc<OpSlots>, pub now: fn() -> i64 }
pub fn handle(frame: &[u8], peer: &PeerIdentity, deps: &Deps) -> Vec<u8>   // всегда одна строка ответа
```
Порядок, каждый отказ — ответ `Reply::error` и немедленный выход:
1. `decode`; при ошибке `id` в ответе — пустая строка;
2. `peer.verify()` → `PeerChanged`;
3. `authorizer.authorize(peer, action_for(op.class()))` → `Denied`;
4. `OpSlots::try_acquire()` (счётчик, не больше `MAX_CONCURRENT_OPS` одновременно) → `Busy`; слот освобождается при выходе из `handle`, в том числе при панике (guard);
5. `owned(&deps.base, peer.uid, instance)` — uid только из `peer`;
6. операция:
   - `WorkerStart/Reload/Stop/Status`, `Reconcile` → `Workers` (`run_as` — `run_as_for(peer.uid, peer.gid)`);
   - `NetApply`, `NetRevert`, `AppLaunch` → `Unsupported` (после проверки прав; содержание — I07 и I11).

`Reconcile` не имеет `instance`: `owned` строится с фиксированным именем `"journal"` только ради путей журнала.

Ответ — только `code` из фиксированного набора и `ReplyData`. Ни путь, ни argv, ни текст ошибки ОС в ответ и в журнал процесса не попадают.""")

vertex("I06.S1", "S", "Дать вход `cm controller serve` на unix-сокете.", "M", "L2", ["I06.W2"],
       ["src/controller/server.rs", "src/main.rs"],
       """\
`src/main.rs`: `if args.first() == "controller" { exit(cm::controller::server::dispatch(&args[1..])) }` рядом с `source` и `identity`.

`cm controller serve --socket PATH [--base DIR]`:
- вне `test_mode()` требует euid 0 (код 4) и `--base` не принимает (всегда `SYSTEM_BASE`); в `test_mode()` `--base` обязателен;
- сокет:
  - если `LISTEN_PID` — этот процесс и `LISTEN_FDS >= 1`, берётся fd 3 (как в helper);
  - иначе — `bind` по `PATH`, права 0666. Родительский каталог обязан существовать, не быть симлинком и принадлежать euid;
  - доступ ограничивают `SO_PEERCRED` и polkit, а не права сокета;
- фабрика: `MihomoFactory { binary: core::unit::CORE_BIN (в test_mode — `CM_CORE_BIN`, если задан), lease_dir: <base>/leases }`;
- на соединение — поток; одновременно не больше 32 соединений, лишние закрываются сразу;
- в потоке: `peer::capture` один раз; цикл `read_frame → handle → write`; ошибка `read_frame` → ответ с этим кодом и закрытие соединения; тайм-аут чтения кадра 30 с;
- `SIGTERM` и `SIGINT`: перестать принимать, `Workers::stop_all()`, удалить свой сокет (если создавал сам), выйти с кодом 0.

Коды выхода: 0 — штатная остановка; 2 — использование; 4 — нет прав или сокет небезопасен.

Интеграция в установку (юнит, сокет systemd, общий процесс с helper) в эту вершину не входит: её решает пользователь при приёмке (см. отчёт).""")

vertex(FINAL, "Z", "Принять контроллер отрицательными проверками.", "M", "L2", ["I06.S1"],
       ["docs/design/cm-network-manager/i06-evidence/<дата>/summary.json"],
       """\
Вершина роли 2: кода нет. Закрывается, когда все рёбра проверены и обходов нет.""",
       external=["I04.T05.c"])

# ───────────────────────────── рёбра ─────────────────────────────
edge("C01", "I06.P1", "I06.P2",
     "Кадр несёт только версию, ключ и операцию; uid, путей и байтов конфига в схеме нет.",
     "`tests/audit_i06_protocol.rs`: сериализация и разбор образцов.",
     [
         "`{\"v\":1,\"id\":\"a1b2c3d4-0001\",\"op\":{\"type\":\"worker_start\",\"instance\":\"browser\",\"generation\":7}}` ⇄ Request{v 1, id, WorkerStart{browser, 7}} (roundtrip)",
         "классы: WorkerStatus → Status; WorkerStart, WorkerReload, WorkerStop, Reconcile → Worker; NetApply, NetRevert → Net; AppLaunch → App",
         "Op::instance(): Reconcile → None, остальные → Some",
         "ControlError::code(): BadFrame → \"bad_frame\", GenerationMismatch → \"generation_mismatch\", PeerChanged → \"peer_changed\"; все 18 кодов различны и состоят из [a-z_]",
         "Reply::ok(\"k\", Started{generation 7}) → `{\"v\":1,\"id\":\"k\",\"ok\":true,\"code\":\"ok\",\"data\":{\"type\":\"started\",\"generation\":7}}`",
         "Reply::error(\"k\", Denied) → ok false, code \"denied\", data null",
     ])
edge("C02", "I06.P2", "I06.W2",
     "Декодер отвергает всё, что не является точным кадром версии 1, и не читает больше 64 КиБ.",
     "`tests/audit_i06_protocol.rs`: табличный тест и детерминированная мутация.",
     [
         "кадр длиной 65537 байт → TooLarge; ровно 65536 байт корректного JSON с длинным id → BadId (не TooLarge)",
         "`not json`, `{}`, `[]`, байты 0xFF → BadFrame",
         "лишнее поле: `{\"v\":1,\"id\":\"a1b2c3d4-0001\",\"uid\":0,\"op\":{…}}` → BadFrame; лишнее поле внутри op (`\"uid\":0`, `\"path\":\"/x\"`) → BadFrame",
         "неизвестный type `\"worker_kill\"` → BadFrame",
         "v 0 и v 2 → UnsupportedVersion",
         "id \"short\", \"UPPER-CASE-1\", \"a b c d e f g h\", 65 символов → BadId",
         "instance \"\", \"-x\", \"A\", \"a/b\", \"../x\", 33 символа → BadInstance",
         "WorkerReload generation 5 next 5 и next 4 → BadArgument; next 6 → Ok",
         "AppLaunch program \"firefox\", \"/usr/../bin/sh\", с NUL → BadArgument; 65 аргументов → BadArgument; аргумент 4097 байт → BadArgument",
         "read_frame: поток «кадр на 70000 байт\\n» + корректный кадр → первый TooLarge, второй читается целиком",
         "read_frame: пустой поток → Ok(None); «abc» без \\n → BadFrame",
         "encode(reply) оканчивается одним \\n и не содержит \\n внутри",
         "мутация: 100000 кадров из корректного образца с детерминированным PRNG (xorshift, seed 1) — замена, вставка, удаление байта: decode не паникует и возвращает Ok либо один из кодов BadFrame, TooLarge, UnsupportedVersion, BadId, BadInstance, BadArgument",
         "request_digest одинаков для одинаковых op и различен для WorkerStart{browser,7} и WorkerStart{browser,8}; не зависит от id",
     ])
edge("C03", "I06.P1", "I06.P3",
     "У каждого класса операций своё действие polkit; `manage` не используется.",
     "`tests/audit_i06_protocol.rs` и существующий тест политики в `src/main.rs`.",
     [
         "action_for(Status) == \"io.github.cm.status\"; Worker → \"io.github.cm.worker\"; Net → \"io.github.cm.net\"; App → \"io.github.cm.app\"",
         "сгенерированная политика содержит `<action id=\"io.github.cm.worker\">` с allow_active yes, `io.github.cm.net` с allow_active auth_admin_keep и allow_any auth_admin, `io.github.cm.app` с allow_active yes",
         "у каждого нового действия есть message на en и переводы ru, de, it, zh, ar",
     ])
edge("C04", "I06.P3", "I06.W2",
     "Проверка прав получает личность peer целиком и не знает uid из запроса; реализация pkcheck одна.",
     "`tests/audit_i06_protocol.rs` (подменный Authorizer) и `tests/audit_contracts.rs`-подобная проверка исходников.",
     [
         "PolkitAuthorizer: peer uid 0 → Ok без запуска pkcheck (PATH без pkcheck)",
         "test_mode и CM_HELPER_ALLOW=1 → Ok; CM_HELPER_ALLOW=1 без test_mode → pkcheck вызывается (PATH с подставным pkcheck, который пишет аргументы в файл и выходит 1) → Denied",
         "подставной pkcheck получает `--action-id io.github.cm.worker --process <pid>,<start>,<uid>` с pid и start_time из PeerIdentity",
         "в `src/helper/*.rs` нет строки `Command::new(\"pkcheck\")`; она есть ровно в одном файле — `src/controller/actions.rs`",
         "существующие тесты helper (gate) проходят без изменений",
     ])
edge("C05", "I06.I1", "I06.I2",
     "uid, gid и pid берутся только из SO_PEERCRED; подмена PID после захвата обнаруживается.",
     "`tests/audit_i06_peer.rs`: socketpair и дочерние процессы.",
     [
         "capture на socketpair внутри одного процесса → uid == euid, pid == getpid(), start_time == поле 22 /proc/self/stat",
         "parse_start_time(\"1234 (a b) c) S 1 1 1 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 987654 0 0\") → Some(987654)",
         "parse_cgroup(\"0::/user.slice/user-1000.slice/session-3.scope\\n\") → Some(\"/user.slice/user-1000.slice/session-3.scope\"); пустой текст → None",
         "session_of(\"/user.slice/user-1000.slice/session-3.scope\") → Some(\"3\"); session_of(\"/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope\") → None",
         "netns_inode == st_ino /proc/self/ns/net",
         "дочерний процесс открывает сокет и завершается; после wait: identity.alive() == false, verify() → Err(PeerChanged)",
         "живой дочерний процесс: alive() == true, verify() == Ok",
         "format!(\"{:?}\", identity) не содержит пути cgroup",
     ])
edge("C06", "I06.I1", "I06.W2",
     "Диспетчер проверяет личность перед каждой операцией, а не один раз на соединение.",
     "`tests/audit_i06_dispatch.rs`.",
     [
         "peer завершился между двумя кадрами одного соединения → второй ответ code \"peer_changed\", фабрика адаптеров не вызвана",
     ])
edge("C07", "I06.I2", "I06.W1",
     "Каталог, конфиг и имя юнита однозначно определяются uid peer и именем экземпляра; чужой файл не читается.",
     "`tests/audit_i06_owner.rs` в `TempDirGuard`.",
     [
         "owned(\"/b\", 1000, \"browser\") → root \"/b/u1000\", unit \"cm-core-u1000-browser.service\"",
         "config_path(7) == \"/b/u1000/instances/browser/config/gen-7.json\"; journal_path == \"/b/u1000/journal.jsonl\"",
         "owned(base, 1000, \"../x\") → Err(BadInstance)",
         "read_owned_config: обычный файл 0600 владельца → Ok с теми же байтами",
         "файл — симлинк на существующий файл → InvalidConfig; каталог config — симлинк → InvalidConfig",
         "права 0666 → InvalidConfig; размер MAX_CONFIG_BYTES + 1 → InvalidConfig; файла нет → InvalidConfig",
         "владелец файла ≠ owned.uid (owned с uid euid+1 при том же файле) → InvalidConfig",
         "render_user_core_unit(\"/var/lib/cm/users/u1000\", 1000, browser, default) содержит строки `User=1000`, `ReadWritePaths=-/var/lib/cm/users/u1000/instances/browser`, `RuntimeDirectory=cm-core-u1000-browser`",
         "render_core_unit для того же id не изменился (нет `User=`)",
     ])
edge("C08", "I06.H1", "I06.I3",
     "Дочерний процесс не наследует дескрипторы, capabilities и возможность их вернуть.",
     "`tests/audit_i06_harden.rs`: дочерний `/bin/sh -c` печатает /proc/self/status и список /proc/self/fd.",
     [
         "родитель открыл файл без O_CLOEXEC (fd ≥ 3): в дочернем /proc/self/fd только 0, 1, 2 и дескриптор самого ls",
         "keep_fds [N]: дескриптор N в дочернем открыт, остальные ≥ 3 закрыты",
         "в /proc/self/status дочернего: `NoNewPrivs:\\t1`, `CapAmb:\\t0000000000000000`, `CapBnd:\\t0000000000000000`",
         "`ulimit -n` в дочернем → 4096; `ulimit -c` → 0",
         "secret_fd(b\"secret-marker\"): чтение даёт те же 13 байт; запись → ошибка EPERM; F_GET_SEALS содержит WRITE, GROW, SHRINK, SEAL",
         "secret_fd: флаг FD_CLOEXEC установлен",
     ])
edge("C09", "I06.I3", "I06.W1",
     "Процесс worker и приложения работает под uid, gid и группами вызывающего и не может вернуть root.",
     "`tests/audit_i06_drop.rs`: внутри `unshare --user --map-users` с подчинённым uid, как `tests/audit_i04_tun.rs`.",
     [
         "spawn_as(RunAs{uid 1, gid 1, groups [1]}, /usr/bin/id, [\"-u\"]) → вывод \"1\"; `id -g` → \"1\"; `id -G` → \"1\"",
         "дочерний `sh -c 'cat /proc/self/status'`: `CapEff:\\t0000000000000000`, `NoNewPrivs:\\t1`",
         "env: передан {A=1} при родителе с SECRET_TOKEN=x → в дочернем `env` только A=1",
         "spawn_as с program \"id\" (не абсолютный) → Err(BadArgument)",
         "MihomoWorker без set_run_as: существующий `audit_i04_lifecycle` проходит без изменений",
         "MihomoWorker с set_run_as(uid 1): процесс ядра в harness имеет Uid 1 в /proc/<pid>/status (L2, с CM_TEST_MIHOMO)",
     ], level="L2")
edge("C10", "I06.T1", "I06.T2",
     "Журнал переживает обрыв записи и восстанавливает состояние каждой транзакции.",
     "`tests/audit_i06_journal.rs` в `TempDirGuard`.",
     [
         "open создаёт файл 0600; путь-симлинк → Err(Failed)",
         "Begin → Pending; Begin, Step(a), Commit(reply \"R\") → Committed, steps_done [a], reply Some(\"R\")",
         "Begin, Step(a), Abort → Aborted; Begin, Step(a), Compensated → Aborted; Begin, CompensationFailed → Dirty",
         "к файлу дописано `{\"txn\":\"x\",\"ev` без \\n: replay возвращает прежние транзакции; после open + append файл состоит только из целых строк",
         "две транзакции вперемешку: replay в порядке их Begin",
         "compact при файле > 1 МиБ из 5000 Committed и 2 Pending: остаются 2 Pending и 256 последних Committed; размер < 1 МиБ",
     ])
edge("C11", "I06.T2", "I06.W1",
     "При отказе шага выполненное откатывается в обратном порядке; обрыв процесса оставляет след, по которому reconcile завершает откат.",
     "`tests/audit_i06_txn.rs`: шаги-счётчики, записывающие порядок вызовов.",
     [
         "шаги A, B, C успешны → вызовы [A.apply, B.apply, C.apply], журнал Begin, Step A, Step B, Step C, Commit; результат Ok",
         "B.apply → Err(Timeout): вызовы [A.apply, B.apply, A.compensate], журнал Begin, Step A, Abort; результат Err(Timeout)",
         "C.apply → Err(Failed): компенсации в порядке [B.compensate, A.compensate]",
         "B.apply ошибка и A.compensate ошибка → журнал … CompensationFailed, результат Err(Failed), статус Dirty",
         "crash_before(\"B\"): вызовы [A.apply], результат Err(Crashed), журнал Begin, Step A — статус Pending",
         "после этого reconcile: вызовы [A.compensate], возвращает 1, статус Aborted; повторный reconcile возвращает 0 и ничего не вызывает",
         "crash_before(\"commit\") после трёх шагов → Pending с steps_done [A, B, C]; reconcile вызывает [C.compensate, B.compensate, A.compensate]",
         "обрыв на каждом из шагов и на commit (4 точки): после reconcile ни одной Pending",
     ])
edge("C12", "I06.T1", "I06.T3",
     "Повторный запрос с тем же ключом не выполняется второй раз.",
     "`tests/audit_i06_txn.rs`.",
     [
         "нет транзакции → Fresh",
         "Committed с reply \"R\" и тем же digest → Stored(\"R\")",
         "тот же txn, другой digest → Err(Conflict)",
         "Pending с тем же digest → Err(Conflict)",
     ])
edge("C13", "I06.T3", "I06.W1",
     "Устаревшее поколение отвергается; поколение хранится атомарно и по uid.",
     "`tests/audit_i06_txn.rs`.",
     [
         "check_start(None, 1) Ok; check_start(Some(3), 3) Ok; check_start(Some(3), 4) Ok; check_start(Some(3), 2) → GenerationMismatch",
         "check_running(None, 1) → NotRunning; check_running(Some(3), 3) Ok; check_running(Some(3), 2) и (Some(3), 4) → GenerationMismatch",
         "Generations: set(browser, 7), load заново → current(browser) == Some(7); clear → None; файл 0600",
         "оставленный generations.json.tmp с мусором не влияет на load",
     ])
edge("C14", "I06.W1", "I06.W2",
     "Жизненный цикл worker идёт только через контроллер; отказ адаптера не оставляет следов; экземпляры разных uid не видят друг друга.",
     "`tests/audit_i06_core_ops.rs`: `AdapterFactory`, выдающая `core::FakeAdapter`; base в `TempDirGuard`; конфиги `gen-N.json` пишет тест.",
     [
         "start(browser, gen 1) → Started{1}; status → running true, generation Some(1), api \"api_ready\"",
         "повторный start с тем же txn → тот же ответ, фабрика вызвана 1 раз",
         "start(browser, gen 1) с новым txn при работающем → Err(Conflict)",
         "start с gen 0 при текущем 1 (после stop и нового start gen 1) → Err(GenerationMismatch)",
         "нет файла gen-2.json: start gen 2 → Err(InvalidConfig), фабрика не вызвана",
         "reload(gen 1 → 2) → Ok, status generation Some(2); reload(gen 1 → 3) после этого → Err(GenerationMismatch)",
         "адаптер отказал в reload (fail_next Reload InvalidConfig) → Err(InvalidConfig), status generation прежнее, generations.json прежний",
         "stop(gen 2) → Ok; status → running false, generation None; stop ещё раз (новый txn) → Err(NotRunning)",
         "адаптер отказал в start (fail_next Start Failed) → Err(Failed); status running false; generations.json без browser; журнал: транзакция Aborted",
         "17-й экземпляр того же uid → Err(Quota); первый экземпляр другого uid при этом стартует",
         "экземпляр browser uid 1000 запущен: status(owned uid 1001, browser) → running false",
         "CoreError → ControlError: Unsupported → Unsupported, Busy → Busy, RouteUnavailable → Failed",
     ], level="L2")
edge("C15", "I06.W2", "I06.S1",
     "Каждый кадр проходит один и тот же порядок проверок; ответ не содержит ничего кроме кода и данных ответа.",
     "`tests/audit_i06_dispatch.rs`: `handle` с подменными Authorizer и фабрикой.",
     [
         "мусорный кадр → `{\"v\":1,\"id\":\"\",\"ok\":false,\"code\":\"bad_frame\",\"data\":null}`",
         "Authorizer отказывает → code \"denied\"; фабрика не вызвана; каталог владельца не создан",
         "Authorizer получает действие \"io.github.cm.worker\" для worker_start и \"io.github.cm.status\" для worker_status",
         "worker_start от peer uid U читает конфиг только из <base>/u<U>/…; файл в <base>/u<U+1>/… с тем же именем не открывается (проверка: его нет в ответе и нет обращения — конфиг U отсутствует → invalid_config)",
         "net_apply и app_launch при разрешающем Authorizer → code \"unsupported\"; при отказывающем → \"denied\" (права проверяются раньше)",
         "MAX_CONCURRENT_OPS: 4 операции заняты (фабрика блокируется на барьере) → пятый кадр code \"busy\"; после освобождения — проходит",
         "ни один ответ не содержит подстрок base-каталога, \"gen-\", \"/proc\", имени пользователя",
         "reconcile → code \"ok\", data {type reconciled, compensated N}",
     ])
edge("C16", "I06.S1", FINAL,
     "Сервис на сокете выполняет полный сценарий worker и выдерживает отрицательные проверки.",
     "`tests/audit_i06_server.rs`: бинарник `cm controller serve` в `unshare -rn` с `CM_STATE_DIR`, `CM_HELPER_ALLOW=1`, `CM_CORE_BIN=$CM_TEST_MIHOMO`; клиент — unix-сокет из теста.",
     [
         "worker_start(gen 1) с конфигом из `audit_i04_lifecycle` → ok started; worker_status → running true, api \"api_ready\"; worker_stop → ok; после stop нет процессов ядра и аренд",
         "тот же id повторно → тот же ответ, второй процесс ядра не появился",
         "kill -9 контроллера посреди worker_start (точка `CM_CONTROLLER_CRASH_BEFORE=record_generation`, читается только в test_mode) → перезапуск, reconcile → compensated 1, процессов ядра нет, поколение не записано",
         "устаревший запрос: worker_stop(gen 1) после reload до gen 2 → generation_mismatch, worker продолжает работать",
         "argv/path-инъекция: instance \"x;rm -rf\", \"$(id)\", \"..\" → bad_instance; ни одного нового файла вне base",
         "кадр 1 МиБ без \\n → too_large и закрытие соединения; сервис продолжает принимать новые соединения",
         "33-е одновременное соединение закрывается сразу; первые 32 работают",
         "SIGTERM сервису при работающем worker → код выхода 0, процессов ядра нет, сокет удалён",
         "без test_mode и не от root: `cm controller serve --socket X` → код 4",
         "чужой uid (L2, `unshare --user --map-users` с двумя uid): клиент uid B не может остановить worker uid A — worker_stop(browser) → not_running, worker A работает; в <base>/u<A> нет файлов, созданных от имени B",
         "за весь прогон в выводе сервиса и ответах нет содержимого конфига (маркер `secret-marker-i06` в конфиге)",
     ], level="L2")

CFG = dict(
    code="I06",
    title="Рубеж I06 «Привилегированный контроллер»",
    date="2026-10-10",
    source="i06_dag.py",
    final=FINAL,
    links="Решение Q08: [ADR-CONTROLLER.md](ADR-CONTROLLER.md). Адаптер ядра: [I04-REPORT.md](I04-REPORT.md).",
    dag_intro=[
        "Связь с общим планом:",
        "- вершины заменяют прежние I06.T01.b–T04.a;",
        "- приёмка сохраняет id `I06.T05.a`, от неё зависят I07.T01.a и I08.T02.a (критический путь).",
        "",
        "Что входит: протокол, личность peer, владение по uid, запуск от имени пользователя, транзакции, жизненный цикл worker через `CoreAdapter`, вход `cm controller serve`.",
        "",
        "Что не входит:",
        "- содержание операций `net_*` (I07) и `app_launch` (I11): контроллер их принимает, проверяет права и отвечает `unsupported`;",
        "- установка юнита и сокета в систему.",
    ],
    code_task=[
        "Реализовать модуль `src/controller/` и вход `cm controller serve` целиком: все вершины ниже, в порядке зависимостей.",
        "",
        "Контроллер — привилегированная сторона CM:",
        "- принимает типизированные кадры на unix-сокете;",
        "- устанавливает, кто просит, только по данным ядра (`SO_PEERCRED`, pidfd);",
        "- проверяет права через polkit;",
        "- управляет worker-ами ядра через `CoreAdapter` как транзакциями с журналом.",
        "",
        "Требование Q16: два пользователя на одной машине. Один не читает конфиги другого и не управляет его worker-ами. Поэтому в кадре нет uid и путей: всё вычисляется из сокета.",
    ],
    rules_code="""\
1. **Перед сдачей — полный gate, EXIT=0:**
   `CARGO_TARGET_DIR=$HOME/.cache/cm-grok-target CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo sh tests/check_audit.sh`.
2. **Переформатирование существующих файлов запрещено.**
   - Запрещены `cargo fmt` и `rustfmt` по любому существующему файлу. `rustfmt src/main.rs` форматирует ещё `tui.rs` и `tui/*`, `rustfmt src/lib.rs` — весь крейт, `rustfmt src/helper/mod.rs` — весь helper.
   - В diff существующих файлов (`src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`, `src/helper/auth.rs`, `src/core/unit.rs`, `src/core/mihomo/lifecycle.rs`, `tests/check_audit.sh`) — только смысловые строки.
   - `rustfmt --edition 2024` разрешён только для новых файлов `src/controller/*.rs`; добавить `src/controller` в `find` строки `NEW_MODULE_FMT` в `tests/check_audit.sh`. `src/core/mihomo/lifecycle.rs` уже в списке проверки формата, а `src/core/unit.rs` сейчас проходит `rustfmt --check`: после правки оба обязаны её проходить (`unit.rs` добавить в список).
   - Проверка перед сдачей: `git diff --stat` по существующим файлам — десятки строк, не сотни.
3. Сборка только с `--offline --locked`. Новых зависимостей нет: `libc`, `serde`, `serde_json`, `sha2` уже в `Cargo.toml`.
4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога `--base` теста. Настоящий `pkcheck` в тестах не вызывается.
5. Каждый `unsafe` — с `// SAFETY:` (gate проверяет). В `pre_exec` — только async-signal-safe вызовы.
6. **Тесты пишет роль 2 следующей итерацией.** От тебя — код, который проходит контракты рёбер ниже (точные значения, коды, порядок проверок), и сохранность существующего gate. Новые тестовые файлы не нужны.
7. **Никаких данных запроса в ответах и ошибках.** Ответ — код из фиксированного набора и `ReplyData`. Пути, argv, env, байты конфига и тексты ошибок ОС не попадают ни в ответ, ни в `Debug`, ни в вывод процесса.
8. **uid — только из сокета.** Любое место, где uid, путь или каталог владельца берётся из тела запроса, — отказ при ревью.
9. Коммит — только по поручению пользователя. Сдача — отчёт.""",
    tests_task=[
        "Написать за один проход проверки всех рёбер:",
        "- unit и fixture-тесты (L1);",
        "- тесты с дочерними процессами и подчинённым uid (L2);",
        "- сценарий сервиса на сокете с настоящим ядром и отрицательные проверки (ребро C16).",
        "Значения в рёбрах — ожидаемые результаты.",
    ],
    rules_tests="""\
1. **Код продукта не менять.**
2. **Дефект — это красный тест плюс запись.** Если код не выполняет контракт, тест остаётся красным, а в отчёте пишется дефект `Cxx: ожидалось …, получено …`. Исправление — следующая итерация роли 1. Найденный обход (чужой uid, подмена PID, утечка) — дефект P0, он идёт первым в отчёте.
3. Тесты лежат в `tests/audit_i06_*.rs`. Новые файлы добавить в `rustfmt --check` в `tests/check_audit.sh`; существующие файлы не переформатировать (никаких `cargo fmt` и `rustfmt` по старым файлам).
4. Временные каталоги — `TempDirGuard`. Сетевые сценарии и сценарии с ядром — только внутри `unshare -rn`; сценарии с двумя uid — `unshare --user --map-users`, как в `tests/audit_i04_tun.rs`. Без `CM_TEST_MIHOMO` проверки с ядром печатают SKIPPED и не засчитываются.
5. Настоящий `pkcheck` не вызывается: подставной исполняемый файл в `PATH` теста либо подменный `Authorizer`.
6. Значения в рёбрах ниже — ожидаемые результаты, а не примеры. Тест сравнивает именно их.
7. **Evidence:** `docs/design/cm-network-manager/i06-evidence/<дата>/summary.json` — rev, rustc, команды, счётчики, PASS/FAIL по каждому ребру. SKIPPED и NOT_APPLICABLE не записываются как PASS.
8. Fuzz `cargo-fuzz` на час требует сети для установки и в этой итерации не выполняется: его заменяет детерминированная мутация из C02. В отчёте это записывается как открытый пункт, а не как PASS.
9. Полный gate перед сдачей; красные тесты дефектов перечисляются в отчёте поимённо.""",
    tests_report=[
        "- Отчёт: таблица «ребро → PASS/FAIL → тест», список дефектов в формате `Cxx: ожидалось …, получено …`, обходы — первыми.",
        "- `i06-evidence/<дата>/summary.json`.",
    ],
    final_done="Все рёбра I06-DAG проверены, обходов нет, evidence записан.",
)


def main():
    kit.check(V, E, FINAL)
    if "--check" in sys.argv:
        print(f"I06: {len(V)} вершин, {len(E)} рёбер — граф корректен")
        return
    kit.write(OUT, DIRECTIONS, V, E, CFG)


if __name__ == "__main__":
    main()
