"""Пакет Grok, часть 2: запуск приложений (I11), группы (I12), регион (I14), качество (I16), интерфейс (I17).

Данные для pack_dag.py. Формат — как в i06_dag.py.
"""

DIRECTIONS = [
    ("A", "Приложения", "Проверенный запуск приложения в своей сети под своим uid; ярлыки; общий туннель группы."),
    ("R", "Регион", "Страна выхода по нескольким наблюдениям; окружение процесса; что делать, когда страна сменилась."),
    ("Q", "Качество", "Объяснимый выбор узла: сначала ограничения, потом оценка, без дёрганья."),
    ("U", "Интерфейс", "Настоящие состояния туннелей вместо макета — в TUI и апплете."),
]

V = []
E = []


def vertex(vid, direction, title, size, level, deps, files, spec, external=(), covers=()):
    V.append(dict(id=vid, direction=direction, title=title, size=size, level=level, deps=deps,
                  files=files, spec=spec, external=list(external), covers=list(covers)))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


# ───────────────────────────── A: приложения ─────────────────────────────
vertex("I11.A1", "A", "Проверить описание приложения перед запуском.", "M", "L1", [],
       ["src/app/mod.rs", "src/app/spec.rs", "src/lib.rs"],
       """\
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

`SpecError::Display` — фиксированные фразы через `t!`; в `ForbiddenEnv` — только имя переменной, без значения.""",
       covers=["I11.T01.a"])

vertex("I11.A2", "A", "Собрать план запуска: сеть, пользователь, окружение.", "M", "L1", ["I11.A1"],
       ["src/app/plan.rs", "src/identity/plan.rs"],
       """\
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

В `src/identity/plan.rs` — только `pub` у `PARENT_ENV_KEYS` и у функции сборки allowlist, чтобы список был один.""",
       external=["BI.L1", "I06.I3"], covers=["I11.T01.a", "I14.T03.a"])

vertex("I11.A3", "A", "Запускать приложение в его netns операцией контроллера.", "L", "L2", ["I11.A2"],
       ["src/controller/app_ops.rs", "src/controller/dispatch.rs", "src/controller/drop.rs"],
       """\
Заполняет `AppLaunch`, который в I06 отвечает `unsupported`.

`AppLaunch { instance, generation, program, args }`:
1. у экземпляра есть сеть (I08.N6) и файл `resolv.conf` этой сети → иначе `NotRunning`;
2. состояние туннеля допускает запуск: worker запущен и отвечает (`Workers::status` → `running` и `api_ready`) → иначе `NotRunning`; `generation` кадра равно поколению worker-а (его клиент читает из `WorkerStatus`) → иначе `GenerationMismatch`. Запуск в сеть без работающего ядра запрещён: приложение стартовало бы в `Blocked`. Запись о worker-е остаётся и после гибели процесса, поэтому одного `running` мало;
3. `LaunchSpec` из `program` и `args` кадра (правила I11.A1 для пути и аргументов; env из кадра не принимается);
4. `run_as_for(peer.uid, peer.gid)`;
5. запуск: дочерний процесс входит в netns и только потом сбрасывает привилегии.

В `src/controller/drop.rs` — `pub fn drop_into_netns_pre_exec(command, netns_fd: RawFd, resolv: Option<&Path>, run_as, keep_fds, limits) -> Result<(), ControlError>`: первым шагом `setns(netns_fd, CLONE_NEWNET)`; при заданном `resolv` — `unshare(CLONE_NEWNS)`, корень в `MS_REC|MS_SLAVE` и bind этого файла поверх `/etc/resolv.conf` (одного `setns` мало: приложение читало бы DNS хоста); затем шаги `drop_pre_exec`. Отказ любого шага — приложение не запускается. Дескриптор netns открывается в родителе: `/run/netns/cm-<index>` (`O_RDONLY | O_CLOEXEC`) и закрывается сразу после `spawn`; в `test_mode` корень — `<base>/netns`. Рабочий каталог приложения — `HOME`, а если его нет — `/`.

Окружение: allowlist из окружения **клиента** недоступен контроллеру, поэтому `AppLaunch` получает env так: `PATH=/usr/bin:/bin`, `HOME` — домашний каталог uid из `getpwuid_r`, `XDG_RUNTIME_DIR=/run/user/<uid>`, плюс `TZ` и `LANG` из пресета экземпляра, если он записан (файл `<root>/instances/<instance>/env.json` владельца: `{"timezone": …, "locale": …}`, проверки владельца и прав — как у `read_owned_config`).

Ответ — новый вариант `ReplyData::Launched { pid: u32 }`. Контроллер не ждёт завершения приложения, но забирает статус выхода в фоне (нет зомби).

Не входит: cgroup-область на сессию и запись `Session` в Store — следующая итерация (нужен `systemd-run` или делегированный cgroup).""",
       external=["I06.W2", "I08.N6"], covers=["I11.T02.a"])

