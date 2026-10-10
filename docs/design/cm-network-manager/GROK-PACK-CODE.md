# Grok: Единый пакет: контроллер, ядра, сеть приложения, запуск, регион, качество, интерфейс — весь код за один проход

Дата: 2026-10-10. Координатор: Claude. Граф и контракты: [PACK-DAG.md](PACK-DAG.md). Основания: [ADR-CONTROLLER.md](ADR-CONTROLLER.md), [ADR-WORKER-TOPOLOGY.md](ADR-WORKER-TOPOLOGY.md), [ADR-TUN-OWNERSHIP.md](ADR-TUN-OWNERSHIP.md), [I05-GAPS.md](I05-GAPS.md), [ADR-BROWSER-IDENTITY.md](ADR-BROWSER-IDENTITY.md), [ADR-PACKET-FLOW.md](ADR-PACKET-FLOW.md).

## Задача

Реализовать все вершины ниже в порядке зависимостей. Это пять новых модулей (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`), два подмодуля ядер (`src/core/delivery/`, `src/core/xray/`), представление статуса и небольшие правки существующих файлов.

Порядок работы и сдачи — волнами, каждая волна собирается и проходит gate отдельно:
1. **Контроллер:** I06.P1 … I06.S1.
2. **Ядра:** I05.D1 … I05.D4, I05.X1, I05.X2.
3. **Сеть приложения:** I08.N1 … I08.N6, I09.N7, I09.N8.
4. **Приложения:** I11.A1 … I11.A5, I12.A6, I12.A7.
5. **Чистые модули:** I14.R1 … R3, I16.Q1, Q2, I17.U1, U2 — не зависят от волн 1–4 и могут идти первыми.

Если волна упирается в противоречие спецификации, остановись на ней, сдай сделанное и опиши противоречие. Не придумывай обходной путь в P0-коде (контроллер, сеть).

Тесты пишет роль 2 по рёбрам. Поэтому точные значения из рёбер — часть спецификации, а не пожелание.

## Правила

1. **Перед сдачей — полный gate, EXIT=0:**
   `CARGO_TARGET_DIR=$HOME/.cache/cm-grok-target CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo sh tests/check_audit.sh`.
2. **Переформатирование существующих файлов запрещено.**
   - Запрещены `cargo fmt` и `rustfmt` по любому существующему файлу. `rustfmt src/main.rs` форматирует ещё `tui.rs` и `tui/*`, `rustfmt src/lib.rs` — весь крейт, `rustfmt src/helper/mod.rs` — весь helper.
   - В diff существующих файлов (`src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`, `src/helper/auth.rs`, `src/core/mod.rs`, `src/core/unit.rs`, `src/core/leases.rs`, `src/core/mihomo/lifecycle.rs`, `src/core/mihomo/config.rs`, `src/identity/plan.rs`, `src/status.rs`, `src/tui/mock_tunnels.rs`, `cosmic/src/tunnel_icons.rs`, `tests/check_audit.sh`) — только смысловые строки.
   - `rustfmt --edition 2024` разрешён только для новых файлов (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`, `src/core/delivery/`, `src/core/xray/`, `src/core/process.rs`, `src/status/tunnels.rs`); добавить эти каталоги и файлы в проверку формата в `tests/check_audit.sh`. `src/core/mihomo/lifecycle.rs` уже в списке проверки формата, а `src/core/unit.rs` сейчас проходит `rustfmt --check`: после правки оба обязаны её проходить (`unit.rs` добавить в список).
   - Проверка перед сдачей: `git diff --stat` по существующим файлам — десятки строк, не сотни.
3. Сборка только с `--offline --locked`. Новых зависимостей нет: `libc`, `serde`, `serde_json`, `sha2` уже в `Cargo.toml`.
4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога `--base` теста. Настоящий `pkcheck` не вызывается. Команды `ip` и `nft` выполняются только внутри `unshare -rn`; на хосте — никогда. Загрузка ядер из сети в этой итерации не выполняется (только код и подменный `Download`).
5. Каждый `unsafe` — с `// SAFETY:` (gate проверяет). В `pre_exec` — только async-signal-safe вызовы.
6. **Тесты пишет роль 2 следующей итерацией.** От тебя — код, который проходит контракты рёбер ниже (точные значения, коды, порядок проверок), и сохранность существующего gate. Новые тестовые файлы не нужны.
7. **Никаких данных запроса в ответах и ошибках.** Ответ — код из фиксированного набора и `ReplyData`. Пути, argv, env, байты конфига и тексты ошибок ОС не попадают ни в ответ, ни в `Debug`, ни в вывод процесса.
8. **uid — только из сокета.** Любое место, где uid, путь или каталог владельца берётся из тела запроса, — отказ при ревью.
9. Коммит — только по поручению пользователя. Сдача — отчёт.
10. **Сеть — fail-closed по построению.** Любое изменение порядка команд из I08.N2 или текста таблицы из I08.N3 — отказ при ревью: порядок проверен лабораторией.

## Вершины по порядку

### I06.P1 — Описать кадры запроса и ответа контроллера.

Файлы: `src/controller/mod.rs`, `src/controller/protocol.rs`, `src/lib.rs`. Зависит от: —.

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
`Reply::ok(id, data)` → `ok: true, code: "ok"`; `Reply::error(id, error)` → `ok: false, code: error.code(), data: None`. Других текстов в ответе нет.

Контракты, которые проверит роль 2:
- **C01 → I06.P2:** Кадр несёт только версию, ключ и операцию; uid, путей и байтов конфига в схеме нет.
  - `{"v":1,"id":"a1b2c3d4-0001","op":{"type":"worker_start","instance":"browser","generation":7}}` ⇄ Request{v 1, id, WorkerStart{browser, 7}} (roundtrip)
  - классы: WorkerStatus → Status; WorkerStart, WorkerReload, WorkerStop, Reconcile → Worker; NetApply, NetRevert → Net; AppLaunch → App
  - Op::instance(): Reconcile → None, остальные → Some
  - ControlError::code(): BadFrame → "bad_frame", GenerationMismatch → "generation_mismatch", PeerChanged → "peer_changed"; все 18 кодов различны и состоят из [a-z_]
  - Reply::ok("k", Started{generation 7}) → `{"v":1,"id":"k","ok":true,"code":"ok","data":{"type":"started","generation":7}}`
  - Reply::error("k", Denied) → ok false, code "denied", data null
- **C03 → I06.P3:** У каждого класса операций своё действие polkit; `manage` не используется.
  - action_for(Status) == "io.github.cm.status"; Worker → "io.github.cm.worker"; Net → "io.github.cm.net"; App → "io.github.cm.app"
  - сгенерированная политика содержит `<action id="io.github.cm.worker">` с allow_active yes, `io.github.cm.net` с allow_active auth_admin_keep и allow_any auth_admin, `io.github.cm.app` с allow_active yes
  - у каждого нового действия есть message на en и переводы ru, de, it, zh, ar

### I06.P2 — Декодировать и кодировать кадры с жёсткими пределами.

Файлы: `src/controller/codec.rs`. Зависит от: I06.P1.

`pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, ControlError>`
- Читает до `\n`, не больше `MAX_FRAME_BYTES + 1` байт. Длиннее → `TooLarge`; хвост до `\n` при этом вычитывается порциями без накопления, чтобы следующее чтение началось с нового кадра.
- EOF без данных → `Ok(None)`; EOF посреди кадра → `BadFrame`.
- Возвращает кадр без завершающего `\n`.

`pub fn decode(frame: &[u8]) -> Result<Request, ControlError>` — порядок проверок:
1. длина > `MAX_FRAME_BYTES` → `TooLarge`;
2. не UTF-8, не JSON, неизвестное поле или неизвестный `type` → `BadFrame`;
3. `v != CONTROL_VERSION` → `UnsupportedVersion`;
4. `id` не `^[a-z0-9-]{8,64}$` → `BadId`;
5. `instance` не проходит `core::InstanceId::new` → `BadInstance`;
6. `WorkerReload`: `next_generation <= generation` → `BadArgument`;
7. `AppLaunch`: `program` не абсолютный путь, содержит NUL или `..`-сегмент; `args` больше 64 штук, аргумент длиннее 4096 байт или содержит NUL → `BadArgument`.

`pub fn encode(reply: &Reply) -> Vec<u8>` — одна строка JSON и `\n`; внутри строки нет `\n`.

`pub fn request_digest(request: &Request) -> String` — sha256 (hex) канонического JSON поля `op` (сериализация serde_json без пробелов); нужен идемпотентности (I06.T3).

Контракты, которые проверит роль 2:
- **C02 → I06.W2:** Декодер отвергает всё, что не является точным кадром версии 1, и не читает больше 64 КиБ.
  - кадр длиной 65537 байт → TooLarge; ровно 65536 байт корректного JSON с длинным id → BadId (не TooLarge)
  - `not json`, `{}`, `[]`, байты 0xFF → BadFrame
  - лишнее поле: `{"v":1,"id":"a1b2c3d4-0001","uid":0,"op":{…}}` → BadFrame; лишнее поле внутри op (`"uid":0`, `"path":"/x"`) → BadFrame
  - неизвестный type `"worker_kill"` → BadFrame
  - v 0 и v 2 → UnsupportedVersion
  - id "short", "UPPER-CASE-1", "a b c d e f g h", 65 символов → BadId
  - instance "", "-x", "A", "a/b", "../x", 33 символа → BadInstance
  - WorkerReload generation 5 next 5 и next 4 → BadArgument; next 6 → Ok
  - AppLaunch program "firefox", "/usr/../bin/sh", с NUL → BadArgument; 65 аргументов → BadArgument; аргумент 4097 байт → BadArgument
  - read_frame: поток «кадр на 70000 байт\n» + корректный кадр → первый TooLarge, второй читается целиком
  - read_frame: пустой поток → Ok(None); «abc» без \n → BadFrame
  - encode(reply) оканчивается одним \n и не содержит \n внутри
  - мутация: 100000 кадров из корректного образца с детерминированным PRNG (xorshift, seed 1) — замена, вставка, удаление байта: decode не паникует и возвращает Ok либо один из кодов BadFrame, TooLarge, UnsupportedVersion, BadId, BadInstance, BadArgument
  - request_digest одинаков для одинаковых op и различен для WorkerStart{browser,7} и WorkerStart{browser,8}; не зависит от id

### I06.P3 — Сопоставить классы операций с действиями polkit и проверять права.

Файлы: `src/controller/actions.rs`, `src/helper/auth.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: I06.P1.

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
  В существующих строках — только добавление элементов списка.

Контракты, которые проверит роль 2:
- **C04 → I06.W2:** Проверка прав получает личность peer целиком и не знает uid из запроса; реализация pkcheck одна.
  - PolkitAuthorizer: peer uid 0 → Ok без запуска pkcheck (PATH без pkcheck)
  - test_mode и CM_HELPER_ALLOW=1 → Ok; CM_HELPER_ALLOW=1 без test_mode → pkcheck вызывается (PATH с подставным pkcheck, который пишет аргументы в файл и выходит 1) → Denied
  - подставной pkcheck получает `--action-id io.github.cm.worker --process <pid>,<start>,<uid>` с pid и start_time из PeerIdentity
  - в `src/helper/*.rs` нет строки `Command::new("pkcheck")`; она есть ровно в одном файле — `src/controller/actions.rs`
  - существующие тесты helper (gate) проходят без изменений

### I06.I1 — Установить личность процесса на другом конце сокета.

Файлы: `src/controller/peer.rs`. Зависит от: —.

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

`Debug` для `PeerIdentity` не печатает `cgroup` целиком (только uid, pid, session). Чтение uid из любого другого источника запрещено.

Контракты, которые проверит роль 2:
- **C05 → I06.I2:** uid, gid и pid берутся только из SO_PEERCRED; подмена PID после захвата обнаруживается.
  - capture на socketpair внутри одного процесса → uid == euid, pid == getpid(), start_time == поле 22 /proc/self/stat
  - parse_start_time("1234 (a b) c) S 1 1 1 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 987654 0 0") → Some(987654)
  - parse_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n") → Some("/user.slice/user-1000.slice/session-3.scope"); пустой текст → None
  - session_of("/user.slice/user-1000.slice/session-3.scope") → Some("3"); session_of("/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope") → None
  - netns_inode == st_ino /proc/self/ns/net
  - дочерний процесс открывает сокет и завершается; после wait: identity.alive() == false, verify() → Err(PeerChanged)
  - живой дочерний процесс: alive() == true, verify() == Ok
  - format!("{:?}", identity) не содержит пути cgroup
- **C06 → I06.W2:** Диспетчер проверяет личность перед каждой операцией, а не один раз на соединение.
  - peer завершился между двумя кадрами одного соединения → второй ответ code "peer_changed", фабрика адаптеров не вызвана

### I06.I2 — Вычислить каталог, файл конфига и имя юнита владельца из uid peer.

Файлы: `src/controller/owner.rs`, `src/core/unit.rs`. Зависит от: I06.I1.

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

`render_core_unit` не менять.

Контракты, которые проверит роль 2:
- **C07 → I06.W1:** Каталог, конфиг и имя юнита однозначно определяются uid peer и именем экземпляра; чужой файл не читается.
  - owned("/b", 1000, "browser") → root "/b/u1000", unit "cm-core-u1000-browser.service"
  - config_path(7) == "/b/u1000/instances/browser/config/gen-7.json"; journal_path == "/b/u1000/journal.jsonl"
  - owned(base, 1000, "../x") → Err(BadInstance)
  - read_owned_config: обычный файл 0600 владельца → Ok с теми же байтами
  - файл — симлинк на существующий файл → InvalidConfig; каталог config — симлинк → InvalidConfig
  - права 0666 → InvalidConfig; размер MAX_CONFIG_BYTES + 1 → InvalidConfig; файла нет → InvalidConfig
  - владелец файла ≠ owned.uid (owned с uid euid+1 при том же файле) → InvalidConfig
  - render_user_core_unit("/var/lib/cm/users/u1000", 1000, browser, default) содержит строки `User=1000`, `ReadWritePaths=-/var/lib/cm/users/u1000/instances/browser`, `RuntimeDirectory=cm-core-u1000-browser`
  - render_core_unit для того же id не изменился (нет `User=`)

### I06.H1 — Подготовить дочерний процесс: дескрипторы, пределы, capabilities, секреты через fd.

Файлы: `src/controller/harden.rs`. Зависит от: —.

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

`secret_fd`: `memfd_create("cm-secret", MFD_CLOEXEC | MFD_ALLOW_SEALING)`, запись всех байтов, `lseek` в начало, печати `F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL`. Секрет не попадает ни в argv, ни в env.

Контракты, которые проверит роль 2:
- **C08 → I06.I3:** Дочерний процесс не наследует дескрипторы, capabilities и возможность их вернуть.
  - родитель открыл файл без O_CLOEXEC (fd ≥ 3): в дочернем /proc/self/fd только 0, 1, 2 и дескриптор самого ls
  - keep_fds [N]: дескриптор N в дочернем открыт, остальные ≥ 3 закрыты
  - в /proc/self/status дочернего: `NoNewPrivs:\t1`, `CapAmb:\t0000000000000000`, `CapBnd:\t0000000000000000`
  - `ulimit -n` в дочернем → 4096; `ulimit -c` → 0
  - secret_fd(b"secret-marker"): чтение даёт те же 13 байт; запись → ошибка EPERM; F_GET_SEALS содержит WRITE, GROW, SHRINK, SEAL
  - secret_fd: флаг FD_CLOEXEC установлен

### I06.I3 — Запускать процесс от имени вызывающего пользователя, а не root.

Файлы: `src/controller/drop.rs`, `src/core/mihomo/lifecycle.rs`. Зависит от: I06.H1.

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

`src/core/mihomo/lifecycle.rs`: у `MihomoWorker` новый метод `pub fn set_run_as(&mut self, run_as: RunAs)`. Если он задан, `spawn` вызывает `drop_pre_exec` вместо нынешнего `pre_exec` с одним `setsid` (сам `setsid` остаётся первым шагом). Без `set_run_as` поведение прежнее. В diff `lifecycle.rs` — только эти строки.

Контракты, которые проверит роль 2:
- **C09 → I06.W1:** Процесс worker и приложения работает под uid, gid и группами вызывающего и не может вернуть root.
  - spawn_as(RunAs{uid 1, gid 1, groups [1]}, /usr/bin/id, ["-u"]) → вывод "1"; `id -g` → "1"; `id -G` → "1"
  - дочерний `sh -c 'cat /proc/self/status'`: `CapEff:\t0000000000000000`, `NoNewPrivs:\t1`
  - env: передан {A=1} при родителе с SECRET_TOKEN=x → в дочернем `env` только A=1
  - spawn_as с program "id" (не абсолютный) → Err(BadArgument)
  - MihomoWorker без set_run_as: существующий `audit_i04_lifecycle` проходит без изменений
  - MihomoWorker с set_run_as(uid 1): процесс ядра в harness имеет Uid 1 в /proc/<pid>/status (L2, с CM_TEST_MIHOMO)
- **X01 → I05.X2:** Xray запускается под uid вызывающего тем же механизмом, что mihomo.
  - XrayWorker с set_run_as(uid 1): процесс ядра имеет Uid 1 и CapEff 0 в /proc/<pid>/status
  - без set_run_as поведение прежнее (uid запускающего)
- **X06 → I11.A2:** План запуска несёт RunAs вызывающего без изменений.
  - plan(..., RunAs{uid 1000, gid 1000, groups [1000, 998]}, ...).run_as == тот же RunAs

### I06.T1 — Вести журнал владения с дозаписью и восстановлением после обрыва.

Файлы: `src/controller/journal.rs`. Зависит от: —.

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
- **Оборванная последняя строка** (нет `\n` или не JSON) при `replay` пропускается; при `open` файл усекается до конца последней целой строки.
- `compact`: если файл больше 1 МиБ — переписать атомарно (`.tmp`, fsync, rename), оставив все `Pending` и `Dirty` целиком и последние 256 `Committed`. Вызывается из `append` после записи `Commit`.

Контракты, которые проверит роль 2:
- **C10 → I06.T2:** Журнал переживает обрыв записи и восстанавливает состояние каждой транзакции.
  - open создаёт файл 0600; путь-симлинк → Err(Failed)
  - Begin → Pending; Begin, Step(a), Commit(reply "R") → Committed, steps_done [a], reply Some("R")
  - Begin, Step(a), Abort → Aborted; Begin, Step(a), Compensated → Aborted; Begin, CompensationFailed → Dirty
  - к файлу дописано `{"txn":"x","ev` без \n: replay возвращает прежние транзакции; после open + append файл состоит только из целых строк
  - две транзакции вперемешку: replay в порядке их Begin
  - compact при файле > 1 МиБ из 5000 Committed и 2 Pending: остаются 2 Pending и 256 последних Committed; размер < 1 МиБ
- **C12 → I06.T3:** Повторный запрос с тем же ключом не выполняется второй раз.
  - нет транзакции → Fresh
  - Committed с reply "R" и тем же digest → Stored("R")
  - тот же txn, другой digest → Err(Conflict)
  - Pending с тем же digest → Err(Conflict)

### I06.T2 — Выполнять шаги транзакции с компенсацией и восстановлением.

Файлы: `src/controller/txn.rs`. Зависит от: I06.T1.

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

Шаг обязан быть идемпотентным в `compensate`: после обрыва неизвестно, успел ли `apply` шага, перед которым упал процесс.

Контракты, которые проверит роль 2:
- **C11 → I06.W1:** При отказе шага выполненное откатывается в обратном порядке; обрыв процесса оставляет след, по которому reconcile завершает откат.
  - шаги A, B, C успешны → вызовы [A.apply, B.apply, C.apply], журнал Begin, Step A, Step B, Step C, Commit; результат Ok
  - B.apply → Err(Timeout): вызовы [A.apply, B.apply, A.compensate], журнал Begin, Step A, Abort; результат Err(Timeout)
  - C.apply → Err(Failed): компенсации в порядке [B.compensate, A.compensate]
  - B.apply ошибка и A.compensate ошибка → журнал … CompensationFailed, результат Err(Failed), статус Dirty
  - crash_before("B"): вызовы [A.apply], результат Err(Crashed), журнал Begin, Step A — статус Pending
  - после этого reconcile: вызовы [A.compensate], возвращает 1, статус Aborted; повторный reconcile возвращает 0 и ничего не вызывает
  - crash_before("commit") после трёх шагов → Pending с steps_done [A, B, C]; reconcile вызывает [C.compensate, B.compensate, A.compensate]
  - обрыв на каждом из шагов и на commit (4 точки): после reconcile ни одной Pending

### I06.T3 — Хранить поколения и отвечать на повторный запрос сохранённым ответом.

Файлы: `src/controller/registry.rs`. Зависит от: I06.T1.

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
  - `Pending`, `Dirty` или `Aborted` с тем же digest → `Conflict` (сначала нужен `reconcile`).

Контракты, которые проверит роль 2:
- **C13 → I06.W1:** Устаревшее поколение отвергается; поколение хранится атомарно и по uid.
  - check_start(None, 1) Ok; check_start(Some(3), 3) Ok; check_start(Some(3), 4) Ok; check_start(Some(3), 2) → GenerationMismatch
  - check_running(None, 1) → NotRunning; check_running(Some(3), 3) Ok; check_running(Some(3), 2) и (Some(3), 4) → GenerationMismatch
  - Generations: set(browser, 7), load заново → current(browser) == Some(7); clear → None; файл 0600
  - оставленный generations.json.tmp с мусором не влияет на load

### I06.W1 — Управлять жизненным циклом worker через CoreAdapter как транзакциями.

Файлы: `src/controller/core_ops.rs`. Зависит от: I06.T2, I06.T3, I06.I2, I06.I3.

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

**Точка обрыва для проверок.** `crash_before` для `txn::run` в продукте всегда возвращает `false`. Только при `test_mode()` и заданной переменной `CM_CONTROLLER_CRASH_BEFORE=<имя шага или commit>` на этом шаге процесс завершается через `std::process::abort()`. Вне `test_mode()` переменная не читается.

Контракты, которые проверит роль 2:
- **C14 → I06.W2:** Жизненный цикл worker идёт только через контроллер; отказ адаптера не оставляет следов; экземпляры разных uid не видят друг друга.
  - start(browser, gen 1) → Started{1}; status → running true, generation Some(1), api "api_ready"
  - повторный start с тем же txn → тот же ответ, фабрика вызвана 1 раз
  - start(browser, gen 1) с новым txn при работающем → Err(Conflict)
  - start с gen 0 при текущем 1 (после stop и нового start gen 1) → Err(GenerationMismatch)
  - нет файла gen-2.json: start gen 2 → Err(InvalidConfig), фабрика не вызвана
  - reload(gen 1 → 2) → Ok, status generation Some(2); reload(gen 1 → 3) после этого → Err(GenerationMismatch)
  - адаптер отказал в reload (fail_next Reload InvalidConfig) → Err(InvalidConfig), status generation прежнее, generations.json прежний
  - stop(gen 2) → Ok; status → running false, generation None; stop ещё раз (новый txn) → Err(NotRunning)
  - адаптер отказал в start (fail_next Start Failed) → Err(Failed); status running false; generations.json без browser; журнал: транзакция Aborted
  - 17-й экземпляр того же uid → Err(Quota); первый экземпляр другого uid при этом стартует
  - экземпляр browser uid 1000 запущен: status(owned uid 1001, browser) → running false
  - CoreError → ControlError: Unsupported → Unsupported, Busy → Busy, RouteUnavailable → Failed

### I06.W2 — Провести кадр через проверки и вернуть ответ без утечек.

Файлы: `src/controller/dispatch.rs`. Зависит от: I06.P2, I06.P3, I06.I1, I06.W1.

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

Ответ — только `code` из фиксированного набора и `ReplyData`. Ни путь, ни argv, ни текст ошибки ОС в ответ и в журнал процесса не попадают.

Контракты, которые проверит роль 2:
- **C15 → I06.S1:** Каждый кадр проходит один и тот же порядок проверок; ответ не содержит ничего кроме кода и данных ответа.
  - мусорный кадр → `{"v":1,"id":"","ok":false,"code":"bad_frame","data":null}`
  - Authorizer отказывает → code "denied"; фабрика не вызвана; каталог владельца не создан
  - Authorizer получает действие "io.github.cm.worker" для worker_start и "io.github.cm.status" для worker_status
  - worker_start от peer uid U читает конфиг только из <base>/u<U>/…; файл в <base>/u<U+1>/… с тем же именем не открывается (проверка: его нет в ответе и нет обращения — конфиг U отсутствует → invalid_config)
  - net_apply и app_launch при разрешающем Authorizer → code "unsupported"; при отказывающем → "denied" (права проверяются раньше)
  - MAX_CONCURRENT_OPS: 4 операции заняты (фабрика блокируется на барьере) → пятый кадр code "busy"; после освобождения — проходит
  - ни один ответ не содержит подстрок base-каталога, "gen-", "/proc", имени пользователя
  - reconcile → code "ok", data {type reconciled, compensated N}
- **X02 → I08.N6:** Операции сети проходят тот же порядок проверок, что операции worker: кадр → личность → права → владелец.
  - net_apply от peer, завершившегося перед кадром → peer_changed, RecordingExec пуст
  - net_apply с лишним полем `"index":5` в op → bad_frame: номер туннеля нельзя задать из запроса
  - net_apply и net_revert больше не отвечают unsupported
- **X03 → I11.A3:** Запуск приложения — операция класса App со своим действием polkit.
  - Authorizer получает "io.github.cm.app"; при отказе → denied, процесс не создан
  - app_launch больше не отвечает unsupported

### I06.S1 — Дать вход `cm controller serve` на unix-сокете.

Файлы: `src/controller/server.rs`, `src/main.rs`. Зависит от: I06.W2.

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

Интеграция в установку (юнит, сокет systemd, общий процесс с helper) в эту вершину не входит: её решает пользователь при приёмке (см. отчёт).

Контракты, которые проверит роль 2:
- **C16 → I06.T05.a:** Сервис на сокете выполняет полный сценарий worker и выдерживает отрицательные проверки.
  - worker_start(gen 1) с конфигом из `audit_i04_lifecycle` → ok started; worker_status → running true, api "api_ready"; worker_stop → ok; после stop нет процессов ядра и аренд
  - тот же id повторно → тот же ответ, второй процесс ядра не появился
  - kill -9 контроллера посреди worker_start (точка `CM_CONTROLLER_CRASH_BEFORE=record_generation`, читается только в test_mode) → перезапуск, reconcile → compensated 1, процессов ядра нет, поколение не записано
  - устаревший запрос: worker_stop(gen 1) после reload до gen 2 → generation_mismatch, worker продолжает работать
  - argv/path-инъекция: instance "x;rm -rf", "$(id)", ".." → bad_instance; ни одного нового файла вне base
  - кадр 1 МиБ без \n → too_large и закрытие соединения; сервис продолжает принимать новые соединения
  - 33-е одновременное соединение закрывается сразу; первые 32 работают
  - SIGTERM сервису при работающем worker → код выхода 0, процессов ядра нет, сокет удалён
  - без test_mode и не от root: `cm controller serve --socket X` → код 4
  - чужой uid (L2, `unshare --user --map-users` с двумя uid): клиент uid B не может остановить worker uid A — worker_stop(browser) → not_running, worker A работает; в <base>/u<A> нет файлов, созданных от имени B
  - за весь прогон в выводе сервиса и ответах нет содержимого конфига (маркер `secret-marker-i06` в конфиге)
- **X05 → I11.A5:** Клиент говорит с контроллером тем же кадром, что описан в протоколе.
  - controller::client::call шлёт одну строку JSON версии 1 с id из 16 hex-символов и читает одну строку ответа
  - ответ длиннее 64 КиБ или не JSON → ошибка клиента, код выхода `cm app run` 4

### I06.T05.a — Принять контроллер отрицательными проверками.

Файлы: `docs/design/cm-network-manager/i06-evidence/<дата>/summary.json`. Зависит от: I06.S1.

Вершина роли 2: кода нет. Закрывается, когда все рёбра проверены и обходов нет.

Контракты, которые проверит роль 2:
- **Z01 → PACK.Z:** Контроллер принят: отрицательные проверки I06 пройдены до сквозного сценария.
  - все рёбра C01–C16 — PASS; обходов нет

### I05.D1 — Описать закреплённые версии ядер и проверку sha256.

Файлы: `src/core/delivery/mod.rs`, `src/core/delivery/pin.rs`, `src/core/mod.rs`. Зависит от: —.

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

`verify_sha256`: сравнение без учёта регистра hex; длина ≠ 64 или не hex → `HashMismatch`.

Контракты, которые проверит роль 2:
- **K01 → I05.D2:** Пины совпадают с I05-PIN.md, а проверка хеша отвергает любое расхождение.
  - pin(Mihomo).archive_sha256 == "ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20"; binary_sha256 == "7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1"
  - pin(Xray).archive_sha256 == "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae"; binary_sha256 == "8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed"; archive == Zip{member "xray"}
  - каждый url начинается с `https://github.com/` и содержит версию пина
  - verify_sha256(b"{}", "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a") Ok; тот же hex в верхнем регистре Ok; один изменённый символ → HashMismatch; строка 63 символа → HashMismatch
  - с `CM_TEST_MIHOMO`: sha256 файла ядра == pin(Mihomo).binary_sha256; с `CM_TEST_XRAY`: == pin(Xray).binary_sha256 (без переменных — SKIPPED)

### I05.D2 — Распаковать gzip и один файл из zip с пределами размера.

Файлы: `src/core/delivery/unpack.rs`. Зависит от: I05.D1.

```rust
pub fn unpack_gzip(bytes: &[u8], max: u64) -> Result<Vec<u8>, DeliveryError>
pub fn unpack_zip_member(bytes: &[u8], member: &str, max: u64) -> Result<Vec<u8>, DeliveryError>
```
`unpack_gzip`: `flate2::read::GzDecoder`, чтение через `take(max + 1)`; больше `max` → `TooLarge`; ошибка потока → `BadArchive`.

`unpack_zip_member` — собственный минимальный читатель (крейта zip в зависимостях нет, новых не добавлять):
1. найти End of Central Directory (`PK\x05\x06`) в последних 65557 байтах; нет → `BadArchive`;
2. zip64 (любое поле 0xFFFF/0xFFFFFFFF), многотомный архив (disk ≠ 0) → `BadArchive`;
3. пройти центральный каталог (`PK\x01\x02`), не больше 4096 записей; запись с точным именем `member` (без `/`, без `..`) — искомая; нет → `MemberMissing`;
4. флаг шифрования (bit 0) → `BadArchive`; метод только 0 (stored) или 8 (deflate), иначе `BadArchive`;
5. заявленный несжатый размер > `max` → `TooLarge` до распаковки;
6. локальный заголовок (`PK\x03\x04`) по смещению; данные в пределах файла, иначе `BadArchive`;
7. deflate — `flate2::read::DeflateDecoder` через `take(max + 1)`; фактический размер > `max` → `TooLarge`; фактический размер ≠ заявленному → `BadArchive`;
8. CRC-32 результата (`flate2::Crc`) ≠ записанному → `BadArchive`.

Все смещения и длины — с проверкой переполнения и границ; паник на произвольном входе нет.

Контракты, которые проверит роль 2:
- **K02 → I05.D3:** Распаковка не выдаёт больше предела и не паникует на произвольных байтах.
  - gzip из 1000 байт, max 1000 → Ok; max 999 → TooLarge; обрезанный на 10 байт → BadArchive
  - gzip-бомба: 64 МиБ нулей, max 1 МиБ → TooLarge, пик памяти теста не растёт до 64 МиБ (чтение через take)
  - zip с членами `LICENSE` и `xray` (deflate): member "xray" → его байты; member "nope" → MemberMissing
  - zip, где у `xray` заявлен размер 10, а в потоке 20 байт → BadArchive; заявлен размер > max → TooLarge
  - zip с неверным CRC → BadArchive; с флагом шифрования → BadArchive; метод 12 (bzip2) → BadArchive
  - member "../xray" и "a/xray" → MemberMissing; запись в архиве с именем `../xray` не выбирается по запросу "xray"
  - zip без EOCD, пустой вход, 22 байта мусора → BadArchive
  - мутация: 20000 вариантов корректного zip (xorshift, seed 1; замена, вставка, удаление байта) — без паник, результат Ok или один из кодов DeliveryError
  - с `CM_TEST_XRAY_ZIP` (скачанный Xray-linux-64.zip): unpack_zip_member("xray") даёт sha256 == pin(Xray).binary_sha256 (без переменной — SKIPPED)

### I05.D3 — Получить бинарник ядра: загрузка, проверка архива, распаковка, проверка бинарника.

Файлы: `src/core/delivery/fetch.rs`. Зависит от: I05.D2.

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

`TransportDownload`: только `https://`, ошибки транспорта → `Fetch`, превышение предела → `TooLarge`. URL берётся только из `Pin`, не из аргументов пользователя.

Контракты, которые проверит роль 2:
- **K03 → I05.D4:** Наружу выходит только бинарник, прошедший обе проверки хеша.
  - корректный архив → Ok(binary); порядок вызовов: get один раз с max_bytes == pin.max_archive
  - архив с другим содержимым → HashMismatch, распаковка не вызывалась (архив-бомба с неверным хешем не распаковывается)
  - верный хеш архива, но binary_sha256 другой → HashMismatch
  - Download возвращает Fetch → Fetch; TooLarge → TooLarge
  - TransportDownload::get("http://example.invalid/x", 10) → Fetch (не https) без сетевого запроса

### I05.D4 — Установить кандидата атомарно и уметь откатиться.

Файлы: `src/core/delivery/install.rs`. Зависит от: I05.D3.

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

`cm vpn core update` в эту вершину не входит: существующую команду не менять.

Контракты, которые проверит роль 2:
- **K04 → PACK.Z:** Обрыв на любом шаге установки оставляет рабочий `current`; откат возвращает предыдущую версию.
  - install v1 → current указывает на versions/v1/mihomo, файл 0755, previous отсутствует
  - install v2 → current → v2, previous → v1; файл v1 на месте
  - probe возвращает строку без version_marker → VersionMismatch; current по-прежнему v2; каталога `.tmp` нет
  - accept возвращает ошибку → ConfigRejected; current по-прежнему v2
  - rollback → current → v1, previous → v2; ещё один rollback → current → v2
  - rollback без previous → NothingToRollBack
  - current — всегда симлинк; после каждого шага `readlink` даёт существующий исполняемый файл

### I05.X1 — Построить config Xray из узлов Store.

Файлы: `src/core/xray/mod.rs`, `src/core/xray/config.rs`, `src/core/mod.rs`. Зависит от: —.

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

Ошибки не содержат значений узла (адресов, паролей, uuid).

Контракты, которые проверит роль 2:
- **K05 → I05.X2:** Каждый поддержанный узел даёт конфиг, который принимает закреплённый Xray; неподдержанное отклоняется явно.
  - ss {server 203.0.113.8, port 443, cipher aes-128-gcm, password x} → `{"tag":"node","protocol":"shadowsocks","settings":{"servers":[{"address":"203.0.113.8","port":443,"method":"aes-128-gcm","password":"x"}]}}`
  - vless {uuid 11111111-2222-4333-8444-555555555555, tls true, servername example.invalid, network ws, ws-opts {path /p, headers {Host h.example}}} → protocol vless, users[0] {id, encryption none}, streamSettings {network ws, wsSettings {path /p, headers {Host h.example}}, security tls, tlsSettings {serverName example.invalid}}
  - vless с reality-opts {public-key K, short-id S} → security reality, realitySettings {serverName, publicKey K, shortId S, fingerprint chrome}
  - trojan {password x, sni example.invalid} → security tls всегда, tlsSettings.serverName example.invalid
  - vmess без alterId и cipher → alterId 0, security auto
  - type tuic, hysteria2, wireguard, http, socks5 → Err(Unsupported(<type>)); network h2 → Err(Unsupported("network")); dialer-proxy → Err(Unsupported("dialer"))
  - порт 0 или 70000, uuid "x", нет server → Err(InvalidNode); текст ошибки не содержит адреса, пароля, uuid
  - generate_config(&[], 20000) → Empty; generate_config(nodes, 0) → InvalidPort
  - generate_config(ss, 20000): inbounds[0] == {tag cm, listen 127.0.0.1, port 20000, protocol mixed, settings {udp true}}; outbounds теги [node, direct, block]; routing.rules[0] == {type field, inboundTag [cm], outboundTag node}
  - с `CM_TEST_XRAY`: `xray run -test` принимает конфиг для ss, vmess, vless+ws+tls, vless+reality, trojan (без переменной — SKIPPED)

### I05.X2 — Вести жизненный цикл Xray через CoreAdapter.

Файлы: `src/core/xray/lifecycle.rs`. Зависит от: I05.X1, I06.I3.

`pub struct XrayWorker` — те же каталоги и аренды, что у `MihomoWorker` (`InstanceRoot`, `LeaseRegistry`, порт), `impl CoreAdapter`:

- `capabilities()`: `core: "xray"`, `version: "26.3.27"`, `reload_without_restart: false`, `delay_probe: false`.
- `validate(config)`: файл во временном каталоге instance, `xray run -test -c <file>` с `env_clear()`, тайм-аут 20 с; код ≠ 0 → `InvalidConfig`. Вывод ядра в ошибку не попадает.
- `start(config)`: `CoreConfig` — документ Xray от `generate_config` с портом-заглушкой; адаптер подставляет арендованный порт в `inbounds[0].port` (и отказывает `InvalidConfig`, если входящих не ровно один, он не `mixed` или слушает не `127.0.0.1`), пишет `config.json` 0600, `validate`, запускает `xray run -c <file>` в новой сессии (`setsid`), ждёт до 5 с, пока порт принимает TCP. Готовность: `ApiState::ApiReady` (процесс жив и порт принимает), `RouteState::RouteReady`, `RemoteState::Unknown`.
- `reload(config)`: `Err(CoreError::Unsupported)` — у Xray нет перезагрузки без рестарта ([I05-GAPS.md](I05-GAPS.md)). Отдельный метод `pub fn restart(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError>`: `validate` нового конфига → `stop` → `start`; если новый не прошёл `validate`, работающий процесс не трогается.
- `health()`: процесс жив и порт принимает → прежняя готовность; иначе `DOWN`. `remote` всегда `Unknown`.
- `statistics()`: `Err(CoreError::Unsupported)`.
- `stop()`: как у `MihomoWorker` — SIGTERM группе, через 2 с SIGKILL, освобождение аренд.
- `set_run_as(RunAs)` — как у `MihomoWorker` (I06.I3).

Общий с `MihomoWorker` код (ожидание порта, остановка группы, запись приватного файла) вынести в `src/core/process.rs` и использовать в обоих; копий не оставлять. В `lifecycle.rs` mihomo — только замена тел этих функций на вызовы.

Контракты, которые проверит роль 2:
- **K06 → PACK.Z:** Xray проходит тот же жизненный цикл, что mihomo, а отсутствие reload видно в capabilities и в коде ошибки.
  - capabilities: core "xray", reload_without_restart false, delay_probe false
  - start → ApiReady, порт 127.0.0.1:<аренда> принимает TCP; config.json 0600 и содержит арендованный порт
  - reload → Err(Unsupported), процесс прежний (тот же pid)
  - restart с корректным конфигом → новый pid, порт принимает; restart с конфигом, который `-test` отвергает → Err(InvalidConfig), прежний pid жив
  - statistics → Err(Unsupported); health после kill -9 → DOWN
  - stop → нет процессов группы, аренды освобождены
  - документ с двумя входящими или listen 0.0.0.0 → start → Err(InvalidConfig), аренда не удержана
  - `audit_i04_lifecycle` (mihomo) проходит без изменений после выноса общего кода в src/core/process.rs

### I08.N1 — Вывести все имена и адреса сети туннеля из его номера.

Файлы: `src/net/mod.rs`, `src/net/plan.rs`, `src/lib.rs`. Зависит от: —.

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
`index >= MAX_TUNNELS` → `BadIndex`. Функция чистая: одинаковый вход — одинаковый результат. Диапазоны `10.213.0.0/16` и `198.18.0.0/16` — константы модуля с комментарием: путь данных проверен лабораторией `tools/packet_flow_lab.sh`.

Контракты, которые проверит роль 2:
- **K07 → I08.N2:** Имена и адреса туннеля однозначны, не пересекаются между туннелями и укладываются в пределы ядра.
  - tunnel_net(0, 1000) == {netns cm-0, veth_host cmv0h, veth_ns cmv0n, host_addr 10.213.0.1/30, ns_addr 10.213.0.2/30, gateway 10.213.0.1, tun cmtun0, tun_addr 198.18.0.1/30, dns_addr 198.18.0.2, table 100, rule_priority 1000, mtu 1400, owner_uid 1000}
  - tunnel_net(63, 1000): veth_host cmv63h, host_addr 10.213.63.1/30, tun cmtun63, table 163, rule_priority 1063
  - tunnel_net(64, 1000) → Err(BadIndex)
  - для всех index 0..63: имена интерфейсов ≤ 15 символов; все имена, адреса, таблицы и приоритеты попарно различны
- **K12 → I08.N3:** Отрисовка таблицы использует только имена из TunnelNet.
  - для index 0..63 каждая строка accept содержит ровно veth_host и tun этого туннеля
- **K13 → I08.N5:** TUN-конфиг и resolv.conf берут адреса из TunnelNet, а не из констант.
  - attach_tun для index 5: device cmtun5, inet4-address [198.18.5.1/30]; resolv_conf → nameserver 198.18.5.2
- **K17 → I09.N7:** Сверка ищет ровно те имена, таблицы и приоритеты, которые выдаёт TunnelNet.
  - для index 5: audit на пустом Observed → [MissingBlackhole(5), MissingRule(5), MissingTunRoute(5), MissingVeth(5), MissingTun(5), MissingNetns(5)]
  - Observed, собранный из имён tunnel_net(5, 1000) (правило priority 1005 iif cmv5h table 105; маршруты blackhole и dev cmtun5 в таблице 105; интерфейсы cmv5h, cmtun5; netns cm-5) → []

### I08.N2 — Составить команды создания и удаления сети туннеля.

Файлы: `src/net/commands.rs`. Зависит от: I08.N1.

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

Аргументы — отдельные элементы `args`, без оболочки. Шаг 10 — `Program::Ip` с аргументами `netns exec cm-0 sysctl …`.

Контракты, которые проверит роль 2:
- **K08 → I08.N4:** Порядок команд не оставляет момента, когда трафик veth может уйти в основную таблицу.
  - create_commands(tunnel_net(0,1000), Block) — ровно 17 команд из спецификации, в том же порядке, аргументы по одному
  - при Ipv6Policy::Pass команды 10 нет (16 команд)
  - индекс команды `route add blackhole default metric 200 table 100` меньше индекса `route add default dev cmtun0 table 100`, а тот меньше индекса `rule add iif cmv0h lookup 100 priority 1000`
  - destroy_commands: [rule del iif cmv0h lookup 100 priority 1000; route flush table 100; link del cmtun0; link del cmv0h; netns del cm-0]
  - ни один аргумент не содержит пробела, `;`, `|`, `$`

### I08.N3 — Отрисовать таблицу nftables для всех туннелей.

Файлы: `src/net/firewall.rs`. Зависит от: I08.N1.

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
- `pub fn replace_commands(tunnels: &[TunnelNet]) -> Vec<Cmd>`: одна команда `nft -f -` со stdin `"table inet cm\ndelete table inet cm\n" + render_table(...)` — атомарная замена таблицы одной транзакцией nft.
- Имя цепочки `forward`: `fwd` — зарезервированное слово nft.

Контракты, которые проверит роль 2:
- **K09 → I08.N4:** Таблица запрещает интерфейсам CM любой путь, кроме своего TUN, и не трогает чужой трафик.
  - render_table([0, 3]) == текст из спецификации байт в байт
  - render_table([]) содержит оба правила `"cmv*" counter drop` и ни одного accept
  - туннели переданы в порядке [3, 0] → пары всё равно в порядке 0, 3
  - в тексте `policy accept;` и нет слова `fwd`
  - replace_commands: одна команда Nft с args ["-f", "-"], stdin начинается с "table inet cm\ndelete table inet cm\n"
  - внутри `unshare -rn`: `nft -c -f -` принимает stdin replace_commands([0, 3]); повторная загрузка той же таблицы проходит (замена, а не ошибка «уже существует»)

### I08.N4 — Выполнять команды сети с откатом при отказе.

Файлы: `src/net/exec.rs`. Зависит от: I08.N2, I08.N3.

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

`SystemExec`: программа — только по фиксированному абсолютному пути; код ≠ 0 → `CommandFailed`; вывод команды в ошибку не попадает; `stdin` передаётся через pipe.

Контракты, которые проверит роль 2:
- **K10 → I08.N6:** Отказ любой команды создания сворачивает уже созданное; запреты стоят раньше интерфейсов.
  - create: первая выполненная команда — Nft (таблица), затем 17 команд create_commands
  - fail_at = k для каждого k из 1..=17: результат Err(CommandFailed); после отказа выполнены все команды destroy_commands
  - destroy: все 5 команд выполняются, даже если первая вернула ошибку; последней идёт Nft с таблицей для remaining
  - SystemExec: программа `Ip` запускается как `/usr/bin/ip`; окружение дочернего процесса пустое (проверка подставным `ip` невозможна — путь фиксирован; проверяется по исходнику: в `src/net/exec.rs` нет `Command::new("ip")` без абсолютного пути)

### I08.N5 — Включить TUN и перехват DNS в конфиге worker-а.

Файлы: `src/core/mihomo/config.rs`. Зависит от: I08.N1.

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

`MihomoWorker`: новый метод `pub fn set_tunnel_net(&mut self, t: TunnelNet)`. Если задан, `start` и `reload` используют `attach_tun` вместо `attach_worker_listeners`, порт не арендуется, а готовность маршрута — «интерфейс `t.tun` существует и поднят» вместо ожидания порта.

`pub fn resolv_conf(t: &TunnelNet) -> String` в `src/net/dns.rs` → `"nameserver <t.dns_addr>\noptions edns0\n"`; файл кладётся в `/etc/netns/<t.netns>/resolv.conf` командой из I08.N6.

Контракты, которые проверит роль 2:
- **K11 → I08.N6:** Worker в TUN-режиме слушает только свой TUN, перехватывает DNS и не добавляет маршрутов сам.
  - attach_tun(doc, tunnel_net(0,1000)).tun == {enable true, device cmtun0, stack gvisor, auto-route false, auto-redirect false, auto-detect-interface false, mtu 1400, inet4-address [198.18.0.1/30], dns-hijack [any:53, tcp://any:53]}
  - в результате нет ключа `listeners`; `dns.enable` true, `dns.enhanced-mode` redir-host
  - входной документ с ключом `tun` или `dns` → Err(Forbidden); с `mixed-port` → Err(Forbidden); с правилом GEOIP → Err(InvalidRule)
  - resolv_conf(tunnel_net(0,1000)) == "nameserver 198.18.0.2\noptions edns0\n"
  - с `CM_TEST_MIHOMO`: `mihomo -t` принимает результат attach_tun + attach_instance_controller

### I08.N6 — Создавать и удалять сеть туннеля операциями контроллера.

Файлы: `src/controller/net_ops.rs`, `src/controller/dispatch.rs`, `src/core/leases.rs`. Зависит от: I08.N4, I08.N5, I06.W2.

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

Класс операции и действие polkit — `Net` / `io.github.cm.net` (без изменений).

Контракты, которые проверит роль 2:
- **K14 → PACK.Z:** Сеть туннеля создаётся и удаляется только транзакцией контроллера; после обрыва не остаётся половины сети.
  - net_apply(browser, gen 1) → ok, data {type net, index 0, netns cm-0}; аренда Tunnel "0" у держателя u<uid>-browser; файл <base>/etc-netns/cm-0/resolv.conf == "nameserver 198.18.0.2\noptions edns0\n"
  - второй экземпляр → index 1; net_revert(browser) → аренда 0 свободна; следующий net_apply снова получает index 0
  - Authorizer получает действие "io.github.cm.net"; при отказе → denied, RecordingExec пуст
  - RecordingExec отказывает на 5-й команде create → ответ code failed; аренды нет; resolv.conf удалён; журнал: транзакция Aborted
  - обрыв (`crash_before`) перед шагом net_create → после reconcile аренды нет и resolv.conf удалён
  - net_revert без сети → not_running; net_apply с тем же id повторно → тот же ответ, команды не выполняются второй раз
  - 65-й туннель → code quota либо failed с освобождением всего (аренд Tunnel ровно 64)
  - worker_start для экземпляра с сетью: фабрика получает Some(TunnelNet) с тем же index
- **X04 → I11.A3:** Приложение получает сеть только того экземпляра, который принадлежит вызывающему.
  - uid B: app_launch(browser), сеть browser создана uid A → not_running; процесс не создан
  - дескриптор netns открывается по номеру из аренды владельца, а не по имени из кадра

### I09.N7 — Сверять желаемое состояние сети с фактическим.

Файлы: `src/net/observe.rs`. Зависит от: I08.N1.

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
- `audit`: для каждого желаемого туннеля проверяются blackhole (dst `default`, kind `blackhole`, таблица `t.table`), маршрут в TUN, правило (`priority`, `iif`), интерфейсы, netns. Интерфейс `cmv<N>h` или правило с приоритетом 1000..1063 без желаемого туннеля → `OrphanVeth` / `OrphanRule`.
- `repair_commands`: `MissingBlackhole` → команда 14 из I08.N2; `MissingRule` → команда 16; `OrphanVeth(name)` → `ip link del <name>`; `OrphanRule(p)` → `ip rule del priority <p>`. Для `MissingVeth`, `MissingTun`, `MissingNetns` — пустой список: сеть пересоздаётся целиком через `NetRevert` и `NetApply`, а не чинится по частям.
- Порядок результата `audit` — по index, внутри — в порядке перечисления вариантов.

Контракты, которые проверит роль 2:
- **K15 → PACK.Z:** Расхождение желаемого и фактического состояния сети обнаруживается и называется; чинится только безопасное.
  - фикстуры после create для index 0: audit → []
  - из фикстуры маршрутов убран blackhole → [MissingBlackhole(0)]; repair → [ip route add blackhole default metric 200 table 100]
  - убрано правило iif → [MissingRule(0)]; repair → [ip rule add iif cmv0h lookup 100 priority 1000]
  - нет cmtun0 в links → [MissingTunRoute(0), MissingTun(0)] в этом порядке; repair для MissingTun → []
  - лишний интерфейс cmv7h без желаемого туннеля → [OrphanVeth("cmv7h")]; repair → [ip link del cmv7h]
  - правило priority 1042 без туннеля → [OrphanRule(1042)]; правило priority 32766 (main) не считается сиротой
  - parse_rules("not json") → Err(Drift)

### I09.N8 — Вести состояние туннеля конечным автоматом.

Файлы: `src/net/state.rs`. Зависит от: —.

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
- Остановка туннеля с живыми сессиями приложений (Q12): вызывающий оставляет сеть (`NetRevert` не вызывается) — приложение остаётся в `Blocked`; это правило записать в doc-комментарии `next`.

Контракты, которые проверит роль 2:
- **K16 → PACK.Z:** Автомат туннеля не имеет перехода, в котором трафик идёт при неготовом ядре.
  - Stopped + StartRequested → Starting; Starting + CoreReady → Up; Up + RemoteLost → Degraded; Degraded + RemoteBack → Up
  - Up + CoreDown → Blocked; Degraded + CoreDown → Blocked; Up + DriftFound → Blocked; Starting + DriftFound → Blocked
  - Blocked + CoreReady → Up; Blocked + Repaired → Starting; Failed + StartRequested → Starting
  - любое состояние + StopRequested → Stopped
  - все пары вне таблицы оставляют состояние прежним (например Stopped + CoreReady → Stopped, Starting + NetReady → Starting)
  - passes_traffic истинно только для Up и Degraded
  - net_axis: Up → Verified, Degraded → Partial, Blocked → Blocked, Failed → Error, Stopped → Unknown, Starting → Unknown
  - ни из одного состояния одним событием нельзя попасть в Up, кроме CoreReady и RemoteBack

### I11.A1 — Проверить описание приложения перед запуском.

Файлы: `src/app/mod.rs`, `src/app/spec.rs`, `src/lib.rs`. Зависит от: —.

`pub mod app;` в `src/lib.rs`. Вход — `profiles::model::ApplicationDefinition` (тип не менять).
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec { pub program: PathBuf, pub args: Vec<String>, pub cwd: Option<PathBuf>, pub env: BTreeMap<String, String> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecError { NotAbsolute, HasNul, DotDot, TooManyArgs, ArgTooLong, BadEnvName, ForbiddenEnv(String), DuplicateEnv, BadCwd }
pub fn check(definition: &ApplicationDefinition) -> Result<LaunchSpec, SpecError>
```
Правила, в этом порядке:
1. `executable` — абсолютный путь (`NotAbsolute`), без NUL (`HasNul`), без сегмента `..` (`DotDot`); поиск по `PATH` не выполняется никогда;
2. `argv`: не больше 256 аргументов (`TooManyArgs`), каждый ≤ 8192 байт (`ArgTooLong`), без NUL (`HasNul`);
3. `cwd`, если задан: абсолютный, без NUL и `..` (`BadCwd`);
4. `environment`: имя по `^[A-Za-z_][A-Za-z0-9_]*$` (`BadEnvName`); значение без NUL (`HasNul`); повтор имени → `DuplicateEnv`;
5. запрещённые имена → `ForbiddenEnv(имя)`:
   - всё, что начинается с `LD_`;
   - `TZ`, `LANG`, `LANGUAGE`, всё с префиксом `LC_` — их задаёт окружение региона;
   - `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`;
   - `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR` — их выдаёт запуск, а не описание.

`SpecError::Display` — фиксированные фразы через `t!`; в `ForbiddenEnv` — только имя переменной, без значения.

Контракты, которые проверит роль 2:
- **M01 → I11.A2:** Описание приложения не может подменить программу, окружение региона или библиотеки.
  - executable "/usr/bin/firefox", argv ["--new-window"], env [{MOZ_ENABLE_WAYLAND, 1}] → Ok(LaunchSpec{program /usr/bin/firefox, args [--new-window], env {MOZ_ENABLE_WAYLAND: 1}})
  - executable "firefox" → NotAbsolute; "/usr/../bin/sh" → DotDot; с NUL → HasNul
  - 257 аргументов → TooManyArgs; аргумент 8193 байта → ArgTooLong
  - cwd "relative" и "/a/../b" → BadCwd
  - env имя "1A", "A-B", "" → BadEnvName; два раза A → DuplicateEnv
  - env LD_PRELOAD, LD_LIBRARY_PATH, TZ, LANG, LANGUAGE, LC_ALL, LC_TIME, PATH, HOME, DBUS_SESSION_BUS_ADDRESS → ForbiddenEnv с этим именем
  - ForbiddenEnv("TZ") в Display не содержит значения переменной
- **M04 → I11.A4:** Ярлык не может выполнить ничего, кроме `cm app run <id>`.
  - desktop_entry("browser", "Браузер", None) == "[Desktop Entry]\nType=Application\nName=Браузер\nExec=cm app run browser\nTerminal=false\nX-CM-Application=browser\n"
  - с icon "firefox" — строка `Icon=firefox` между Exec и Terminal
  - exec_quote("plain") == "plain"; exec_quote("a b") == "\"a b\""; exec_quote("a$b") == "\"a\\$b\""; exec_quote("50%") == "50%%"; exec_quote("a\"b") == "\"a\\\"b\""
  - name с переводом строки → Err; name "a\\b" → строка `Name=a\\\\b`
  - id "../x" или "a b" → Err(BadId)

### I11.A2 — Собрать план запуска: сеть, пользователь, окружение.

Файлы: `src/app/plan.rs`, `src/identity/plan.rs`. Зависит от: I11.A1, I06.I3.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppLaunchPlan { pub netns: String, pub program: PathBuf, pub args: Vec<String>, pub cwd: Option<PathBuf>,
                           pub env: BTreeMap<String, String>, pub run_as: RunAs }
pub fn plan(spec: &LaunchSpec, netns: &str, run_as: RunAs, preset: &EnvironmentPreset,
            session_env: &BTreeMap<String, String>) -> AppLaunchPlan
```
`env` собирается в таком порядке (следующий слой перекрывает предыдущий):
1. из `session_env` — только allowlist `identity::plan::PARENT_ENV_KEYS` (сделать его `pub`; `PATH` по умолчанию `/usr/bin:/bin`);
2. переменные `spec.env`;
3. окружение региона: `TZ = preset.timezone`, `LANG = preset.locale`.

`preset` — `profiles::model::EnvironmentPreset`. Ничего другого от родителя в `env` не попадает.

В `src/identity/plan.rs` — только `pub` у `PARENT_ENV_KEYS` и у функции сборки allowlist, чтобы список был один.

Контракты, которые проверит роль 2:
- **M02 → I11.A3:** Окружение процесса состоит только из allowlist, переменных описания и региона; регион перекрывает всё.
  - session_env {DISPLAY=:0, HOME=/h, SECRET_TOKEN=x, TZ=Europe/Moscow, LC_TIME=ru_RU.UTF-8}, spec.env {A=1}, preset {timezone Europe/Berlin, locale de_DE.UTF-8} → env == {A=1, DISPLAY=:0, HOME=/h, LANG=de_DE.UTF-8, PATH=/usr/bin:/bin, TZ=Europe/Berlin}
  - в env нет SECRET_TOKEN, LC_TIME
  - plan.netns == переданному; plan.run_as == переданному
  - identity::plan::PARENT_ENV_KEYS — один список на оба плана (в `src/app/plan.rs` нет своей копии имён DISPLAY, WAYLAND_DISPLAY)

### I11.A3 — Запускать приложение в его netns операцией контроллера.

Файлы: `src/controller/app_ops.rs`, `src/controller/dispatch.rs`, `src/controller/drop.rs`. Зависит от: I11.A2, I06.W2, I08.N6.

Заполняет `AppLaunch`, который в I06 отвечает `unsupported`.

`AppLaunch { instance, generation, program, args }`:
1. у экземпляра есть сеть (I08.N6) и её поколение совпадает → иначе `NotRunning` / `GenerationMismatch`;
2. состояние туннеля допускает запуск: worker запущен (`Workers::status` → running) → иначе `NotRunning`. Запуск в сеть без работающего ядра запрещён: приложение стартовало бы в `Blocked`;
3. `LaunchSpec` из `program` и `args` кадра (правила I11.A1 для пути и аргументов; env из кадра не принимается);
4. `run_as_for(peer.uid, peer.gid)`;
5. запуск: дочерний процесс входит в netns и только потом сбрасывает привилегии.

В `src/controller/drop.rs` — `pub fn drop_into_netns_pre_exec(command, netns_fd: RawFd, run_as, keep_fds, limits)`: первым шагом `setns(netns_fd, CLONE_NEWNET)`, затем шаги `drop_pre_exec`. Дескриптор netns открывается в родителе: `/run/netns/cm-<index>` (`O_RDONLY | O_CLOEXEC`); в `test_mode` корень — `<base>/netns`.

Окружение: allowlist из окружения **клиента** недоступен контроллеру, поэтому `AppLaunch` получает env так: `PATH=/usr/bin:/bin`, `HOME` — домашний каталог uid из `getpwuid_r`, `XDG_RUNTIME_DIR=/run/user/<uid>`, плюс `TZ` и `LANG` из пресета экземпляра, если он записан (файл `<root>/instances/<instance>/env.json` владельца: `{"timezone": …, "locale": …}`, проверки владельца и прав — как у `read_owned_config`).

Ответ — новый вариант `ReplyData::Launched { pid: u32 }`. Контроллер не ждёт завершения приложения, но забирает статус выхода в фоне (нет зомби).

Не входит: cgroup-область на сессию и запись `Session` в Store — следующая итерация (нужен `systemd-run` или делегированный cgroup).

Контракты, которые проверит роль 2:
- **M03 → I11.A5:** Приложение запускается внутри сети своего туннеля, под uid вызывающего, и только при работающем ядре.
  - app_launch без сети → not_running; с сетью, но без worker → not_running
  - после net_apply и worker_start: app_launch → ok, data {type launched, pid N}
  - файл приложения: интерфейсы только lo и cmv0n с адресом 10.213.0.2/30 (нет интерфейсов хоста)
  - env приложения: PATH, HOME, XDG_RUNTIME_DIR и TZ/LANG из env.json; нет переменных контроллера
  - приложение с NUL или относительным program в кадре → bad_argument (отсекает декодер)
  - после завершения приложения у контроллера нет зомби-потомков
  - с подчинённым uid (`--map-users`): `id -u` приложения == uid клиента, CapEff 0
- **Z02 → PACK.Z:** Сквозной сценарий: приложение в своём netns выходит наружу только через TUN своего worker-а; при гибели worker-а трафик блокируется, а не идёт напрямую.
  - net_apply → worker_start (mode direct) → app_launch `curl -q -s --noproxy '*' -m 5 http://198.51.100.2:8080/` → HTTP 200
  - счётчик `iifname "cmv0h" oifname "cmtun0"` > 0; счётчики `"cmv*" counter drop` == 0; `/connections` worker-а показывает downloadTotal > 0
  - kill -9 процесса ядра → тот же запрос: тайм-аут (curl rc 28), HTTP-кода нет; на интерфейсе «интернета» нет пакетов с адреса 10.213.0.2
  - удаление cmtun0 (`ip link del`) → запрос по-прежнему не проходит: в таблице 100 остаётся `blackhole default metric 200`
  - контроль: без правила iif и без таблицы cm (и с NAT наружу) тот же запрос даёт 200 — блокируют именно правила CM
  - DNS: `getent hosts example.test` внутри netns уходит на 198.18.0.2 (пакеты на cmtun0, порт 53), на «интернет»-интерфейсе нет DNS-пакетов с адреса 10.213.0.2
  - net_revert → нет cmv0h, cmtun0, netns cm-0, правила priority 1000 и таблицы 100; таблица cm содержит только два запрета
  - curl вызывается с `-q`: `~/.curlrc` пользователя может задавать прокси

### I11.A4 — Сгенерировать ярлык приложения с верным экранированием.

Файлы: `src/app/desktop.rs`. Зависит от: I11.A1.

`pub fn desktop_entry(id: &str, name: &str, icon: Option<&str>) -> Result<String, SpecError>` и `pub fn exec_quote(arg: &str) -> String`.

Текст (ровно так, `\n` в конце каждой строки):
```
[Desktop Entry]
Type=Application
Name=<name>
Exec=cm app run <exec_quote(id)>
Icon=<icon>            ← строка есть, только если icon задан
Terminal=false
X-CM-Application=<id>
```
- `id` — по правилам `profiles::model::Id`; иначе `BadEnvName` не использовать — вернуть `SpecError::NotAbsolute` нельзя: добавить вариант `SpecError::BadId`.
- `name`: переводы строк и управляющие символы запрещены (`HasNul`); `\` → `\\` (правило значений Desktop Entry).
- `exec_quote` по спецификации Desktop Entry: аргумент заключается в двойные кавычки, если содержит пробел, табуляцию, перевод строки или любой из символов `"'\><~|&;$*?#()` и обратную кавычку; внутри кавычек экранируются обратной чертой `"`, обратная кавычка, `$` и `\`; затем каждый `%` удваивается (`%%`).
- Ярлык запускает `cm app run`, а не программу напрямую: иначе приложение стартовало бы в сети хоста.

Контракты, которые проверит роль 2:
- **M05 → I11.A5:** Команда печатает ровно текст ярлыка.
  - `cm app desktop browser Браузер` → stdout == строка из M04, код 0
  - `cm app desktop ../x N` → код 2

### I11.A5 — Дать команду `cm app`.

Файлы: `src/app/cli.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: I11.A3, I11.A4, I06.S1.

`src/main.rs`: `if args.first() == "app" { exit(cm::app::cli::dispatch(&args[1..])) }` рядом с `identity`.

- `cm app check FILE` — читает `ApplicationDefinition` из JSON-файла, печатает итог `spec::check` (программа, число аргументов, имена переменных без значений). Код 0 или 2.
- `cm app desktop ID NAME [--icon ICON]` — печатает ярлык в stdout. Код 0 или 2.
- `cm app run INSTANCE -- PROGRAM [ARGS…]` — отправляет контроллеру `AppLaunch` (сокет `CM_CONTROLLER_SOCKET`, по умолчанию `/run/cm/controller.sock`), `generation` берёт из ответа `WorkerStatus` этого экземпляра; печатает `pid`. Коды: 0; 2 — использование или `SpecError`; 3 — отказ контроллера (в выводе — его код в квадратных скобках); 4 — контроллер недоступен.
- `cm app` без аргументов — справка, код 2.

Клиент контроллера: `pub fn call(socket: &Path, op: Op) -> Result<Reply, ClientError>` в `src/controller/client.rs` — один кадр, один ответ, тайм-аут 30 с; `id` — 16 случайных hex-символов из `/dev/urandom`.

Строки — через `t!`, переводы на 6 языков в конец `src/i18n_table.rs`.

Контракты, которые проверит роль 2:
- **M06 → PACK.Z:** `cm app` различает ошибки использования, отказы контроллера и его недоступность.
  - `cm app` → справка, код 2; справка на en, de, it, zh, ar без кириллицы
  - `cm app check FILE` с корректным описанием → код 0, в выводе нет значений переменных; с LD_PRELOAD → код 2 и [ForbiddenEnv]
  - `cm app run browser -- /usr/bin/true`: клиент шлёт worker_status, затем app_launch с generation из ответа; ответ launched pid 42 → stdout содержит 42, код 0
  - сервер отвечает code not_running → код 3, вывод содержит [not_running]
  - сокета нет → код 4
  - `cm app run browser -- true` (не абсолютный путь) → код 2 без обращения к сокету

### I12.A6 — Считать держателей общего туннеля и решать, когда его останавливать.

Файлы: `src/app/shared.rs`. Зависит от: —.

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TunnelRefs { /* tunnel id → множество session id */ }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefAction { None, StartTunnel(Id), StopTunnel(Id) }
impl TunnelRefs {
    pub fn acquire(&mut self, tunnel: &Id, session: &Id) -> RefAction;
    pub fn release(&mut self, tunnel: &Id, session: &Id, owner: &TunnelOwner) -> RefAction;
    pub fn holders(&self, tunnel: &Id) -> usize;
}
pub fn rebuild(sessions: &[Session]) -> TunnelRefs   // только SessionLifecycle::Active
```
- `acquire`: первый держатель → `StartTunnel`; повтор той же сессии — без изменений и `None`.
- `release`: держателей стало 0 → `StopTunnel`, **кроме** `TunnelOwner::Host`: туннель хоста живёт своим режимом, завершение последней сессии приложения его не останавливает (Q12). Неизвестная сессия → `None`.
- Счётчик не уходит ниже нуля и не зависит от порядка операций разных сессий.

Контракты, которые проверит роль 2:
- **M07 → I12.A7:** Туннель группы живёт, пока есть хотя бы одна активная сессия; туннель хоста сессиями не управляется.
  - acquire(t, s1) → StartTunnel(t); acquire(t, s2) → None; holders 2
  - acquire(t, s1) повторно → None; holders 2
  - release(t, s1, Group) → None; release(t, s2, Group) → StopTunnel(t); holders 0
  - release(t, s9, Group) для неизвестной сессии → None; holders не уходит ниже 0
  - release последней сессии с owner Host → None
  - все 24 перестановки операций [acquire s1, acquire s2, release s1, release s2] при корректном порядке внутри сессии дают ровно один StartTunnel и один StopTunnel
  - rebuild: из 3 сессий (2 Active на t, 1 Ended на t) → holders(t) == 2

### I12.A7 — Решать, что делать с живой сессией при смене назначения.

Файлы: `src/app/reassign.rs`. Зависит от: I12.A6.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reassign { NoChange, AppliesToNextLaunch, RestartRequired { reason: ReassignReason } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReassignReason { TunnelChanged, GenerationChanged, NodeGone }
pub fn decide(session: Option<&Session>, current: &ApplicationAssignment, wanted: &ApplicationAssignment,
              tunnel_generation: u64, node_present: bool) -> Reassign
```
- нет активной сессии: назначение то же → `NoChange`, иначе `AppliesToNextLaunch`;
- активная сессия, а назначение указывает на другой туннель или группу → `RestartRequired { TunnelChanged }`;
- назначение то же, но `tunnel_generation != session.tunnel_generation` → `RestartRequired { GenerationChanged }`;
- `node_present == false` → `RestartRequired { NodeGone }` (проверяется первым);
- иначе `NoChange`.