vertex("I11.A4", "A", "Сгенерировать ярлык приложения с верным экранированием.", "S", "L1", ["I11.A1"],
       ["src/app/desktop.rs"],
       """\
`pub fn desktop_entry(id: &str, name: &str, icon: Option<&str>) -> Result<String, SpecError>` и `pub fn exec_quote(arg: &str) -> String`.

Текст (ровно так, `\\n` в конце каждой строки):
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
- `name`: переводы строк и управляющие символы запрещены (`HasNul`); `\\` → `\\\\` (правило значений Desktop Entry).
- `exec_quote` по спецификации Desktop Entry: аргумент заключается в двойные кавычки, если содержит пробел, табуляцию, перевод строки или любой из символов `"'\\><~|&;$*?#()` и обратную кавычку; внутри кавычек экранируются обратной чертой `"`, обратная кавычка, `$` и `\\`; затем каждый `%` удваивается (`%%`).
- Ярлык запускает `cm app run`, а не программу напрямую: иначе приложение стартовало бы в сети хоста.""",
       covers=["I11.T03.a"])

vertex("I11.A5", "A", "Дать команду `cm app`.", "M", "L1", ["I11.A3", "I11.A4"],
       ["src/app/cli.rs", "src/main.rs", "src/i18n_table.rs"],
       """\
`src/main.rs`: `if args.first() == "app" { exit(cm::app::cli::dispatch(&args[1..])) }` рядом с `identity`.

- `cm app check FILE` — читает `ApplicationDefinition` из JSON-файла, печатает итог `spec::check` (программа, число аргументов, имена переменных без значений). Код 0 или 2.
- `cm app desktop ID NAME [--icon ICON]` — печатает ярлык в stdout. Код 0 или 2.
- `cm app run INSTANCE -- PROGRAM [ARGS…]` — отправляет контроллеру `AppLaunch` (сокет `CM_CONTROLLER_SOCKET`, по умолчанию `/run/cm/controller.sock`), `generation` берёт из ответа `WorkerStatus` этого экземпляра; печатает `pid`. Коды: 0; 2 — использование или `SpecError`; 3 — отказ контроллера (в выводе — его код в квадратных скобках); 4 — контроллер недоступен.
- `cm app` без аргументов — справка, код 2.

Клиент контроллера: `pub fn call(socket: &Path, op: Op) -> Result<Reply, ClientError>` в `src/controller/client.rs` — один кадр, один ответ, тайм-аут 30 с; `id` — 16 случайных hex-символов из `/dev/urandom`.

Строки — через `t!`, переводы на 6 языков в конец `src/i18n_table.rs`.""",
       external=["I06.S1"], covers=["I11.T03.a"])

vertex("I12.A6", "A", "Считать держателей общего туннеля и решать, когда его останавливать.", "M", "L1", [],
       ["src/app/shared.rs"],
       """\
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
- Счётчик не уходит ниже нуля и не зависит от порядка операций разных сессий.""",
       covers=["I12.T02.a"])

vertex("I12.A7", "A", "Решать, что делать с живой сессией при смене назначения.", "S", "L1", ["I12.A6"],
       ["src/app/reassign.rs"],
       """\
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

Живое соединение никогда не переадресуется молча (Q13): функция не имеет варианта «переключить на лету».""",
       covers=["I12.T04.a"])

# ───────────────────────────── R: регион ─────────────────────────────
vertex("I14.R1", "R", "Определять страну выхода по нескольким наблюдениям.", "M", "L1", [],
       ["src/env/mod.rs", "src/env/geo.rs", "src/lib.rs"],
       """\
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

`region_axis`: `Agreed` и страна == `preset_country` → `Verified`; `Agreed` с другой страной → `Blocked`; `Disagree` → `Partial`; `Stale` и `Insufficient` → `Unknown`.""",
       covers=["I14.T01.a"])

vertex("I14.R2", "R", "Проверить, что окружение региона применимо на этой системе.", "S", "L1", ["I14.R1"],
       ["src/env/apply.rs"],
       """\
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

Порядок результата — порядок перечисления вариантов. Система не меняется: функция только читает.""",
       external=["BI.R3"], covers=["I14.T03.a", "I14.T04.a"])

vertex("I14.R3", "R", "Решать, что делать с сессией, когда страна выхода сменилась.", "S", "L1", ["I14.R1"],
       ["src/env/invalidate.rs"],
       """\
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

Приложение при этом не перезапускается и не переключается: вызывающий только меняет ось REGION и показывает причину (I14.T05).""",
       covers=["I14.T05.a"])

# ───────────────────────────── Q: качество ─────────────────────────────
vertex("I16.Q1", "Q", "Отбирать узлы ограничениями и ранжировать оценкой с объяснением.", "M", "L1", [],
       ["src/quality/mod.rs", "src/quality/score.rs", "src/lib.rs"],
       """\
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

`ranked` — по возрастанию `score`, при равенстве — по имени узла (детерминированно). `rejected` — в порядке входа. Ограничение никогда не ослабляется ради того, чтобы «хоть что-то выбрать»: пустой `ranked` — допустимый ответ.""",
       covers=["I16.T01.a", "I16.T02.a"])

vertex("I16.Q2", "Q", "Переключать узел только при заметном и устойчивом выигрыше.", "S", "L1", ["I16.Q1"],
       ["src/quality/switch.rs"],
       """\
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

Переключение узла у живой сессии приложения — отдельное решение I12.A7 (`RestartRequired`); эта функция отвечает только на вопрос «какой узел лучше».""",
       covers=["I16.T03.a"])

# ───────────────────────────── U: интерфейс ─────────────────────────────
vertex("I17.U1", "U", "Строить представление туннелей из настоящих сессий и проверок.", "M", "L1", [],
       ["src/status/tunnels.rs", "src/status.rs", "src/tui/mock_tunnels.rs"],
       """\
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
- `failure`: `net` = `error` → `api-down`; `region` = `blocked` → `region-mismatch`; иначе `None` (первое сработавшее).""",
       external=["I17-D.T05.a"], covers=["I17.T01.a", "I17.T02.a"])

vertex("I17.U2", "U", "Выбирать значок апплета по состояниям туннелей.", "S", "L1", ["I17.U1"],
       ["cosmic/src/tunnel_icons.rs"],
       """\
`pub fn badge_for(views: &[TunnelView]) -> Option<TunnelBadge>` в существующем `cosmic/src/tunnel_icons.rs` (тип `TunnelBadge` уже есть; `TunnelView` — из `cm::status::tunnels`).

Приоритет, первое сработавшее:
1. у любого туннеля `condition == "blocked"` → `Blocked`;
2. у любого туннеля `condition == "degraded"` → `Partial`;
3. у любого туннеля `host == "off"` и `sessions > 0` → `HostOffAppActive`;
4. иначе `None` — апплет показывает обычный значок обновлений.

Подключение значка в панель (`cosmic/src/panel.rs`) в эту вершину не входит: источник `TunnelView` в апплете появится вместе с контроллером на установке.""",
       covers=["I17.T03.a"])

# ───────────────────────────── рёбра ─────────────────────────────
edge("M01", "I11.A1", "I11.A2",
     "Описание приложения не может подменить программу, окружение региона или библиотеки.",
     "`tests/audit_pack_app.rs`, табличный тест.",
     [
         "executable \"/usr/bin/firefox\", argv [\"--new-window\"], env [{MOZ_ENABLE_WAYLAND, 1}] → Ok(LaunchSpec{program /usr/bin/firefox, args [--new-window], env {MOZ_ENABLE_WAYLAND: 1}})",
         "executable \"firefox\" → NotAbsolute; \"/usr/../bin/sh\" → DotDot; с NUL → HasNul",
         "257 аргументов → TooManyArgs; аргумент 8193 байта → ArgTooLong",
         "cwd \"relative\" и \"/a/../b\" → BadCwd",
         "env имя \"1A\", \"A-B\", \"\" → BadEnvName; два раза A → DuplicateEnv",
         "env LD_PRELOAD, LD_LIBRARY_PATH, TZ, LANG, LANGUAGE, LC_ALL, LC_TIME, PATH, HOME, DBUS_SESSION_BUS_ADDRESS → ForbiddenEnv с этим именем",
         "ForbiddenEnv(\"TZ\") в Display не содержит значения переменной",
     ])