Живое соединение никогда не переадресуется молча (Q13): функция не имеет варианта «переключить на лету».

Контракты, которые проверит роль 2:
- **M08 → PACK.Z:** Смена назначения никогда не переадресует живую сессию молча.
  - нет сессии, то же назначение → NoChange; другое → AppliesToNextLaunch
  - сессия на OwnTunnel{t1}, wanted OwnTunnel{t2} → RestartRequired{TunnelChanged}
  - сессия на OwnTunnel{t1}, wanted Group{g} → RestartRequired{TunnelChanged}
  - то же назначение, session.tunnel_generation 3, текущее 4 → RestartRequired{GenerationChanged}
  - node_present false при любом назначении → RestartRequired{NodeGone}
  - то же назначение, то же поколение, узел есть → NoChange

### I14.R1 — Определять страну выхода по нескольким наблюдениям.

Файлы: `src/env/mod.rs`, `src/env/geo.rs`, `src/lib.rs`. Зависит от: —.

`pub mod env;` в `src/lib.rs`.
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation { pub source: String, pub country: String /* ISO 3166-1 alpha-2 */, pub at_unix_ms: i64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionDecision { Agreed { country: String, sources: usize }, Disagree { countries: Vec<String> }, Insufficient { fresh: usize }, Stale }
pub const MIN_SOURCES: usize = 2;
pub const MAX_AGE_MS: i64 = 15 * 60 * 1000;
pub fn decide(observations: &[Observation], now_unix_ms: i64) -> RegionDecision
pub fn region_axis(decision: &RegionDecision, preset_country: &str) -> VerificationValue
```
`decide`:
1. отбрасываются наблюдения с некорректным кодом страны (не две заглавные латинские буквы) и из будущего (`at > now`);
2. от каждого `source` берётся только самое свежее;
3. свежие — не старше `MAX_AGE_MS`. Свежих нет, а устаревшие есть → `Stale`; наблюдений нет вовсе → `Insufficient { fresh: 0 }`;
4. свежих источников меньше `MIN_SOURCES` → `Insufficient { fresh }`;
5. все свежие назвали одну страну → `Agreed`; иначе `Disagree` со списком стран по алфавиту без повторов.

Имя узла («🇩🇪 Germany-1») наблюдением не является и в функцию не передаётся.

`region_axis`: `Agreed` и страна == `preset_country` → `Verified`; `Agreed` с другой страной → `Blocked`; `Disagree` → `Partial`; `Stale` и `Insufficient` → `Unknown`.

Контракты, которые проверит роль 2:
- **M09 → I14.R2:** Страна выхода подтверждается только согласием нескольких свежих источников.
  - два источника DE, оба 1 минуту назад → Agreed{DE, 2}
  - DE и NL → Disagree{[DE, NL]}; три источника DE, DE, NL → Disagree{[DE, NL]}
  - один свежий источник → Insufficient{1}; пусто → Insufficient{0}
  - два источника 16 минут назад → Stale; один свежий и один старый → Insufficient{1}
  - один источник дал DE (10 мин назад) и NL (1 мин назад), второй — NL → Agreed{NL, 2} (берётся свежее от источника)
  - страна "Germany", "de", "" и наблюдение из будущего отбрасываются
  - region_axis: Agreed{DE} при пресете DE → Verified; при пресете NL → Blocked; Disagree → Partial; Stale и Insufficient → Unknown
- **M11 → I14.R3:** Смена страны выхода меняет только ось REGION; приложение не перезапускается и не переключается.
  - preset DE, current Agreed{DE} → Keep → Verified
  - current Agreed{NL} → RegionMismatch{expected DE, observed NL} → Blocked
  - current Disagree, Insufficient → RegionUnknown → Unknown
  - previous Agreed{DE}, current Stale → RegionUnknown (подтверждение не продлевается)

### I14.R2 — Проверить, что окружение региона применимо на этой системе.

Файлы: `src/env/apply.rs`. Зависит от: I14.R1.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvIssue { ZoneMissing, LocaleNotInstalled, LanguagesEmpty, LocaleLanguageMismatch }
pub fn check_preset(preset: &EnvironmentPreset, tzdir: &Path, installed_locales: &[String]) -> Vec<EnvIssue>
pub fn parse_locale_list(text: &str) -> Vec<String>     // вывод `locale -a`
pub fn normalize_locale(name: &str) -> String           // "de_DE.UTF-8" и "de_DE.utf8" → "de_DE.utf8"
```
- `ZoneMissing`: файла `tzdir/<timezone>` нет либо имя не проходит правила `identity::tzdata` (использовать ту же проверку имени — сделать её `pub`).
- `LocaleNotInstalled`: `normalize_locale(preset.locale)` нет среди `normalize_locale` установленных. Это предупреждение, не отказ: Chromium берёт локаль из своей ICU (лаборатория, п. 10), а программам на glibc нужна установленная.
- `LanguagesEmpty`: `preset.languages` пуст.
- `LocaleLanguageMismatch`: язык `preset.locale` (до `_`) не совпадает с языком `preset.languages[0]` (до `-`).

Порядок результата — порядок перечисления вариантов. Система не меняется: функция только читает.

Контракты, которые проверит роль 2:
- **M10 → PACK.Z:** Проверка окружения называет каждую проблему и ничего не меняет в системе.
  - preset {Europe/Berlin, de_DE.UTF-8, [de-DE, de]}, зона есть, locales [de_DE.utf8, en_US.utf8] → []
  - зоны нет → [ZoneMissing]; timezone "../etc/passwd" → [ZoneMissing]
  - locales [en_US.utf8] → [LocaleNotInstalled]
  - languages [] → [LanguagesEmpty]; locale de_DE.UTF-8 при languages [fr-FR] → [LocaleLanguageMismatch]
  - parse_locale_list("C\nC.utf8\nde_DE.utf8\n") == [C, C.utf8, de_DE.utf8]; normalize_locale("de_DE.UTF-8") == normalize_locale("de_DE.utf8")

### I14.R3 — Решать, что делать с сессией, когда страна выхода сменилась.

Файлы: `src/env/invalidate.rs`. Зависит от: I14.R1.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalidation { Keep, RegionUnknown, RegionMismatch { expected: String, observed: String } }
pub fn on_region_change(preset_country: &str, previous: &RegionDecision, current: &RegionDecision) -> Invalidation
pub fn axis_after(invalidation: &Invalidation) -> VerificationValue
```
- `current` — `Agreed` со страной пресета → `Keep`;
- `Agreed` с другой страной → `RegionMismatch { expected: preset_country, observed }`;
- `Disagree`, `Stale`, `Insufficient` → `RegionUnknown`, **кроме** случая, когда `previous` был `Agreed` со страной пресета, а `current` — `Stale`: тогда тоже `RegionUnknown` (устаревшее подтверждение не продлевается).

`axis_after`: `Keep` → `Verified`; `RegionUnknown` → `Unknown`; `RegionMismatch` → `Blocked`.

Приложение при этом не перезапускается и не переключается: вызывающий только меняет ось REGION и показывает причину (I14.T05).

Контракты, которые проверит роль 2:
- **M12 → PACK.Z:** Функции региона чистые: не читают сеть, файлы и часы.
  - в `src/env/geo.rs` и `src/env/invalidate.rs` нет `std::fs`, `std::net`, `SystemTime`, `Command`

### I16.Q1 — Отбирать узлы ограничениями и ранжировать оценкой с объяснением.

Файлы: `src/quality/mod.rs`, `src/quality/score.rs`, `src/lib.rs`. Зависит от: —.

`pub mod quality;` в `src/lib.rs`.
```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate { pub node: String, pub country: Option<String>, pub protocol: String,
                       pub delay_ms: Option<u32>, pub loss_percent: u8, pub last_ok_unix_ms: Option<i64> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraints { pub countries: Option<Vec<String>>, pub protocols: Option<Vec<String>>, pub max_delay_ms: Option<u32> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection { NoMeasurement, CountryNotAllowed, CountryUnknown, ProtocolNotAllowed, TooSlow, TooLossy }
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked { pub node: String, pub score: u32 }
pub struct Outcome { pub ranked: Vec<Ranked>, pub rejected: Vec<(String, Rejection)> }
pub fn evaluate(candidates: &[Candidate], constraints: &Constraints) -> Outcome
pub fn score(delay_ms: u32, loss_percent: u8) -> u32
```
Отбор — первое сработавшее правило:
1. `delay_ms == None` → `NoMeasurement`;
2. задан `countries`: `country == None` → `CountryUnknown`; не из списка → `CountryNotAllowed`;
3. задан `protocols` и протокола нет в списке → `ProtocolNotAllowed`;
4. задан `max_delay_ms` и `delay_ms` больше → `TooSlow`;
5. `loss_percent > 20` → `TooLossy`.

`score` (меньше — лучше): `delay_ms + loss_percent as u32 * 50`. Потери весят больше задержки: 1 % потерь равен 50 мс.

`ranked` — по возрастанию `score`, при равенстве — по имени узла (детерминированно). `rejected` — в порядке входа. Ограничение никогда не ослабляется ради того, чтобы «хоть что-то выбрать»: пустой `ranked` — допустимый ответ.

Контракты, которые проверит роль 2:
- **M13 → I16.Q2:** Ограничения применяются раньше оценки и никогда не ослабляются; ранжирование детерминировано.
  - score(100, 0) == 100; score(100, 2) == 200; score(80, 1) == 130
  - кандидаты a{120 мс, 0 %}, b{80 мс, 1 %}, c{нет замера} без ограничений → ranked [a(120), b(130)], rejected [(c, NoMeasurement)]
  - countries [DE]: узел NL → CountryNotAllowed; узел без страны → CountryUnknown
  - protocols [vless]: узел ss → ProtocolNotAllowed; max_delay_ms 100: узел 120 мс → TooSlow; loss 21 % → TooLossy, loss 20 % проходит
  - все узлы отвергнуты → ranked пуст (ограничение не снимается)
  - равные score: порядок по имени узла; перестановка входа не меняет ranked

### I16.Q2 — Переключать узел только при заметном и устойчивом выигрыше.

Файлы: `src/quality/switch.rs`. Зависит от: I16.Q1.

```rust
pub const MIN_GAIN_PERCENT: u32 = 20;
pub const MIN_DWELL_MS: i64 = 60_000;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Switch { Stay { reason: StayReason }, To { node: String } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StayReason { AlreadyBest, GainTooSmall, DwellNotElapsed, NoCandidates }
pub fn decide(current: Option<&str>, ranked: &[Ranked], last_switch_unix_ms: i64, now_unix_ms: i64) -> Switch
```
- `ranked` пуст → `Stay { NoCandidates }`: текущий узел не меняется на произвольный;
- `current == None` или текущего нет в `ranked` (он отвергнут ограничениями) → `To { ranked[0] }` немедленно, без ожидания;
- текущий — `ranked[0]` → `Stay { AlreadyBest }`;
- `now - last_switch < MIN_DWELL_MS` → `Stay { DwellNotElapsed }`;
- выигрыш `(cur.score - best.score) * 100 / cur.score < MIN_GAIN_PERCENT` → `Stay { GainTooSmall }`;
- иначе `To { ranked[0] }`.

Переключение узла у живой сессии приложения — отдельное решение I12.A7 (`RestartRequired`); эта функция отвечает только на вопрос «какой узел лучше».

Контракты, которые проверит роль 2:
- **M14 → PACK.Z:** Узел не меняется из-за шума измерений и не меняется на произвольный, когда кандидатов нет.
  - ranked [] → Stay{NoCandidates}
  - current None → To{ranked[0]}; current отсутствует в ranked → To{ranked[0]} даже при last_switch 1 с назад
  - current == ranked[0] → Stay{AlreadyBest}
  - current score 100, лучший 85 (выигрыш 15 %) → Stay{GainTooSmall}; лучший 80 (20 %) → To
  - выигрыш 50 %, но с прошлого переключения 59 с → Stay{DwellNotElapsed}; 60 с → To

### I17.U1 — Строить представление туннелей из настоящих сессий и проверок.

Файлы: `src/status/tunnels.rs`, `src/status.rs`, `src/tui/mock_tunnels.rs`. Зависит от: —.

`src/status/tunnels.rs` (модуль библиотеки; подключить в `src/status.rs` одной строкой `pub mod tunnels;` — если `status.rs` не каталог-модуль, создать `src/status/` рядом, не перенося существующий код):
```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelView { pub name: String, pub host: String, pub apps: String, pub condition: String,
                        pub axes: BTreeMap<String, String>, pub age_s: BTreeMap<String, Option<u64>>,
                        pub failure: Option<String>, pub sessions: u32 }
pub fn build(host: &HostPolicy, tunnel: &TunnelInstance, sessions: &[Session], verifications: &[Verification],
             group: Option<&ApplicationGroup>, now_unix_ms: i64) -> TunnelView
```
Формат совпадает с фикстурами `tests/fixtures/i17/states.json`: страница `src/tui/mock_tunnels.rs` принимает `TunnelView` без изменений отрисовки. В `mock_tunnels.rs` — только замена собственной структуры `Scenario` на `pub use cm::status::tunnels::TunnelView as Scenario;` и чтение фикстур через неё.

Правила `build`:
- `name` — `tunnel.id`; `host` — `off`/`proxy`/`tunnel` по `host.mode`; `apps` — `shared`, если `tunnel.owner` — `Group`, иначе `separate`;
- `sessions` — число активных сессий этого туннеля (`tunnel_instance_id` совпал, `lifecycle == Active`);
- по каждой оси берётся самая свежая `Verification` **текущего поколения** туннеля; проверки прежних поколений не учитываются. Нет проверки → значение `unknown`, возраст `None`;
- значение оси — snake_case варианта `VerificationValue`; `age_s = (now - evidence_at) / 1000`, отрицательное → 0;
- `condition`: любая ось `blocked` → `blocked`; иначе любая `error` или `partial` → `degraded`; иначе `unknown`. Значения `ok` нет: страница не сводит оси в один зелёный статус;
- `failure`: `net` = `error` → `api-down`; `region` = `blocked` → `region-mismatch`; иначе `None` (первое сработавшее).

Контракты, которые проверит роль 2:
- **M15 → I17.U2:** Представление туннеля строится из проверок текущего поколения и не сводит оси в один «зелёный» статус.
  - host Proxy, туннель owner Application, поколение 4, одна активная сессия, проверки поколения 4: net Verified 40 с назад, region Partial 300 с назад → {host proxy, apps separate, sessions 1, axes {net verified, region partial, state unknown, app unknown}, age_s {net 40, region 300, state None, app None}, condition degraded, failure None}
  - проверка net поколения 3 (Verified) при туннеле поколения 4 не учитывается → net unknown
  - две проверки одной оси: берётся с большим evidence_at
  - любая ось blocked → condition blocked; все verified → condition unknown (значения ok нет)
  - net error → failure api-down; region blocked → failure region-mismatch; оба сразу → api-down
  - owner Group → apps shared; сессии Ended не считаются
  - evidence_at в будущем → age 0
  - 24 снимка `tests/fixtures/i17/snapshots/*.txt` проходят без перезаписи после замены Scenario на TunnelView

### I17.U2 — Выбирать значок апплета по состояниям туннелей.

Файлы: `cosmic/src/tunnel_icons.rs`. Зависит от: I17.U1.

`pub fn badge_for(views: &[TunnelView]) -> Option<TunnelBadge>` в существующем `cosmic/src/tunnel_icons.rs` (тип `TunnelBadge` уже есть; `TunnelView` — из `cm::status::tunnels`).

Приоритет, первое сработавшее:
1. у любого туннеля `condition == "blocked"` → `Blocked`;
2. у любого туннеля `condition == "degraded"` → `Partial`;
3. у любого туннеля `host == "off"` и `sessions > 0` → `HostOffAppActive`;
4. иначе `None` — апплет показывает обычный значок обновлений.

Подключение значка в панель (`cosmic/src/panel.rs`) в эту вершину не входит: источник `TunnelView` в апплете появится вместе с контроллером на установке.

Контракты, которые проверит роль 2:
- **M16 → PACK.Z:** Значок апплета отражает худшее состояние; при отсутствии туннелей апплет не меняется.
  - [] → None
  - [{condition blocked}, {condition degraded}] → Blocked
  - [{condition degraded}] → Partial
  - [{host off, sessions 2, condition unknown}] → HostOffAppActive
  - [{host tunnel, sessions 2, condition unknown}] → None

## Сдача

Отчёт:
- по каждой вершине: сделано или открыто с причиной;
- `git diff --stat` по существующим файлам;
- EXIT и счётчики gate;
- список мест, где спецификация допускала толкование, и выбранное толкование.

## Что закрывает пакет в общем плане

| Вершина | Закрывает подзадачи |
|---|---|
| `I05.D1` | I05.T04.a |
| `I05.D2` | I05.T04.a |
| `I05.D3` | I05.T04.a |
| `I05.D4` | I05.T04.b |
| `I05.X1` | I05.T03.a |
| `I05.X2` | I05.T03.b |
| `I08.N1` | I08.T01.b |
| `I08.N2` | I08.T02.a, I09.T02.a |
| `I08.N3` | I08.T03.a, I09.T02.a |
| `I08.N4` | I08.T02.a |
| `I08.N5` | I10.T01.a, I10.T02.a |
| `I08.N6` | I08.T02.a, I08.T04.a |
| `I09.N7` | I09.T03.a |
| `I09.N8` | I09.T01.a, I09.T04.a |
| `I11.A1` | I11.T01.a |
| `I11.A2` | I11.T01.a, I14.T03.a |
| `I11.A3` | I11.T02.a |
| `I11.A4` | I11.T03.a |
| `I11.A5` | I11.T03.a |
| `I12.A6` | I12.T02.a |
| `I12.A7` | I12.T04.a |
| `I14.R1` | I14.T01.a |
| `I14.R2` | I14.T03.a, I14.T04.a |
| `I14.R3` | I14.T05.a |
| `I16.Q1` | I16.T01.a, I16.T02.a |
| `I16.Q2` | I16.T03.a |
| `I17.U1` | I17.T01.a, I17.T02.a |
| `I17.U2` | I17.T03.a |

Вершины `I06.*` заменяют I06.T01.b–T04.a; `I06.T05.a` — приёмка контроллера.