edge("M02", "I11.A2", "I11.A3",
     "Окружение процесса состоит только из allowlist, переменных описания и региона; регион перекрывает всё.",
     "`tests/audit_pack_app.rs`.",
     [
         "session_env {DISPLAY=:0, HOME=/h, SECRET_TOKEN=x, TZ=Europe/Moscow, LC_TIME=ru_RU.UTF-8}, spec.env {A=1}, preset {timezone Europe/Berlin, locale de_DE.UTF-8} → env == {A=1, DISPLAY=:0, HOME=/h, LANG=de_DE.UTF-8, PATH=/usr/bin:/bin, TZ=Europe/Berlin}",
         "в env нет SECRET_TOKEN, LC_TIME",
         "plan.netns == переданному; plan.run_as == переданному",
         "identity::plan::PARENT_ENV_KEYS — один список на оба плана (в `src/app/plan.rs` нет своей копии имён DISPLAY, WAYLAND_DISPLAY)",
     ])
edge("M03", "I11.A3", "I11.A5",
     "Приложение запускается внутри сети своего туннеля, под uid вызывающего, и только при работающем ядре.",
     "`tests/audit_pack_app_ops.rs`: `unshare -U --map-root-user --map-auto -n -m` (tmpfs на /run для `ip netns`; с `unshare -r` вызов `setgroups` запрещён и `worker_start` отвечает `failed`), в `<base>/netns` — ссылка `cm-0` на `/run/netns/cm-0`, контроллер с `SystemExec` и `CM_TEST_MIHOMO`; приложение — `/bin/sh -c`, пишущее свои `ip -br addr`, `id -u` и `env` в файл.",
     [
         "app_launch без сети → not_running; с сетью, но без worker → not_running",
         "после `kill -9` процесса ядра: app_launch → not_running (запись о worker-е есть, api не отвечает)",
         "generation кадра ≠ поколению worker-а → generation_mismatch; после worker_reload на поколение 2 запуск с generation 2 → ok",
         "файл приложения: `/etc/resolv.conf` == \"nameserver 198.18.0.2\\noptions edns0\\n\"; `/etc/resolv.conf` вне приложения не изменился",
         "после 5 запусков число открытых дескрипторов контроллера (`/proc/<pid>/fd`) не выросло",
         "env.json — симлинк, файл с записью для группы или длиннее 4096 байт → invalid_config; timezone `../x` → invalid_config",
         "после net_apply и worker_start: app_launch → ok, data {type launched, pid N}",
         "файл приложения: интерфейсы только lo и cmv0n с адресом 10.213.0.2/30 (нет интерфейсов хоста)",
         "env приложения: PATH, HOME, XDG_RUNTIME_DIR и TZ/LANG из env.json; нет переменных контроллера",
         "приложение с NUL или относительным program в кадре → bad_argument (отсекает декодер)",
         "после завершения приложения у контроллера нет зомби-потомков",
         "с подчинённым uid (`--map-users`): `id -u` приложения == uid клиента, CapEff 0",
     ], level="L2")
edge("M04", "I11.A1", "I11.A4",
     "Ярлык не может выполнить ничего, кроме `cm app run <id>`.",
     "`tests/audit_pack_app.rs`.",
     [
         "desktop_entry(\"browser\", \"Браузер\", None) == \"[Desktop Entry]\\nType=Application\\nName=Браузер\\nExec=cm app run browser\\nTerminal=false\\nX-CM-Application=browser\\n\"",
         "с icon \"firefox\" — строка `Icon=firefox` между Exec и Terminal",
         "exec_quote(\"plain\") == \"plain\"; exec_quote(\"a b\") == \"\\\"a b\\\"\"; exec_quote(\"a$b\") == \"\\\"a\\\\$b\\\"\"; exec_quote(\"50%\") == \"50%%\"; exec_quote(\"a\\\"b\") == \"\\\"a\\\\\\\"b\\\"\"",
         "name с переводом строки → Err; name \"a\\\\b\" → строка `Name=a\\\\\\\\b`",
         "id \"../x\" или \"a b\" → Err(BadId)",
     ])
edge("M05", "I11.A4", "I11.A5",
     "Команда печатает ровно текст ярлыка.",
     "`tests/audit_pack_app.rs`: бинарник `cm`.",
     [
         "`cm app desktop browser Браузер` → stdout == строка из M04, код 0",
         "`cm app desktop ../x N` → код 2",
     ])
edge("M06", "I11.A5", "PACK.Z",
     "`cm app` различает ошибки использования, отказы контроллера и его недоступность.",
     "`tests/audit_pack_app.rs`: бинарник `cm` с `CM_CONTROLLER_SOCKET` на подставной сервер из теста.",
     [
         "`cm app` → справка, код 2; справка на en, de, it, zh, ar без кириллицы",
         "`cm app check FILE` с корректным описанием → код 0, в выводе нет значений переменных; с LD_PRELOAD → код 2 и [ForbiddenEnv]",
         "`cm app run browser -- /usr/bin/true`: клиент шлёт worker_status, затем app_launch с generation из ответа; ответ launched pid 42 → stdout содержит 42, код 0",
         "сервер отвечает code not_running → код 3, вывод содержит [not_running]",
         "сокета нет → код 4",
         "`cm app run browser -- true` (не абсолютный путь) → код 2 без обращения к сокету",
     ])
edge("M07", "I12.A6", "I12.A7",
     "Туннель группы живёт, пока есть хотя бы одна активная сессия; туннель хоста сессиями не управляется.",
     "`tests/audit_pack_app.rs`: сценарии и перебор порядков.",
     [
         "acquire(t, s1) → StartTunnel(t); acquire(t, s2) → None; holders 2",
         "acquire(t, s1) повторно → None; holders 2",
         "release(t, s1, Group) → None; release(t, s2, Group) → StopTunnel(t); holders 0",
         "release(t, s9, Group) для неизвестной сессии → None; holders не уходит ниже 0",
         "release последней сессии с owner Host → None",
         "все 24 перестановки операций [acquire s1, acquire s2, release s1, release s2] при корректном порядке внутри сессии дают ровно один StartTunnel и один StopTunnel",
         "rebuild: из 3 сессий (2 Active на t, 1 Ended на t) → holders(t) == 2",
     ])
edge("M08", "I12.A7", "PACK.Z",
     "Смена назначения никогда не переадресует живую сессию молча.",
     "`tests/audit_pack_app.rs`, табличный тест.",
     [
         "нет сессии, то же назначение → NoChange; другое → AppliesToNextLaunch",
         "сессия на OwnTunnel{t1}, wanted OwnTunnel{t2} → RestartRequired{TunnelChanged}",
         "сессия на OwnTunnel{t1}, wanted Group{g} → RestartRequired{TunnelChanged}",
         "то же назначение, session.tunnel_generation 3, текущее 4 → RestartRequired{GenerationChanged}",
         "node_present false при любом назначении → RestartRequired{NodeGone}",
         "то же назначение, то же поколение, узел есть → NoChange",
     ])
edge("M09", "I14.R1", "I14.R2",
     "Страна выхода подтверждается только согласием нескольких свежих источников.",
     "`tests/audit_pack_env.rs`: фикстуры `tests/fixtures/pack/region.json`.",
     [
         "два источника DE, оба 1 минуту назад → Agreed{DE, 2}",
         "DE и NL → Disagree{[DE, NL]}; три источника DE, DE, NL → Disagree{[DE, NL]}",
         "один свежий источник → Insufficient{1}; пусто → Insufficient{0}",
         "два источника 16 минут назад → Stale; один свежий и один старый → Insufficient{1}",
         "один источник дал DE (10 мин назад) и NL (1 мин назад), второй — NL → Agreed{NL, 2} (берётся свежее от источника)",
         "страна \"Germany\", \"de\", \"\" и наблюдение из будущего отбрасываются",
         "region_axis: Agreed{DE} при пресете DE → Verified; при пресете NL → Blocked; Disagree → Partial; Stale и Insufficient → Unknown",
     ])
edge("M10", "I14.R2", "PACK.Z",
     "Проверка окружения называет каждую проблему и ничего не меняет в системе.",
     "`tests/audit_pack_env.rs`: tzdir-фикстура в `TempDirGuard`.",
     [
         "preset {Europe/Berlin, de_DE.UTF-8, [de-DE, de]}, зона есть, locales [de_DE.utf8, en_US.utf8] → []",
         "зоны нет → [ZoneMissing]; timezone \"../etc/passwd\" → [ZoneMissing]",
         "locales [en_US.utf8] → [LocaleNotInstalled]",
         "languages [] → [LanguagesEmpty]; locale de_DE.UTF-8 при languages [fr-FR] → [LocaleLanguageMismatch]",
         "parse_locale_list(\"C\\nC.utf8\\nde_DE.utf8\\n\") == [C, C.utf8, de_DE.utf8]; normalize_locale(\"de_DE.UTF-8\") == normalize_locale(\"de_DE.utf8\")",
     ])
edge("M11", "I14.R1", "I14.R3",
     "Смена страны выхода меняет только ось REGION; приложение не перезапускается и не переключается.",
     "`tests/audit_pack_env.rs`.",
     [
         "preset DE, current Agreed{DE} → Keep → Verified",
         "current Agreed{NL} → RegionMismatch{expected DE, observed NL} → Blocked",
         "current Disagree, Insufficient → RegionUnknown → Unknown",
         "previous Agreed{DE}, current Stale → RegionUnknown (подтверждение не продлевается)",
     ])
edge("M12", "I14.R3", "PACK.Z",
     "Функции региона чистые: не читают сеть, файлы и часы.",
     "`tests/audit_pack_env.rs` и проверка исходников.",
     [
         "в `src/env/geo.rs` и `src/env/invalidate.rs` нет `std::fs`, `std::net`, `SystemTime`, `Command`",
     ])
edge("M13", "I16.Q1", "I16.Q2",
     "Ограничения применяются раньше оценки и никогда не ослабляются; ранжирование детерминировано.",
     "`tests/audit_pack_quality.rs`: фикстуры `tests/fixtures/pack/quality.json`.",
     [
         "score(100, 0) == 100; score(100, 2) == 200; score(80, 1) == 130",
         "кандидаты a{120 мс, 0 %}, b{80 мс, 1 %}, c{нет замера} без ограничений → ranked [a(120), b(130)], rejected [(c, NoMeasurement)]",
         "countries [DE]: узел NL → CountryNotAllowed; узел без страны → CountryUnknown",
         "protocols [vless]: узел ss → ProtocolNotAllowed; max_delay_ms 100: узел 120 мс → TooSlow; loss 21 % → TooLossy, loss 20 % проходит",
         "все узлы отвергнуты → ranked пуст (ограничение не снимается)",
         "равные score: порядок по имени узла; перестановка входа не меняет ranked",
     ])
edge("M14", "I16.Q2", "PACK.Z",
     "Узел не меняется из-за шума измерений и не меняется на произвольный, когда кандидатов нет.",
     "`tests/audit_pack_quality.rs`.",
     [
         "ranked [] → Stay{NoCandidates}",
         "current None → To{ranked[0]}; current отсутствует в ranked → To{ranked[0]} даже при last_switch 1 с назад",
         "current == ranked[0] → Stay{AlreadyBest}",
         "current score 100, лучший 85 (выигрыш 15 %) → Stay{GainTooSmall}; лучший 80 (20 %) → To",
         "выигрыш 50 %, но с прошлого переключения 59 с → Stay{DwellNotElapsed}; 60 с → To",
     ])
edge("M15", "I17.U1", "I17.U2",
     "Представление туннеля строится из проверок текущего поколения и не сводит оси в один «зелёный» статус.",
     "`tests/audit_pack_status.rs` и существующие снимки `mock_tunnels` (не должны измениться).",
     [
         "host Proxy, туннель owner Application, поколение 4, одна активная сессия, проверки поколения 4: net Verified 40 с назад, region Partial 300 с назад → {host proxy, apps separate, sessions 1, axes {net verified, region partial, state unknown, app unknown}, age_s {net 40, region 300, state None, app None}, condition degraded, failure None}",
         "проверка net поколения 3 (Verified) при туннеле поколения 4 не учитывается → net unknown",
         "две проверки одной оси: берётся с большим evidence_at",
         "любая ось blocked → condition blocked; все verified → condition unknown (значения ok нет)",
         "net error → failure api-down; region blocked → failure region-mismatch; оба сразу → api-down",
         "owner Group → apps shared; сессии Ended не считаются",
         "evidence_at в будущем → age 0",
         "24 снимка `tests/fixtures/i17/snapshots/*.txt` проходят без перезаписи после замены Scenario на TunnelView",
     ])
edge("M16", "I17.U2", "PACK.Z",
     "Значок апплета отражает худшее состояние; при отсутствии туннелей апплет не меняется.",
     "тест в `cosmic/src/tunnel_icons.rs` (cargo test в cosmic).",
     [
         "[] → None",
         "[{condition blocked}, {condition degraded}] → Blocked",
         "[{condition degraded}] → Partial",
         "[{host off, sessions 2, condition unknown}] → HostOffAppActive",
         "[{host tunnel, sessions 2, condition unknown}] → None",
     ])
