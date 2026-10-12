# Роль 1, Grok 4.7: Завершающий пакет: доводка A и B, интерфейс, установка и возврат, очередь хоста, приёмка — код

Дата: 2026-10-11. Координатор: Claude. Граф и контракты: [FIN-DAG.md](FIN-DAG.md). Основания: [REVIEW-2026-10-11-AB.md](REVIEW-2026-10-11-AB.md), [AB-DAG.md](AB-DAG.md), [ADR-HOST-FLOW.md](ADR-HOST-FLOW.md), [ADR-DNS.md](ADR-DNS.md), [ADR-XRAY-DATAPATH.md](ADR-XRAY-DATAPATH.md), [ADR-BROWSER-IDENTITY.md](ADR-BROWSER-IDENTITY.md), [I17D-NAV.md](I17D-NAV.md), [INSTALL-PLAN.md](INSTALL-PLAN.md), [ITERATIONS.md](ITERATIONS.md) (критерии A01–A20), [QUESTIONS.md](QUESTIONS.md).

## Задача

Реализовать все вершины ниже в порядке зависимостей. Исполнитель — Grok 4.7.

Волны; каждая собирается и проходит gate отдельно:
1. **Дефекты сторожа и политики, обзор, хранилище:** R.P1, R.W1, R.W2, R.W3, C.O1, C.S1.
2. **Команды:** R.T1, R.A9, C.E1.
3. **Установка и возврат:** I.H1, I.S1, I.T1.
4. **Модель состояния:** C.M2, C.M1, C.C1, C.D1.
5. **Интерфейс:** C.T1, C.A1.
6. **Очередь хоста:** X.Q0, X.Q1, X.Q2, X.Q3, X.Q4, X.Q5.
7. **Документы и приёмка:** P.D1, P.T1.

После волны 3 сдай промежуточный отчёт: роль 2 начинает проверки, не дожидаясь волн 4–7.

Волны 1 и 3 — код P0: он решает, куда идёт трафик и останется ли у машины сеть. Здесь действует правило 14.4 без исключений: при сомнении — блокировка.

Что уже есть и не переписывается: контроллер и его протокол, сеть приложения и хоста, сторож, запуск приложений, регион, личность браузера, TUI и апплет прежнего VPN. Вершины добавляют файлы и точечные правки. Код пакетов A и B в рабочем дереве не закоммичен: он — основа, поверх которой ты работаешь.

Тесты пишет роль 2 (ChatGPT Luna 6) по рёбрам. Поэтому точные значения из рёбер — часть спецификации, а не пожелание.

## Правила

1. **Перед сдачей каждой волны — полный gate, EXIT=0:**
   `CARGO_TARGET_DIR=$HOME/.cache/cm-grok-target CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo CM_TEST_XRAY=$HOME/.cache/cm-cores/xray/xray sh tests/check_audit.sh`.
   Тест стенда `audit_ab_e2e` до работы роли 2 может падать из-за гонки ожидания: это единственный допустимый красный тест, и только с отказом вида `NO OUTPUT`.
2. **Переформатирование существующих файлов запрещено.**
   - `cargo fmt` запрещён всегда.
   - `rustfmt` запрещён для `src/main.rs`, `src/lib.rs`, `src/tui.rs`, `src/status.rs`, `src/i18n_table.rs`, `src/core/adapter.rs`, `src/core/mod.rs`, существующих файлов `cosmic/src/` и любого файла, которого нет в проверке формата в `tests/check_audit.sh`. `rustfmt src/main.rs` форматирует ещё `tui.rs` и `tui/*`, `rustfmt src/lib.rs` — весь крейт.
   - Файлы, которые gate уже проверяет на формат (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`, `src/core/mihomo/*.rs`), обязаны проходить `rustfmt --edition 2024 --check` и после правки. Форматировать — только тот файл, который правил.
   - Новые файлы вне этих каталогов (`src/status/live.rs`, `resources.rs`, `page.rs`, `panel.rs`, `src/diag/*.rs`, `src/profiles/edit.rs`, `src/tui/tunnels.rs`, `cosmic/src/tunnels.rs`) проверяет новый файл `tests/check_fin.sh`. В волне 1 создай его и добавь в `tests/check_audit.sh` **одну** строку `sh tests/check_fin.sh` (перед строкой clippy); другие строки `check_audit.sh` не менять. В `check_fin.sh`: `rustfmt --edition 2024 --check` и `python3 -m py_compile` для своих файлов — каждый только если он уже существует; последняя строка — `[ ! -f tests/check_fin_tests.sh ] || sh tests/check_fin_tests.sh`. Файл `tests/check_fin_tests.sh` принадлежит роли 2: не создавай и не правь его.
   - Проверка перед сдачей: `git diff --stat` по существующим файлам — только смысловые строки.
3. Сборка только с `--offline --locked`. Новых зависимостей нет.
4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога теста. Команды `ip`, `nft`, `sysctl`, `cm host set`, `cm controller teardown` выполняются только внутри `unshare`; на хосте — никогда. Шаги `tests/hostq/` ты пишешь, но не запускаешь: их запускает пользователь. Настоящий `pkcheck` не вызывается.
5. Каждый `unsafe` — с `// SAFETY:`. В `pre_exec` — только async-signal-safe вызовы.
6. **Тесты пишет роль 2 (ChatGPT Luna 6).** От тебя — код, который выполняет контракты рёбер (точные значения, коды, порядок проверок и команд). Новых тестовых файлов не создавай. Всё, что роль 2 должна проверить без терминала и без настоящей машины, лежит в библиотеке (`src/lib.rs` и модули), а не в `src/main.rs`, `src/tui.rs` и `cosmic/`.
7. **Разрешённые правки существующих тестов** — только чтобы они собирались; ничего сверх списка:
   - `tests/audit_i06_protocol.rs`: новые варианты `Op` (`Overview`, `Teardown`) в перечислении и в `match`;
   - `tests/audit_ab_*.rs`, `tests/ab_support/mod.rs`: только если изменилась сигнатура, которую тест вызывает, — добавить аргумент или поле; ожидания не менять.
   Существующий тест падает по иной причине → правило 15.
8. **Никаких данных запроса в ответах и ошибках.** Ответ — код из фиксированного набора и `ReplyData`. Пути, argv, env, байты конфига, адреса узлов и тексты ошибок ОС не попадают ни в ответ, ни в `Debug`, ни в вывод команд, ни в evidence.
9. **uid — только из сокета.** Место, где uid, путь или каталог владельца берётся из тела запроса, — отказ при ревью.
10. **Сеть — блокировка по построению.** Порядок команд `net::commands`, `net::firewall`, `host_create_commands`, текст таблиц nftables не меняются. Починка и снятие строятся из тех же команд.
11. Новые поля в `Deps` не добавлять: общее состояние передаётся через `Workers`.
12. Новые строки интерфейса — через `t!` и `src/i18n_table.rs`; токены в квадратных скобках и слова вида `state=up` не переводятся.
13. Коммит — только по поручению пользователя. Сдача — отчёт.

### Как действовать без вопросов пользователю

14. **Неясность в спецификации.** Выбери толкование в этом порядке и запиши его в отчёт одной строкой; не останавливайся:
    1. как уже делает существующий код рядом и что проверяют его тесты;
    2. что сказано в ADR из раздела «Основания»;
    3. что следует из значений ребра;
    4. более строгий вариант: отказ вместо разрешения, блокировка вместо пропуска, `unknown` вместо `verified`.
15. **Вершина не получается** (gate красный, а причина вне разрешённых правок; два ребра противоречат друг другу; нужна привилегия или сеть хоста). Действия:
    1. убери свои правки этой вершины обратным патчем из `git diff` своих строк. `git checkout`, `git stash`, `git reset` запрещены: в дереве лежит незакоммиченная работа прошлых пакетов;
    2. запиши в отчёт: `<вершина>: open — <причина одной фразой>, <что пробовал>`;
    3. продолжай с вершинами, которые от неё не зависят; зависящие запиши `open: ждёт <вершина>`.
16. **Дефекты от роли 2.** В начале каждой волны прочитай `docs/design/cm-network-manager/fin-evidence/*/defects.json`. Записи `severity` P0 и P1 исправляются раньше новых вершин. Не согласен с дефектом (тест проверяет не то, что сказано в ребре) → не правь код, запиши `<ребро>.<n>: спор — <почему>`; решит координатор.
17. **К пользователю — только три случая:** в выводе или файле оказался секрет либо потеряны данные; для продолжения нужно изменить сеть хоста или что-то установить; нужен коммит или push. Во всех остальных случаях действуют правила 14–16, а вопросы идут координатору в отчёте.

## Вершины по порядку

### R.P1 — Отклонять политику туннеля, которая не является объектом JSON.

Файлы: `src/net/policy.rs`. Зависит от: —.

Дефект D01: `parse(b"[]")` возвращает политику по умолчанию. Причина — serde разбирает структуру и из массива.

```rust
pub fn parse(bytes: &[u8]) -> Result<TunnelPolicy, NetError>;
```

Порядок:
1. `bytes.len() > MAX_POLICY_BYTES` → `NetError::Unsupported`;
2. разбор в `serde_json::Value`; ошибка или не объект → `NetError::Unsupported`;
3. разбор объекта в `TunnelPolicy` (как сейчас: неизвестное поле — отказ, отсутствующее — значение по умолчанию).

Сигнатура и остальные типы файла не меняются.

Контракты, которые проверит роль 2:
- **RP01 → R.Z:** Политика читается только из объекта JSON ограниченного размера.
  - `parse(b"[]")`, `parse(b"null")`, `parse(b"1")`, `parse(b"\"x\"")` → `Err(NetError::Unsupported)`
  - `parse(b"{}")` → политика по умолчанию: ipv6 Block, fake_ip Forbidden, lifetime Pinned, failure Block
  - объект длиной `MAX_POLICY_BYTES + 1` (допустимый JSON, дополненный пробелами) → `Err`
  - `{"lifetime":"refcount","extra":1}` → `Err`: неизвестное поле

### R.W1 — Давать каждой автоматической остановке туннеля свой ключ операции.

Файлы: `src/controller/watch.rs`. Зависит от: —.

Дефект: `stop_instance` вызывает `workers.stop` и `net_ops::revert` с постоянными ключами `watch-stop` и `watch-revert`. Вторая автоматическая остановка того же экземпляра находит в журнале завершённую операцию с этим ключом и возвращает её сохранённый ответ: ядро и сеть остаются.

```rust
/// "auto-<kind>-<now_ms>-<seq>"; kind — "stop" или "revert".
fn auto_txn(kind: &str, now_ms: i64, seq: u32) -> String;
```

- В `Watch` — поле `seq: u32`; перед каждым использованием увеличивается (`wrapping_add(1)`).
- В `Watch` — поле `proc_root: PathBuf` (по умолчанию `/proc`) и конструктор `Watch::with_proc_root(PathBuf)`: подсчёт держателей читает его вместо постоянного `/proc`. Нужен, чтобы роль 2 проверяла срок жизни на дереве-фикстуре.
- Ключ и дайджест операции — одна и та же строка `auto_txn(...)`. Отрицательное `now_ms` заменяется нулём. Длина при любых значениях ≤ 64, алфавит `a-z0-9-` (правило `valid_id` в `codec.rs`).
- Время операции, которое уходит в журнал, — `now_ms / 1000` (сейчас передаётся 0).
- Порядок в `stop_instance` не меняется: сначала ядро, потом сеть.
- Ядро не остановилось (ошибка, кроме `NotRunning`) → сеть **не снимается**. Состояние экземпляра — `Blocked`, `empty_since_ms = None`: следующая попытка — через новый срок ожидания.
- Обе операции прошли → как сейчас: `Stopped`, признаки сброшены.

Контракты, которые проверит роль 2:
- **RW01 → R.Z:** Автоматическая остановка срабатывает каждый раз, а не только первый.
  - политика refcount: запуск, держатели ушли, прошло 5000 мс → ядро остановлено, в записи исполнителя есть команды снятия сети, состояние stopped
  - тот же экземпляр поднят снова, держатели ушли, прошло 5000 мс → остановлен второй раз; в журнале владельца две завершённые операции с разными ключами вида `auto-stop-<число>-<число>`
  - `auto_txn("revert", i64::MAX, u32::MAX)` — длина 42, только `a-z0-9-`; `auto_txn("stop", -5, 1)` = `auto-stop-0-1`
  - подменное ядро отвечает ошибкой на остановку → команд снятия сети в записи нет, состояние blocked
  - политика pinned при тех же событиях → ни одной команды, состояние up

### R.W2 — Повторять перезапуск ядра по расписанию, пока не исчерпан лимит.

Файлы: `src/controller/watch.rs`, `src/controller/core_ops.rs`. Зависит от: —.

Дефект: `Workers::restart` убирает запись ядра до запуска. Если запуск не удался, экземпляра больше нет в `workers.running()`, сторож к нему не возвращается: вторая и следующие попытки не выполняются, состояние не становится `failed`.

Изменения:
- Шаг «ядра» в `Watch::tick` обходит объединение `workers.running()` и экземпляров, у которых `restart_at_ms` задан. Без повторов, в порядке сортировки по `(uid, instance)`.
- Срок перезапуска наступил:
  - `workers.restart` → `Ok`: `restart_at_ms = None`. Состояние меняет следующий проход, когда ядро ответит (`Blocked` + `CoreReady` → `Up`).
  - `Err`: это новый отказ. `failure::decide(Failure::CoreDown, …)`:
    - `RestartCore { delay_ms }` → `restart_at_ms = now_ms + delay_ms`;
    - `GiveUp` → состояние `Failed`, `restart_at_ms = None`.
- У экземпляра нет записи ядра и нет назначенного перезапуска → шаг его пропускает.
- `Workers::restart` при ошибке запись не восстанавливает (как сейчас). Сеть при этом не трогается: TUN постоянный, трафик приложения в нём пропадает.
- Команда клиента `worker_start` для экземпляра в `Failed` работает как сейчас и через `note_start` обнуляет счётчики.

Расписание при проходах каждые 2000 мс и ядре, которое не запускается (задержки `RESTART_DELAYS_MS`):

| Проход, мс | Событие | Следующая попытка, мс |
|---|---|---|
| 0 | ядро не отвечает: `Up` → `Blocked`, restarts = 1 | 0 |
| 2000 | попытка 1 не удалась, restarts = 2 | 3000 |
| 4000 | попытка 2 не удалась, restarts = 3 | 9000 |
| 10000 | попытка 3 не удалась, restarts = 4 | 40000 |
| 40000 | попытка 4 не удалась, restarts = 5 | 160000 |
| 160000 | попытка 5 не удалась → `Failed` | — |

Окно `WINDOW_MS` и лимиты в `failure.rs` не меняются.

Контракты, которые проверит роль 2:
- **RW02 → R.Z:** Перезапуски ядра идут по расписанию до лимита, затем туннель объявляется отказавшим.
  - ядро не запускается, проходы каждые 2000 мс от 0 до 160000: попытки запуска ровно на проходах 2000, 4000, 10000, 40000, 160000 — всего 5
  - после прохода 160000: `tunnel_state` → state failed, restarts 5; на проходах 162000 и 164000 попыток нет
  - между проходами 0 и 160000 состояние blocked; команд `ip` и `nft` от шага «ядра» в записи исполнителя нет
  - ядро запускается с третьей попытки (проход 10000) → на проходе 12000 состояние up, restarts 3
  - `worker_start` после failed → состояние starting, restarts 0

### R.W3 — Сверять и чинить сеть туннеля хоста на каждом проходе сторожа.

Файлы: `src/net/host_audit.rs`, `src/net/mod.rs`, `src/controller/watch.rs`, `src/controller/boot.rs`. Зависит от: —.

Сейчас сторож следит только за сменой внешних интерфейсов. Если правило 2000 или 2001, приёмник или таблица `ip cm_host` пропали, хост в режиме туннеля выходит напрямую, и этого никто не замечает.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostDrift {
    MissingSink,
    MissingSinkRoute { v6: bool },
    MissingTun,
    MissingTunRoute,
    MissingRule { v6: bool, priority: u32 },
    MissingFirewall,
}

pub fn host_audit(
    host: &HostNet,
    uplinks: &[Uplink],
    observed: &Observed,          // links и routes для таблиц 90 и 91, правила IPv4
    rules6: &[ObservedRule],      // `ip -j -6 rule`
    routes6_sink: &[ObservedRoute], // `ip -j -6 route show table 91`
    firewall_present: bool,       // `nft list table ip cm_host` завершился успешно
) -> Vec<HostDrift>;

pub fn host_repair_commands(host: &HostNet, uplinks: &[Uplink], drift: &[HostDrift]) -> Vec<Cmd>;
```

Ожидаемое состояние — ровно то, что ставит `host_create_commands`:
- интерфейсы `cmsink`, `cmtunh`;
- таблица 91: `default dev cmsink` (IPv4 и IPv6); таблица 90: `default dev cmtunh`;
- правила IPv4: `1900+k` и `1950+j` → `main`, `1990` → `main`, `2000` → `90`, `2001` → `91`;
- правила IPv6: `1900+k` → `main`, `1990` → `main`, `2001` → `91`;
- таблица `ip cm_host`.

Правило считается найденным по паре (приоритет, таблица). Порядок элементов результата `host_audit` — порядок перечисления выше.

`host_repair_commands` — подпоследовательность `host_create_commands` в её же порядке, только для недостающего; `route add` заменяется на `route replace`; последней идёт команда `host_firewall_commands(Some(host))`, если есть `MissingFirewall`. Приёмник и его правило чинятся раньше туннеля — как при создании.

Сторож (`Watch::tick`), шаг «внешние интерфейсы», при `host.json` с режимом `tunnel`:
1. смена внешних интерфейсов — как сейчас;
2. наблюдение: `query::observe(query, &[90, 91])` и три запроса из сигнатуры;
3. `host_audit`; расхождений нет → конец;
4. команды починки выполняются по порядку; первая ошибка прерывает починку до следующего прохода;
5. был `MissingTun` и починка прошла → ядру `host` назначается перезапуск (`restart_at_ms = now_ms`): старое ядро держит удалённое устройство.

Сверка при старте (`boot::reconcile_host`), когда `cmtunh` есть: те же шаги 2–4.

Ошибка любого запроса наблюдения → шаг пропускается (как сейчас при ошибке `observe_uplinks`). Экземпляры приложений этим шагом не затрагиваются.

Весь шаг хоста в стороже (смена внешних интерфейсов, сверка, починка) идёт под блокировкой экземпляра `host` его владельца (`workers.try_begin`). Экземпляр занят операцией клиента (`host_apply`, `host_revert`) → шаг пропускается целиком. Без этого сторож восстанавливает правила, которые в этот момент снимает `cm host set off`.

Контракты, которые проверит роль 2:
- **RW03 → R.Z:** Каждое недостающее звено сети хоста находится и чинится своими командами, приёмник — раньше туннеля.
  - полное состояние при одном внешнем интерфейсе с одним адресом → `host_audit` пуст
  - нет правила IPv4 2001 → `[MissingRule{v6:false, priority:2001}]`; починка — одна команда `ip rule add priority 2001 lookup 91`
  - нет правила IPv6 2001 → `[MissingRule{v6:true, priority:2001}]`; починка — `ip -6 rule add priority 2001 lookup 91`
  - нет `cmtunh` → `[MissingTun, MissingTunRoute]`; починка — четыре команды: tuntap add, addr add, link set mtu up, `ip route replace default dev cmtunh table 90`
  - нет `cmsink` и правила 2000 → в командах починки создание приёмника стоит раньше `ip rule add priority 2000 lookup 90`
  - `firewall_present = false` → `[MissingFirewall]`; починка — одна команда `nft -f -` с текстом `render_host_table`
  - проход сторожа при `MissingTun`: после починки ядру `host` назначен перезапуск на этот же момент времени
  - режим хоста `proxy` или `host.json` нет → запросов наблюдения хоста и команд нет
  - экземпляр `host` занят операцией клиента → шаг хоста пропущен: запросов наблюдения и команд нет, смена внешних интерфейсов тоже не выполняется
- **IT01 → I.T1:** Снятие удаляет ровно те объекты, которые сверяет сторож хоста.
  - `host.json` с одним внешним интерфейсом → команды: `host_firewall_commands(None)`, затем `host_destroy_commands`; файл удалён; запись `net:host` очищена
  - `host.json` нет, наблюдаются `cmsink` и правила 1990, 2000, 2001 → те же команды плюс по одной `ip rule del priority N` на каждое наблюдаемое правило схемы
  - чужое правило с приоритетом 1990 и таблицей 200 и правило с приоритетом 3000 → не удаляются
  - две аренды сетей приложений → для каждой команды снятия, аренда освобождена, каталог `etc_root/cm-<i>` удалён; отчёт `nets = 2`
  - повтор на чистом состоянии → отчёт `{cores:0, nets:0, host:false, errors:0}`

### C.O1 — Отдавать клиенту одним ответом состояние хоста и всех его туннелей.

Файлы: `src/controller/protocol.rs`, `src/controller/dispatch.rs`, `src/controller/core_ops.rs`, `src/controller/watch.rs`. Зависит от: —.

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
- Проверка прав — как у `worker_status`.

Контракты, которые проверит роль 2:
- **CO01 → R.T1:** Обзор отдаёт клиенту поколения ядра и сети отдельно.
  - после `net_apply` с поколением 1 и `worker_start` с поколением 2: строка экземпляра — worker_generation 2, net_generation 1
  - после `worker_stop`: worker_generation пусто, net_generation 1
  - после `net_revert`: каталог экземпляра остаётся, оба поколения пусты, netns пусто
- **CO02 → C.M1:** Обзор перечисляет только экземпляры владельца, с состоянием сторожа и без данных запроса.
  - владелец с экземплярами `b`, `a` и `host` → `tunnels` из двух строк в порядке `a`, `b`; `host` в списке нет
  - экземпляры другого uid в ответ не попадают; чужой владелец туннеля хоста видит `host.mode = off`
  - каталог с именем, не проходящим `InstanceId::new`, пропускается; при 70 экземплярах в ответе 64 строки
  - экземпляр без запусков → state stopped, restarts 0, holders 0, оба поколения и netns пусты
  - класс операции — Status; подменный `Authorizer` с отказом → `denied`
- **ZZ02 → FIN.Z:** Новые операции контроллера не ломают принятое раньше.
  - gate: EXIT 0, строк SKIPPED нет, три прогона подряд
  - число тестов в gate не меньше, чем до пакета, плюс тесты `audit_fin_*`
  - стенд `tests/fin_stand.py`: 5 прогонов подряд, каждый EXIT 0
  - `python3 docs/design/cm-network-manager/tools/test_lint.py --dags AB,FIN --strict` → EXIT 0

### R.T1 — Снимать туннель по фактическим поколениям ядра и сети.

Файлы: `src/app/tunnel.rs`. Зависит от: C.O1.

Дефект: `cm tunnel down` передаёт в `net_revert` поколение ядра. Сеть записана с поколением первого `up`. После `up` → `down --keep-net` → `up` поколения расходятся, `net_revert` отвечает `generation_mismatch`, сеть остаётся. Если ядра нет, подставляется поколение 1.

`down ID [--keep-net] [--force]`:
1. хранилище → имя экземпляра (как сейчас); нет → `[unknown_tunnel]`, код 2;
2. `overview` (вершина `C.O1`) → строка экземпляра. Контроллер недоступен → код 4;
3. строки нет либо оба поколения пусты → код 0, действий нет;
4. `holders > 0` и нет `--force` → `[in_use]`, код 5;
5. `worker_generation` задано → `worker_stop` с ним; ответы `ok` и `not_running` — успех, иной → `[<код>]`, код 3;
6. нет `--keep-net` и `net_generation` задано → `net_revert` с ним; правила те же.

`up ID …`, уточнения к нынешнему порядку:
- состояние `starting`, `degraded` или `blocked` → `[busy]`, код 3: экземпляр восстанавливает сторож;
- состояние `stopped` или `failed`, а `worker_generation` в `overview` задано → сначала `worker_stop` с ним (`ok` и `not_running` — успех);
- новое поколение ядра = max(`worker_generation`, `net_generation`, 0) + 1;
- ответ `conflict` на `net_apply` — успех, только если в `overview` у экземпляра есть `net_generation`; иначе `[conflict]`, код 3.

Остальное в `up`, `status`, `verify` не меняется (формат файла региона — вершина `C.M1`).

Контракты, которые проверит роль 2:
- **RT01 → R.Z:** `cm tunnel down` всегда снимает то, что поднял `up`, и повторяется без ошибки.
  - `up t1` → `down t1 --keep-net` → `up t1` → `down t1`: все четыре — код 0; после последней нет `cm-<i>` в `ip netns list`, нет правила `iif cmv<i>h`, нет ядра
  - `down t1` второй раз → код 0, в журнале контроллера новых операций нет
  - `down t1` при живом процессе в сети → `[in_use]`, код 5; с `--force` → код 0
  - `kill -9` ядра, затем `up t1` до восстановления → `[busy]`, код 3
  - `up` для экземпляра, у которого сеть создана другим клиентом того же владельца (`net_generation` задано) → код 0; для экземпляра без сети при ответе conflict → `[conflict]`, код 3

### R.A9 — Реализовать `cm app run` с проверками упаковки, региона, чужого экземпляра и передачи управления.

Файлы: `src/app/cli.rs`, `src/i18n_table.rs`. Зависит от: —.

Вершина `I11.A9` пакета AB не выполнена: в `src/app/cli.rs` добавлено только поле `session: None`. Модули, на которые она опирается, готовы: `app::assign`, `app::packaging`, `app::instance`, `app::env`, `env::private`, `env::apply::check_preset`.

Спецификация — прежняя, дословно:

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

Дополнения:
- путь сокета — `client::socket_path()` (как в `cm tunnel`), а не разбор переменной в этом файле;
- упаковка проверяется раньше любого обращения к контроллеру: отказ по упаковке даёт код 2 и при недоступном контроллере;
- сообщение справки `cm app` дополняется новыми флагами;
- уборка: перед созданием файла сессии, если `app_list` показал пустой список процессов, удаляются все файлы `config/session-*.json` этого экземпляра (живых сессий нет). Каталоги данных сессий команда не удаляет никогда.

Контракты, которые проверит роль 2:
- **RA01 → R.Z:** `cm app run` выполняет проверки по порядку и останавливается на первом отказе.
  - `cm app run t1 -- /usr/bin/systemd-run --user x` без контроллера → `[unsupported_executor_outside]`, код 2 (не 4)
  - описание с Flatpak → `[unsupported_not_verified]`, код 2
  - пояс региона отсутствует в `/usr/share/zoneinfo` → `[zone_missing]`, код 2; локаль не установлена → `[locale_not_installed]`, код 2, с `--allow-missing-locale` запуск идёт
  - программа уже работает вне сети туннеля → `[running_elsewhere]`, код 5; с `--allow-running` → код 0
  - программа-фикстура `tests/brokers/single_instance.py`, завершившаяся сразу, без новых процессов в сети → `[handed_off]`, код 6
  - обычный запуск → в stdout pid, код 0; через 1 с процесс с этим pid в сети туннеля (`readlink /proc/<pid>/ns/net` совпадает с netns туннеля)
  - `--data isolated --ipc private` → файл `config/session-s<12 шестнадцатеричных>.json` с правами 0600; у процесса `XDG_RUNTIME_DIR` — каталог сессии, `DBUS_SESSION_BUS_ADDRESS` не задан

### R.Z — Заново принять пакеты A и B: проверка у каждого значения каждого ребра.

Файлы: `tests/audit_ab_*.rs`, `tests/ab_stand.py`, `tests/fixtures/ab/blocked.json`, `docs/design/cm-network-manager/fin-evidence/<дата>/ab.json`. Зависит от: R.P1, R.W1, R.W2, R.W3, R.T1, R.A9.

Вершина роли 2, кода продукта нет. Ревью показало: стенд и часть тестов AB проверяют не значения рёбер, а константы и заведомо истинные условия. Работа — привести проверки всех 55 рёбер `AB-DAG.json` к правилам раздела «Рамки прохождения» и пройти `tools/test_lint.py --dags AB`.

Контракты, которые проверит роль 2:
- **RZ01 → FIN.Z:** У каждого значения каждого ребра AB есть проверка, и ни одна проверка не пройдёт на сломанном коде по построению.
  - `python3 docs/design/cm-network-manager/tools/test_lint.py --dags AB` → EXIT 0; «без проверки 0», «замечаний 0»
  - в `tests/audit_ab_*.rs` нет `#[ignore]`: D01, U02 сняты после `R.P1` и `R.A9`; F01 и F02 — ошибка фикстуры (каталог `leases` не создан до `LeaseRegistry::create`), исправляется в тесте
  - в `tests/ab_stand.py` нет циклов ожидания без `sleep`: функция `app()` опрашивает файл каждые 100 мс до срока
  - стенд — 5 прогонов подряд, каждый EXIT 0; gate — 3 прогона подряд, каждый EXIT 0 без строк SKIPPED
  - `tests/fixtures/ab/blocked.json` — не больше 12 значений, все уровня L2, у каждого названы причина и шаг очереди хоста
- **RZ02 → X.Q3:** Шаг очереди хоста повторяет на машине то, что уже прошло на стенде, а не проверяет впервые.
  - каждый `stand_ref` проверок шага `hq-host` — номер значения ребра уровня L2 из `AB-DAG.json` или `FIN-DAG.json` (вид `IT03.3`), и проверка с этой меткой есть в `tests/ab_stand.py` или `tests/fin_stand.py`
  - проверок шага `hq-host` без `stand_ref` не больше трёх, и у каждой указана причина (`needs_real_uplink`, `needs_real_dns`, `needs_suspend`)
- **RZ03 → X.Q4:** Каждый критерий A01–A20 привязан либо к проверке стенда, либо к проверке шага на машине.
  - в таблице 20 строк A01–A20; у каждой — `stand` (метки рёбер) или `host` (имена проверок шага), либо оба
  - строк только со `stand` не больше восьми; A01, A02, A06, A07, A11, A12, A16, A20 имеют `host`
  - каждое ребро из `stand` есть в `AB-DAG.json` или `FIN-DAG.json` и имеет хотя бы одну проверку с меткой в `tests/ab_stand.py` или `tests/fin_stand.py`

### I.H1 — Показать без изменений системы, можно ли на этой машине включить туннель хоста.

Файлы: `src/net/cli.rs`, `src/net/host.rs`, `src/i18n_table.rs`. Зависит от: —.

```
cm host check [--json]
```

Только чтение, без контроллера и без привилегий: запросы через `SystemQuery`, файлы `/run` и `/proc`.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Ok, Warn, Block }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckLine { pub name: &'static str, pub verdict: Verdict, pub token: Option<&'static str> }

pub struct HostFacts {
    pub nft: bool, pub links: Vec<String>, pub rules: Vec<ObservedRule>, pub integration: Integration,
    pub uplinks: usize, pub legacy_unit_active: bool, pub controller_socket: bool, pub host_mode: String,
    pub rp_filter_all: Option<u8>, pub rp_filter_default: Option<u8>,
}
pub fn check_lines(facts: &HostFacts) -> Vec<CheckLine>;   // чистая функция, порядок строк фиксирован
```

| № | name | Условие | Результат |
|---|---|---|---|
| 1 | `nft` | команда `nft` не найдена | Block `nft_missing` |
| 2 | `legacy_vpn` | есть интерфейс `cm-vpn` или активна служба VPN хоста | Block `legacy_vpn_active` |
| 3 | `foreign_autoroute` | есть правило с таблицей `2022` | Block `foreign_autoroute` |
| 4 | `resolved` | `integration.resolved` | Block `resolved_unsupported` |
| 5 | `firewalld` | `integration.firewalld` | Warn `firewalld_unverified` |
| 6 | `network_manager` | `integration.network_manager` | Warn `network_manager_unverified` |
| 7 | `uplinks` | 0 → Block `no_uplink`; больше `MAX_UPLINKS` → Block `too_many_uplinks` | |
| 8 | `controller` | сокета контроллера нет | Block `controller_unavailable` |
| 9 | `stale_net` | режим хоста `off`, а `cmtunh` или `cmsink` существует | Block `stale_host_net` |
| 10 | `rp_filter` | всегда Ok; значение печатается | |

Строка без условия — `Ok` без токена. Вывод: `check=<name> result=<ok|warn|block>[ [<token>]]`, для строки 10 — ещё ` all=<n> default=<n>`. `--json` — массив объектов с теми же полями.

Код выхода: 0 — нет `Block`; 1 — есть. `Warn` на код не влияет.

Решение о включении по-прежнему принимает контроллер (`host::preflight`). Команда ничего не обещает сверх него: это подсказка до запроса пароля.

Контракты, которые проверит роль 2:
- **IH01 → X.Q0:** Проверка готовности хоста — чистая функция от наблюдений с фиксированным порядком строк.
  - чистая машина (nft есть, один внешний интерфейс, сокет есть, режим off) → 10 строк, все `ok`, код 0
  - интерфейс `cm-vpn` → строка 2: `check=legacy_vpn result=block [legacy_vpn_active]`, код 1
  - правило с таблицей 2022 → строка 3 block `[foreign_autoroute]`; resolved → строка 4 block `[resolved_unsupported]`
  - firewalld и NetworkManager → строки 5 и 6 `warn`, код 0
  - нет маршрута по умолчанию → строка 7 block `[no_uplink]`; 9 внешних интерфейсов → block `[too_many_uplinks]`
  - режим off и существует `cmsink` → строка 9 block `[stale_host_net]`
  - строка 10: `check=rp_filter result=ok all=0 default=2` при таких значениях в фактах
  - команда не обращается к сокету контроллера и не выполняет `ip … add|del|set|replace`, `nft -f` (запись подменного исполнителя пуста)

### C.S1 — Держать хранилище туннелей приложений у пользователя, а не в системном каталоге.

Файлы: `src/sources/cli.rs`, `src/i18n_table.rs`. Зависит от: —.

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

Хранилище создаётся при первой записи теми же функциями (`Store::initialize`): права 0700, владелец — пользователь.

Контракты, которые проверит роль 2:
- **CS01 → C.E1:** Путь хранилища зависит только от uid и окружения, и пользователь не пишет в системное хранилище.
  - `store_root_for(1000, …)` при `HOME=/home/u` без других переменных → `/home/u/.local/share/cm/profiles`
  - при `XDG_DATA_HOME=/d` → `/d/cm/profiles`; при `CM_PROFILE_STORE=/x` → `/x` для любого uid
  - `store_root_for(0, …)` при `state_dir=/var/lib/cm` и заданном `HOME` → `/var/lib/cm/profiles`
  - `store_writes_allowed(1000, false)` → true; `(0, false)` → false; `(0, true)` → true
  - `cm app define` во временном `HOME` без `CM_PROFILE_STORE` создаёт `<HOME>/.local/share/cm/profiles` с правами 0700

### C.E1 — Заводить и удалять туннели, приложения, группы и профиль хоста командами.

Файлы: `src/profiles/edit.rs`, `src/profiles/mod.rs`, `src/app/define.rs`, `src/app/mod.rs`, `src/app/cli.rs`, `src/app/tunnel.rs`, `src/net/cli.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: C.S1.

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

Секретов в выводе нет: источник называется идентификатором.

Контракты, которые проверит роль 2:
- **CE01 → C.M1:** Команды описания создают граф, который проходит проверку модели и разрешается назначением.
  - `define_app` с собственным туннелем на снимке с источником `s1` → в кандидате приложение `web`, туннель `t-web`, профиль `p-web`, профиль данных `d-web`, пресет `e-web`; `validate()` → Ok; `assign::application` → instance `t-web`, shared false
  - `define_group g1`, затем два `define_app` с `Group(g1)` → `assign::application` обоих даёт instance `t-g1`, shared true
  - без `--node` → выбор `Policy{name:"auto"}`; с узлом другого источника → `Err(UnknownNode)`
  - идентификатор длиной 31 → `Err(BadId)`; повтор `define_app web` → `Err(Exists)`; `define_app` после `remove_app` с тем же ID → `Err(Exists)`
  - `remove_group` при живом приложении группы → `Err(InUse)`; `remove_app` убирает `t-`, `p-`, `d-`, `e-` и оставляет источник
  - `--exec relative/path` → `Err(BadValue)`; пояс `../x` → `Err(BadValue)`; снимок не изменён
  - `define_host` дважды с разными источниками → одна политика `host-policy`, профиль `p-host` указывает на второй источник
- **CE02 → I.S1:** Команды пишут хранилище одной записью и не трогают работающий туннель.
  - `cm app define web --exec /usr/bin/true --source s1` → `ok web`, код 0; ревизия хранилища выросла ровно на 1
  - `cm app list` → строка `id=web tunnel=t-web shared=no exec=/usr/bin/true`
  - неизвестный источник → `[unknown_source]`, код 2; ревизия не изменилась
  - стенд: `cm tunnel up t-web`, затем `cm app remove web` → `[in_use]`, код 5; после `cm tunnel down t-web` → `ok web`
  - в выводе команд нет адресов узлов и URL источника
- **ZZ01 → FIN.Z:** Пользователь проходит весь путь командами продукта, без файлов, написанных руками, и без прямых кадров контроллеру.
  - `cm app define web --exec <программа-фикстура> --source s1 --zone Europe/Berlin --locale C.UTF-8` → `ok web`
  - `cm tunnel up t-web` → код 0; `cm tunnel list` → `state=up net=verified`
  - `cm app run web` → pid, код 0; процесс в сети туннеля; его запрос — HTTP 200 через подставной узел; счётчик прямых пакетов с адресов `10.213.0.0/16` на интерфейсе «интернета» равен 0
  - процесс видит пояс `Europe/Berlin` (`date +%Z` → `CET` или `CEST`), хост — свой
  - `cm tunnel verify t-web --country DE` при источниках, отвечающих DE → код 0; `cm tunnel list` → `region=verified country=DE`
  - `cm diag export --out -` → код 0, без адреса и пароля узла
  - `cm tunnel down t-web` при живом процессе → `[in_use]`, код 5; после завершения процесса → код 0
  - `cm app remove web` → `ok web`; `cm tunnel list` без строки `t-web`; на стенде нет `cm-*` в `ip netns list`

### I.S1 — Включать туннель хоста из хранилища, без файла конфига.

Файлы: `src/net/cli.rs`, `src/app/assign.rs`, `src/i18n_table.rs`. Зависит от: C.E1.

```
cm host set tunnel (--config ФАЙЛ | --stored)
cm host set proxy --config ФАЙЛ
cm host set off
```

```rust
// src/app/assign.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedHost { pub connection_profile_id: Id, pub source_id: Id, pub selection: NodeSelection, pub dns: DnsPolicy }
pub fn host(snapshot: &GraphSnapshot) -> Result<ResolvedHost, AssignError>;
```

- `assign::host`: политика хоста из снимка → её `connection_profile_id` → профиль подключения → источник. Политики нет или профиль в ней не задан → `UnknownProfile`; профиля нет в снимке → `UnknownProfile`; источника нет → `UnknownSource`. Политику заводит `cm host define` (вершина `C.E1`).
- `--stored`: документ строится `document_from_store` с теми же аргументами, что в `cm tunnel up`; ключ `dns` удаляется; документ пишется как поколение экземпляра `host` тем же путём, каким сейчас копируется файл `--config`. Ошибка `assign::host` → `[unknown_profile]` или `[unknown_source]`, код 2.
- Для `tunnel` нужен ровно один из двух источников конфига; иначе — справка и код 2. `--stored` с `proxy` — код 2.
- Поле `mode` политики хоста в хранилище команда не меняет: действующий режим — в `host.json` контроллера.
- Переходы, откат и коды выхода `cm host set` не меняются.

Контракты, которые проверит роль 2:
- **IS01 → C.T1:** Профиль туннеля хоста разрешается из хранилища до обращения к контроллеру.
  - снимок с политикой хоста, профилем и источником → `assign::host` возвращает источник и выбор узла этого профиля
  - политики хоста нет → `Err(UnknownProfile)`; профиль ссылается на отсутствующий источник → `Err(UnknownSource)`
  - `cm host set tunnel` без источника конфига, с обоими источниками, `cm host set proxy --stored` → код 2, обращений к контроллеру нет
- **IS02 → X.Q3:** `cm host set tunnel --stored` поднимает туннель хоста на узле из хранилища.
  - `cm host define --source s1`, затем `cm host set tunnel --stored` → код 0; `cm host status` → `mode=tunnel`; запрос хоста — HTTP 200, счётчик соединений подставного узла вырос
  - в файле поколения экземпляра `host` нет ключа `dns`; права файла 0600
  - без `cm host define` → `[unknown_profile]`, код 2; режим остался off

### I.T1 — Снимать всё, что поставил CM, одной командой и при удалении пакета.

Файлы: `src/controller/teardown.rs`, `src/controller/mod.rs`, `src/controller/protocol.rs`, `src/controller/dispatch.rs`, `src/controller/server.rs`, `src/main.rs`. Зависит от: R.W3.

Сейчас `cm uninstall` выключает службу контроллера. Ядра останавливаются, а сети приложений, правила 1990–2001, приёмник и таблицы nftables остаются. При включённом туннеле хоста машина после удаления остаётся без сети до перезагрузки.

```
cm controller teardown [--offline] [--base КАТАЛОГ]
```

```rust
// protocol.rs
Op::Teardown                                   // класс Host, instance() → None
ReplyData::Teardown { cores: u32, nets: u32, host: bool, errors: u32 }

// teardown.rs
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TeardownReport { pub cores: u32, pub nets: u32, pub host: bool, pub errors: u32 }
pub fn teardown_all(base: &Path, workers: &Workers, nets: &Arc<NetCtx>) -> TeardownReport;
```

`teardown_all`, по порядку; каждая команда выполняется, даже если предыдущая завершилась ошибкой (ошибки считаются в `errors`):
1. ядра всех владельцев: остановка через `Workers` (если ядро в памяти контроллера) и по файлу pid (`kill_pidfile`); записи поколений ядер очищаются;
2. каждая сеть приложения из реестра аренд: команды снятия (те же, что у `net_revert`), аренда освобождается, каталог `etc_root/cm-<i>` удаляется, запись `net:<экземпляр>` очищается;
3. хост:
   - есть `host.json` → `host_firewall_commands(None)`, `host_destroy_commands` с внешними интерфейсами из файла, файл удаляется, запись `net:host` очищается; `host = true`;
   - файла нет, но наблюдаются `cmtunh`, `cmsink` или правила схемы CM → те же команды с внешними интерфейсами из `observe_uplinks`, плюс `ip rule del priority N` (и `-6`) для каждого наблюдаемого правила из набора ниже; `host = true`;
4. таблица `inet cm` заменяется пустой (`firewall::replace_commands(&[])`).

Правила схемы CM для шага 3: приоритет в `1900..=1907` или `1950..=1981` или равен `1990` с таблицей `main`; `2000` с таблицей `90`; `2001` с таблицей `91`. Другие правила не трогаются.

Повторный вызов на чистой системе: все счётчики 0, `host = false`, `errors = 0`.

**Кадр `teardown`:** принимается только от uid 0 сокета, иначе `denied`; uid проверяется раньше обращения к авторизатору. Выполняется под мьютексом сторожа: проход сторожа в это время не идёт. Экземпляр, занятый операцией клиента, ожидается до 5 с; не освободился — его шаги выполняются всё равно, а `errors` увеличивается на 1 (повторный вызов доведёт снятие). После кадра контроллер продолжает работать с пустым состоянием.

**Команда:**
- без `--offline`: кадр `teardown` контроллеру; печатает отчёт одной строкой JSON; коды 0, 3 (отказ), 4 (контроллер недоступен);
- `--offline`: `teardown_all` в этом процессе, без контроллера; исполнитель команд, источник наблюдений и каталог аренд создаются так же, как в `cm controller reconcile --boot`. Вне тестового режима требует uid 0 и неактивную службу `cm-controller.service` (иначе `[controller_running]`, код 3); `--base` допустим только в тестовом режиме — правила те же, что у `reconcile --boot`.

**`cm uninstall`** (оба режима), до выключения служб:
1. если сокет контроллера есть — `cm controller teardown`; неудача не прерывает удаление;
2. выключение служб — как сейчас;
3. `teardown_all` в этом процессе (то же, что `--offline`) — всегда.

Команды сети выполняются тем же исполнителем, что у контроллера. На хосте разработки команда не запускается: проверка — только в пространствах имён.

Контракты, которые проверит роль 2:
- **IT02 → X.Q0:** Кадр снятия доступен только суперпользователю и не пересекается с проходом сторожа.
  - кадр `teardown` от uid 1000 → `denied`; состояние не изменилось
  - кадр от uid 0 → ответ `ReplyData::Teardown`; следующий `overview` → host mode off, список туннелей без поколений
  - `cm controller teardown --offline` вне тестового режима не от uid 0 → код 4; при `--base` вне тестового режима → код 2
  - после снятия проход сторожа не выполняет ни одной команды (запись исполнителя пуста два прохода подряд)
- **IT03 → FIN.Z:** После снятия на стенде не остаётся ни одного объекта CM, и сеть хоста работает напрямую.
  - до снятия: туннель хоста включён, два туннеля приложений двух владельцев работают
  - `cm controller teardown` → код 0, отчёт `cores=3 nets=2 host=true errors=0`
  - после: `ip rule` без приоритетов 1000–2001; `ip link` без `cmtunh`, `cmsink`, `cmv*`, `cmtun*`; `ip netns list` без `cm-*`; `nft list tables` без `cm_host`; таблица `inet cm` без правил для интерфейсов
  - запрос хоста — HTTP 200 напрямую; процессов ядра нет
  - повтор команды → код 0, отчёт из нулей

### C.M2 — Считать нагрузку процессора и память ядра по `/proc`.

Файлы: `src/status/resources.rs`, `src/status.rs`. Зависит от: —.

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
- Процесса нет, файл не читается, формат не тот → `None`. Трафик здесь не считается и между туннелями не суммируется.

Контракты, которые проверит роль 2:
- **CM01 → C.M1:** Нагрузка считается по двум выборкам и не падает на необычных строках `/proc`.
  - строка stat с именем процесса `(a b) c)` → поля берутся после последней `)`; utime 150, stime 50 → 200
  - выборки (ticks 100, 0 мс) и (ticks 300, 2000 мс) при 100 тиках в секунду → cpu_pct 100
  - Δмс = 0 или ticks уменьшились → cpu_pct 0
  - statm `1000 2560 …` при странице 4 КБ → rss_kb 10240; `usage_text` → `cpu 0% · 10 МБ`
  - нет каталога процесса → `sample` возвращает None

### C.M1 — Собирать живое состояние туннелей по четырём осям из хранилища, контроллера и проверок.

Файлы: `src/status/live.rs`, `src/status.rs`, `src/status/tunnels.rs`, `src/env/record.rs`, `src/env/mod.rs`, `src/app/tunnel.rs`, `docs/design/cm-network-manager/I17-UI-CONTRACT.md`. Зависит от: C.O1, C.M2, C.E1.

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

`I17-UI-CONTRACT.md` — таблица «поле интерфейса → источник → когда unknown» по этой спецификации; предварительных полей нет.

Контракты, которые проверит роль 2:
- **CM02 → C.C1:** Каждая ось получает значение только из своего источника; без источника — unknown.
  - state up, записи региона нет, держателей нет → net verified, region unknown, state unknown, app unknown; actions [Down, Verify, Run]
  - запись региона Agreed DE с expected DE и поколением, равным поколению ядра → region verified, country DE; возраст = (now − at)/1000
  - та же запись с другим поколением → region unknown, country пусто
  - expected US при Agreed DE → region blocked, failure region-mismatch, condition blocked
  - state failed → net error, failure core-failed, actions [Up]; state blocked → failure core-restarting, actions [Down]
  - 2 держателя и запись сессии → state partial, app partial, sessions 2; ни в одном наборе входов `state` и `app` не равны verified
  - контроллер Unavailable → все оси unknown, возраста нет, actions пуст, строки туннелей хранилища присутствуют
  - экземпляр из обзора без туннеля в хранилище → строка с id "" после строк хранилища
- **CM03 → C.T1:** Клавиши страницы дают только разрешённые действия; остановка и режим хоста — через вопрос.
  - строка stopped: `u` → `Run(["tunnel","up","t-web"])`; `x` → `Message`, вопрос не открыт
  - строка up: `x` → открыт `Confirm::Down`; `Tab` → вопрос остаётся; `n` → `Message`, вопрос закрыт; `x`, `y` → `Run(["tunnel","down","t-web"])`
  - строка up со страной DE: `v` → `Run(["tunnel","verify","t-web","--country","DE"])`
  - хост off без профиля: `h` → `Message`; с профилем и `legacy_vpn` → `Message`, вопрос не открыт; с профилем без него → `HostOn`, `Enter` → `Run(["host","set","tunnel","--stored"])`
  - хост tunnel: `h`, `y` → `Run(["host","set","off"])`
  - `replace` с новым порядком строк сохраняет выбор по instance; `↓` на последней строке выбор не меняет
  - `q`, `Esc`, `Backspace` вне вопроса → `Back`; `Esc` в вопросе → отмена, не `Back`
- **CM04 → C.D1:** Очистка убирает каждый класс секретов, а выгрузка с остатком не пишется.
  - `scrub` строки с `/home/alice/x`, `alice`, `vless://id@host:443?x=1`, `password=abc`, uuid, 40-символьным токеном, `8.8.4.4` при home `/home/alice` и user `alice` → `find_secrets` результата пуст; исходных значений в результате нет
  - адреса `10.213.0.1`, `198.18.64.2`, `192.168.1.1`, `fe80::1` остаются как есть
  - `find_secrets` на исходной строке возвращает `url`, `key_value`, `uuid`, `long_token`, `public_ip`
  - `shape` документа с 3 узлами (2 vless, 1 trojan), 1 группой, 5 правилами → `{proxies:3, groups:1, rules:5, protocols:{trojan:1, vless:2}}`
  - свойство, 10 000 случаев: для случайной строки из алфавита секретов `find_secrets(scrub(s))` пуст
- **CM05 → C.A1:** Строки апплета повторяют оси живой модели и не складывают туннели в один статус.
  - два туннеля up и один failed → headline «туннели: 2 работают, 1 с ошибкой»
  - контроллер Unavailable → headline «контроллер недоступен»; пустой список → «туннелей нет»
  - строка туннеля: axes — четыре символа через пробел в порядке net, region, state, app; problem true при failed и при оси region blocked
  - первая строка — хост; при режиме off её axes содержит «выкл»

### C.C1 — Печатать живое состояние командой для человека и для программ.

Файлы: `src/app/tunnel.rs`, `src/i18n_table.rs`. Зависит от: C.M1.

```
cm tunnel list [--json]
```

Текст, первая строка — хост, далее по строке на туннель:

```
host mode=<m> state=<s> uplinks=<a,b|-> defined=<yes|no> legacy_vpn=<yes|no> core_pid=<n|->
id=<ID|-> instance=<имя> state=<s> net=<v> region=<v> data=<v> app=<v> holders=<n> restarts=<n> generation=<g|-> country=<CC|-> core_pid=<n|->
```

`data` — ось `state` (слово `state` занято состоянием туннеля). `--json` — `LiveView` одной строкой.

Код выхода: 0; 4 — контроллер недоступен (вывод всё равно печатается: туннели хранилища с unknown). Токены не переводятся.

Контракты, которые проверит роль 2:
- **CC01 → X.Q4:** Вывод `cm tunnel list` разбирается построчно и одинаков по содержанию с JSON.
  - до запуска: строка `host mode=off state=stopped uplinks=- defined=no legacy_vpn=no core_pid=-`; строка туннеля оканчивается на `state=stopped net=unknown region=unknown data=unknown app=unknown holders=0 restarts=0 generation=- country=- core_pid=-`
  - после `cm tunnel up t-web`: `state=up net=verified`, `generation=1`, `core_pid` — pid живого процесса ядра
  - после `cm tunnel verify t-web --country ZZ` с источниками стенда, отвечающими DE → `region=blocked`
  - `--json` разбирается в объект с ключами `at_unix_ms`, `controller`, `host`, `tunnels`; число туннелей равно числу строк текста минус одна
  - контроллер остановлен → код 4, строки туннелей хранилища напечатаны с unknown
- **CC02 → FIN.Z:** Состояние в выводе меняется вслед за событиями стенда не позже одного прохода сторожа.
  - `kill -9` ядра → не позже чем через 3 с `state=blocked net=blocked`; не позже чем через 8 с снова `state=up`, `restarts=1`
  - процесс в сети туннеля запущен → не позже чем через 3 с `holders=1 app=partial`; завершён → `holders=0 app=unknown`
  - `cm host set tunnel --stored` → строка хоста `mode=tunnel state=up`

### C.T1 — Показать туннели в TUI по осям и управлять ими без блокировки интерфейса.

Файлы: `src/status/page.rs`, `src/status.rs`, `src/tui.rs`, `src/tui/tunnels.rs`, `src/i18n_table.rs`. Зависит от: C.M1, I.S1.

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

Фоновый сбор — `Poller`: поток вызывает `gather` сразу и далее каждые `period_ms`, кладёт результат в канал ёмкостью 1 (`try_send`) и завершается, когда `Poller` уничтожен (ожидание периода — отрезками не длиннее 100 мс). `latest` забирает всё накопленное и возвращает последнее. `src/tui/tunnels.rs` создаёт `Poller::start(2000, …)` с `live::gather` при входе на страницу и на каждом такте интерфейса передаёт `latest()` в `Page::replace`. Нажатие клавиши никогда не ждёт сбора.

Контракты, которые проверит роль 2:
- **CT01 → FIN.Z:** Страница рисуется на узком и широком терминале, на двух языках, и нигде не объявляет общий «зелёный» статус.
  - размеры 60×20, 80×24, 120×40: отрисовка без паники; при 60 колонках оси — одним символом, при 120 — символ и слово
  - в тексте экрана есть четыре заголовка осей; нет слов «защищено», «безопасно», «анонимно», «ok» как итога строки
  - возраст 3601 с → в ячейке есть `⚠`; возраст 59 с → нет
  - языки ru и en: подписи значений осей разные, символы одинаковые; токены команд не переведены
  - открытый вопрос остановки содержит имя туннеля и слова о том, что VPN хоста не меняется
  - состояние `контроллер недоступен` — отдельная строка; строки туннелей при этом есть
  - `Poller` с функцией сбора, которая спит 3 с: 100 вызовов `latest` и 100 вызовов `Page::key` подряд завершаются быстрее 100 мс; первое значение приходит не позже чем через 4 с; после уничтожения `Poller` функция сбора больше не вызывается (счётчик вызовов не растёт 3 с)

### C.D1 — Выгружать диагностику без секретов, путей пользователя и аргументов программ.

Файлы: `src/diag/mod.rs`, `src/diag/export.rs`, `src/diag/redact.rs`, `src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: C.M1.

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

Коды: 0; 2 — использование или ошибка записи; 3 — экспорт не чист.

Контракты, которые проверит роль 2:
- **CD01 → X.Q0:** Выгрузка содержит состояние, достаточное для разбора, и ни одного значения из конфигов и запросов.
  - `cm diag export --out -` → код 0; JSON с ключами `version`, `live`, `instances`, `units`, `kernel`, `checks`
  - в выводе нет пароля, uuid и адреса узла стенда, нет пути домашнего каталога и имени пользователя
  - `instances[].shape.proxies` равно числу узлов источника; `generations` перечисляет файлы поколений
  - файл по умолчанию создан с правами 0600, путь напечатан
  - `kernel` содержит ключи для пяти запросов; при включённом туннеле хоста в `ip -j rule` есть приоритет 2000
- **CD02 → FIN.Z:** Выгрузка, в которой после очистки найден секрет, не записывается.
  - экспорт, в чьё поле `notes` тест подставил строку с uuid в обход `scrub` → запись отклонена, результат `export_not_clean`, файла нет
  - `cm diag export --out /нет/каталога/x` → код 2
  - в структуре `Export` нет полей с аргументами и окружением процессов (сериализованный документ не содержит ключей `argv`, `args`, `env`, `cmdline`)

### C.A1 — Показать туннели и нагрузку ядер в апплете.

Файлы: `src/status/panel.rs`, `src/status.rs`, `cosmic/src/tunnels.rs`, `cosmic/src/main.rs`, `cosmic/src/applet.rs`, `cosmic/src/panel.rs`, `cosmic/src/jobs.rs`, `cosmic/src/model.rs`. Зависит от: C.M1.

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

Действий, меняющих состояние, в апплете нет: управление — в TUI.

Контракты, которые проверит роль 2:
- **CA01 → FIN.Z:** Апплет собирается, не блокирует панель сбором и не содержит действий над туннелями.
  - `tests/check_audit.sh` собирает и тестирует `cosmic/` с `--locked --offline` (строка есть в файле); `cosmic/src/main.rs` содержит `mod tunnels`
  - в `cosmic/src/tunnels.rs` нет вызовов `cm host set`, `cm tunnel up|down`, `controller::client::call` с операциями, кроме `Overview` (проверка поиском по исходнику)
  - `live::gather` вызывается только из задания `Kind::Tunnels` (поиск по `cosmic/src`: одно место вызова)

### X.Q0 — Дать общий исполнитель шагов очереди хоста с планом, защитой и откатом.

Файлы: `tests/hostq/run.py`, `tests/hostq/lib.py`, `tests/hostq/steps/__init__.py`. Зависит от: I.H1, I.T1, C.D1.

```
python3 tests/hostq/run.py list
python3 tests/hostq/run.py plan ШАГ
python3 tests/hostq/run.py snapshot
python3 tests/hostq/run.py run ШАГ --yes [--no-snapshot --backup-dir КАТАЛОГ] [флаги шага]
python3 tests/hostq/run.py all --yes
```

```python
# tests/hostq/lib.py
@dataclass
class Meta:
    id: str                 # hq-<имя>
    title: str
    covers: list[str]       # подзадачи общего плана
    mandatory: bool         # без PASS приёмка не «готово»
    mutates: list[str]      # подмножество {"network", "services", "packages", "store"}; пусто — только чтение
    needs_root: bool
    requires: list[str]     # шаги, которые должны иметь PASS раньше
    timeout_s: int

@dataclass
class Check:
    name: str               # a-z0-9_
    status: str             # PASS | FAIL | UNKNOWN | SKIP
    detail: str = ""        # только знаки [a-z0-9_ =.,:%/<>-], до 200
    p0: bool = False        # утечка мимо туннеля, потеря сети, потеря данных
    stand_ref: str | None = None   # значение ребра (вид "IT03.3"), которое шаг повторяет на машине
    reason: str | None = None      # почему нет stand_ref

class Ctx:
    cm: str                                   # путь к установленному cm
    evidence: Path
    def sh(self, argv: list[str], root: bool = False, timeout: int = 30) -> Result   # Result: code, out, err
    def ask(self, question: str) -> str        # "y" | "n" | "skip"

# модуль шага: tests/hostq/steps/<имя>.py
META: Meta
def plan() -> list[str]                        # строки плана; команды, меняющие систему, начинаются с "! "
def preflight(ctx) -> list[Check]              # только чтение
def run(ctx) -> list[Check]
def rollback(ctx) -> list[Check]
```

Правила исполнителя:
1. `plan` и `list` ничего не выполняют, кроме чтения evidence.
2. `run` без `--yes` → `[needs_yes]`, код 3.
3. Шаг с непустым `mutates` требует записи о снимке за сегодня (`x-evidence/<дата>/session.json`, её создаёт `snapshot`: `snapper -c root create …`). Нет записи → `[no_snapshot]`, код 3. `--no-snapshot` допустим только вместе с `--backup-dir` существующего каталога.
4. Шаг из `requires` без PASS → `[requires <id>]`, код 3.
5. Порядок: `preflight` → (любой FAIL → шаг SKIP, `run` не вызывается) → `run` → `rollback`. `rollback` вызывается всегда, когда `mutates` не пуст: и при исключении, и при `SIGINT`/`SIGTERM`, и при превышении `timeout_s`.
6. Шаг с `"network"` в `mutates`: до `run` проверяется связь (`curl -m 5` к `https://www.gstatic.com/generate_204` и `https://cp.cloudflare.com/generate_204`, нужен ответ 204 с обоих). Нет связи → `[no_network]`, код 3. После `rollback` проверка повторяется. Не прошла → исполнитель выполняет `cm controller teardown` (через `sudo`), проверяет ещё раз, и при неудаче печатает блок «как вернуть сеть»: `sudo cm controller teardown --offline`, номер снимка и команду его отката. Проверка `network_restored` в этом случае — FAIL с `p0`.
7. Команды с `root=True` выполняются через `sudo`. Исполнитель сам от root не запускается (uid 0 → код 2).
8. Статус шага: FAIL — есть проверка FAIL; иначе UNKNOWN — есть UNKNOWN; иначе SKIP — все SKIP; иначе PASS. Код выхода: 0 — PASS или UNKNOWN; 1 — FAIL; 3 — шаг не начат.
9. `all` идёт по шагам в порядке `requires` и останавливается на первой проверке FAIL с `p0`.
10. `ask` при закрытом stdin возвращает `skip`. Ответ `skip` даёт проверку UNKNOWN, а не PASS.
11. Команды `cm` шаг выполняет от пользователя. Запрос пароля для операций сети и хоста показывает агент polkit сессии — это ожидаемо.
12. Шагу нужен источник в хранилище пользователя, а его нет → в плане и в `detail` — `no_sources`, в выводе — подсказка: `cm source import --url-stdin` (адрес подписки вводится в stdin, в аргументы и в evidence не попадает).

Evidence: `docs/design/cm-network-manager/x-evidence/<дата>/<шаг>.json` — `{step, rev, binary_sha256, started, finished, status, checks[], rollback}`. В `detail` попадают только слова из набора шага и числа. Вывод команд, адреса, имена узлов и источников не сохраняются; строка с недопустимыми знаками заменяется на `detail_rejected`.

Режимы для проверки ролью 2:
- `CM_HOSTQ_FAKE=<каталог>`: `ctx.sh` ничего не выполняет, берёт ответы по очереди из `<каталог>/answers.json` и пишет команды в `<каталог>/log`; `ask` берёт ответы оттуда же;
- `CM_HOSTQ_STAND=1`: шаг выполняется по-настоящему внутри пространства имён стенда; снимок и `sudo` не требуются, каталог evidence — из `CM_HOSTQ_EVIDENCE`.

Контракты, которые проверит роль 2:
- **XQ01 → X.Q1:** План ничего не меняет, а запуск невозможен без согласия и точки возврата.
  - `run.py plan hq-install` → код 0; журнал команд пуст; в выводе строки с `! ` перед `sudo`
  - `run.py run hq-install` без `--yes` → `[needs_yes]`, код 3; журнал пуст
  - `run.py run hq-install --yes` без записи о снимке → `[no_snapshot]`, код 3; с `--no-snapshot` без `--backup-dir` → код 2
  - `run.py run hq-host --yes` без PASS у `hq-install` → `[requires hq-install]`, код 3
  - `run.py list` → по строке на каждый шаг из `tests/hostq/steps/`; у шага без evidence статус `-`
  - запуск от uid 0 (подменённый `os.geteuid`) → код 2
- **XQ02 → X.Q2:** В evidence попадают только статусы и слова из набора шага.
  - проверка с `detail`, содержащим URL или заглавные буквы → в файле `detail_rejected`
  - файл evidence содержит ключи `step`, `rev`, `binary_sha256`, `started`, `finished`, `status`, `checks`, `rollback` и ни одной строки из подменных ответов команд
  - две фазы A с разным `reach` → проверка сравнения UNKNOWN, не FAIL и не PASS
  - фазы A равны, B отличается на стадии `tls` → FAIL, `detail` `stage=tls`
- **XQ03 → X.Q3:** Откат выполняется при любом исходе шага, а потеря сети после него — отдельная тревога.
  - `run` шага бросает исключение → `rollback` вызван, в evidence `rollback` заполнен, статус шага FAIL
  - подменные ответы: связь до шага есть, после отката нет → в журнале есть `cm controller teardown`; проверка `network_restored` FAIL с p0; в выводе блок «как вернуть сеть» с номером снимка
  - связи нет до шага → `[no_network]`, код 3; `run` не вызывался
  - стенд: `run.py run hq-host --yes` внутри пространства имён → статус PASS или UNKNOWN; проверки 3, 4, 7, 8, 9, 10, 14 — PASS; после шага `ip rule` без приоритетов 1900–2001
  - стенд: шаг прерван `SIGTERM` после включения туннеля → не позже чем через 20 с правил 1900–2001 нет
- **XQ04 → X.Q4:** Ручная проверка без ответа — UNKNOWN, и шаг убирает за собой всё, что создал.
  - `ask` при закрытом stdin → `skip`; все проверки разделов `account`, `applet`, `tui` — UNKNOWN
  - ответ `n` на вопрос → проверка FAIL
  - стенд: `run.py run hq-apps --yes --source s1` → проверки a04, a05, a06, a07, a11, a14, a16_native_runs, a20_export_clean, a20_no_verified_state_app — PASS
  - стенд: после шага в `cm app list` и `cm group list` нет идентификаторов `hq-*`; `cm tunnel list` без экземпляров `t-hq-*` с поколениями
- **XQ05 → X.Q5:** Статус шага выводится из проверок одним правилом, код выхода — из статуса.
  - проверки [PASS, UNKNOWN] → шаг UNKNOWN, код 0
  - проверки [PASS, FAIL, UNKNOWN] → шаг FAIL, код 1
  - проверки [SKIP, SKIP] → шаг SKIP; `preflight` с FAIL → `run` не вызывался, шаг SKIP, код 3
  - `run.py all --yes`: шаг с FAIL p0 → следующие шаги не запускались

### X.Q1 — Проверить установку, миграцию, хранилище, источники и пакетные сценарии на машине.

Файлы: `tests/hostq/steps/install.py`, `tests/hostq/steps/migrate.py`, `tests/hostq/steps/store.py`, `tests/hostq/steps/source.py`, `tests/hostq/steps/packages.py`. Зависит от: X.Q0.

| Шаг | Обязателен | Меняет | Что делает |
|---|---|---|---|
| `hq-install` | да | packages, services | `sh build.sh`; запись sha256 бинарника; `! sudo <бинарник> install`; проверки ниже |
| `hq-migrate` | да | store | `tests/check_i01_regressions.py --run --installed-binary <cm>`; на машине со следами UPD — `cm migration plan` и сверка, что `apply` не нужен или уже выполнен |
| `hq-store` | нет | — | `tests/check_i02_regressions.py --run --installed-binary <cm>` |
| `hq-source` | нет | — | `tests/check_i03_regressions.py --run …`; для каждого источника хранилища — `cm source show` и `cm source import --dry-run` |
| `hq-packages` | да | — | `tests/verify_real_packages.py`; состояние и проверка единиц systemd |

Проверки `hq-install`:
- `binary_matches_manifest` — sha256 установленного `cm` равен записанному;
- `controller_socket_enabled`, `reconcile_enabled` — `systemctl is-enabled` → `enabled`;
- `units_verify` — `systemd-analyze verify` для `cm-controller.socket`, `cm-controller.service`, `cm-reconcile.service` без ошибок;
- `polkit_actions` — в файле политики четыре действия контроллера, включая `io.github.cm.host`;
- `host_check_runs` — `cm host check` завершился с кодом 0 или 1 и напечатал 10 строк;
- `legacy_vpn_untouched` — состояние службы VPN хоста до и после установки одинаково (p0).

`rollback` у `hq-install` ничего не удаляет: печатает номер снимка. Удаление — решение пользователя.

`hq-source`: нет ни одного источника → все проверки UNKNOWN с `detail` `no_sources`. Проверка `ua_class_recorded` пишет класс User-Agent, а не строку. Адреса и имена источников в evidence не попадают.

`hq-packages`: `uninstall_leaves_no_net` выполняется только с флагом `--with-uninstall` (иначе UNKNOWN): `! sudo cm uninstall`, затем `ip rule`, `ip link`, `nft list tables` без объектов CM, затем `! sudo <бинарник> install`.

Контракты, которые проверит роль 2:
- **XQ06 → P.T1:** Шаги установки и миграции проверяют состояние, а не только код возврата команд.
  - `hq-install`: подменный `systemctl is-enabled cm-controller.socket` → `disabled` → проверка `controller_socket_enabled` FAIL
  - sha256 бинарника отличается от записанного → `binary_matches_manifest` FAIL
  - состояние службы VPN хоста до и после разное → `legacy_vpn_untouched` FAIL с p0
  - `hq-source` при пустом хранилище → все проверки UNKNOWN, `detail` `no_sources`
  - `hq-packages` без `--with-uninstall` → `uninstall_leaves_no_net` UNKNOWN; в журнале нет `cm uninstall`
  - план `hq-install`: `rollback` не содержит команд удаления

### X.Q2 — Сравнить прежний и новый путь на одном узле по схеме A/B/A.

Файлы: `tests/hostq/steps/parity.py`. Зависит от: X.Q0.

Шаг `hq-parity`, не обязателен, меняет `network` и `services`. Требует активную подписку прежнего VPN хоста и её же источник в хранилище (иначе все проверки UNKNOWN, `detail` — `no_legacy_subscription` или `no_store_source`).

Измерение одной фазы (`measure`), всё с предельным временем 10 с:
- `reach` — сколько из трёх адресов `…/generate_204` ответили 204;
- `dns` — разрешилось ли имя через системный резолвер;
- `country` — страна выхода по источникам `cm env probe` (две буквы или `unknown`);
- `stage` — при неудаче первая отказавшая стадия: `dns`, `tcp`, `tls`, `http`.

Три сравнения:

| Проверка | A | B | Закрывает |
|---|---|---|---|
| `store_vs_legacy` | прежний путь, подписка из прежнего файла | тот же VPN, источник из хранилища (`cm vpn use source:ID`) | `V.05` |
| `adapter_vs_legacy` | прежний запуск ядра | ядро через адаптер на том же конфиге | `X.08` |
| `flclash_vs_cm` | CM | FlClash на том же узле | `X.05` |

Порядок каждого: A → B → A. Результат:
- PASS — обе фазы A совпали между собой и B совпала с ними по `reach`, `dns`, `country`;
- FAIL — фазы A совпали, B отличается; `detail` — `stage=<стадия>`;
- UNKNOWN — фазы A не совпали между собой (среда нестабильна) либо FlClash не установлен (`flclash_absent`).

`rollback`: вернуть активную подписку и режим прежнего VPN, какими они были до шага; проверка `legacy_restored`.

Формулировки «VPN исправлен» в выводе и evidence нет: шаг сообщает только совпадение или первую отличающуюся стадию.

Контракты, которые проверит роль 2:
- **XQ07 → P.T1:** Сравнение путей называет первую отличающуюся стадию и не делает выводов сверх измеренного.
  - FlClash не найден → `flclash_vs_cm` UNKNOWN, `detail` `flclash_absent`; остальные сравнения выполняются
  - нет активной подписки прежнего VPN → все три проверки UNKNOWN, `detail` `no_legacy_subscription`; команд, меняющих систему, в журнале нет
  - после шага в журнале есть возврат прежней подписки и режима; проверка `legacy_restored` PASS
  - в выводе и evidence нет слов «исправлен», «fixed»

### X.Q3 — Проверить туннель хоста на настоящей машине и вернуть её в исходное состояние.

Файлы: `tests/hostq/steps/host.py`. Зависит от: X.Q0, R.Z, I.S1.

Шаг `hq-host`, обязателен, меняет `network` и `services`, требует `hq-install`. Нужен профиль хоста (`cm host define`); нет → SKIP, `detail` `no_host_profile`.

`preflight`: `cm host check`. `Block` только из-за `legacy_vpn_active` допустим: шаг сам остановит прежний VPN и запомнит это. Любой другой `Block` → SKIP с токеном в `detail`.

`run`, по порядку (в скобках — что повторяется со стенда):
1. `direct_baseline` — связь есть; страна выхода напрямую запомнена только как хеш.
2. Прежний VPN активен → `! sudo cm vpn stop`; `legacy_stopped`.
3. `! cm host set tunnel --stored` → `host_tunnel_on` (код 0, `cm host status` → `mode=tunnel`).
4. `host_http_via_tunnel` — три адреса отвечают 204 (стенд).
5. `exit_differs` — хеш страны или адреса выхода отличается от шага 1; совпал → UNKNOWN (узел в той же стране), не FAIL.
6. `dns_redirected` — после запроса к DNS из `resolv.conf` счётчик правила `dnat` таблицы `ip cm_host` вырос (стенд); `reason` при отсутствии метки — `needs_real_dns`.
7. `ipv6_blocked` — `curl -6 -m 5` к адресу с IPv6 завершается ошибкой (стенд). p0.
8. `core_kill_blocks` — `kill -9` ядра хоста; в следующие 2 с запрос завершается тайм-аутом, а не ответом напрямую (стенд). p0.
9. `core_restarts` — не позже чем через 15 с запрос снова проходит; `cm tunnel list` показывает у хоста `restarts` не меньше 1 (стенд).
10. `drift_repaired` — `! sudo ip rule del priority 2001`; не позже чем через 5 с правило снова есть (стенд, ребро `RW03`).
11. `app_tunnel_independent` — есть хотя бы один определённый туннель приложения: `cm tunnel up`, запрос из его сети проходит, число соединений ядра хоста за это время не выросло (стенд); нет туннелей → UNKNOWN.
12. `uplink_flap` — только с `--with-link-flap`: выключить и включить соединение; не позже чем через 30 с запрос проходит. Иначе UNKNOWN, `reason` `needs_real_uplink`.
13. `suspend_resume` — только с `--manual-suspend`: вопрос пользователю после пробуждения. Иначе UNKNOWN, `reason` `needs_suspend`.
14. `! cm host set off` → `host_off_clean`: правил 1900–2001, `cmtunh`, `cmsink`, таблицы `ip cm_host` нет; связь напрямую есть.

`rollback`: `cm host set off` (ошибка не прерывает), остановить поднятый в шаге 11 туннель, запустить прежний VPN, если шаг его останавливал; проверки `host_off`, `legacy_restored`.

У проверок 4, 7, 8, 9, 10, 11, 14 задан `stand_ref` — номер значения ребра уровня L2 (`AB-DAG.json` или `FIN-DAG.json`), которое проверяет то же на стенде: например `IS02.1` для проверки 4, `RW03.2` для 10, `IT03.3` для 14. Метки стенда совпадают с номерами значений, поэтому их можно назвать до того, как стенд написан.

Контракты, которые проверит роль 2:
- **XQ08 → P.T1:** Шаг туннеля хоста отличает утечку от недоступности и не объявляет успех по косвенным признакам.
  - подменный ответ: после `kill -9` ядра запрос вернул 204 → `core_kill_blocks` FAIL с p0
  - хеш выхода совпал с прямым → `exit_differs` UNKNOWN, не FAIL
  - `cm host check` с `Block` `resolved_unsupported` → шаг SKIP, `detail` содержит токен; команд, меняющих систему, нет
  - `Block` только `legacy_vpn_active` → шаг идёт; в журнале `cm vpn stop` раньше `cm host set tunnel`, в откате — запуск прежнего VPN
  - без `--with-link-flap` и `--manual-suspend` → `uplink_flap` и `suspend_resume` UNKNOWN с причинами `needs_real_uplink`, `needs_suspend`

### X.Q4 — Пройти критерии A01–A20 на настоящих приложениях и проверить интерфейс глазами.

Файлы: `tests/hostq/steps/apps.py`. Зависит от: X.Q0, R.Z, C.C1.

Шаг `hq-apps`, обязателен, меняет `network` и `store`, требует `hq-install`. Флаги: `--source ID` (источник для временных описаний; без него — первый источник хранилища; нет источников → SKIP `no_sources`), `--browser ПУТЬ`, `--appimage ПУТЬ`.

Шаг сам создаёт временные описания (`cm group define hq-g`, `cm app define hq-a …`, `hq-b …`) и в `rollback` останавливает туннели и удаляет описания.

```python
ACCEPTANCE: dict[str, dict]   # "A01" → {"stand": [метки], "host": [имена проверок]}
```

| Критерий | На машине (`host`) | На стенде (`stand`) |
|---|---|---|
| A01 хост напрямую, приложение в туннеле | `a01_app_exit_differs`: страна выхода приложения отличается от прямой (совпала → UNKNOWN) | Z01, Z03 |
| A02 хост и приложение в разных туннелях | `a02_independent` — из `hq-host`, проверка 11 | Z01 |
| A03 прокси хоста не влияет на приложение | — | H12, Z01 |
| A04 два приложения в общем туннеле, первое закрыто | `a04_second_keeps_route` | T12 |
| A05 два приложения в разных туннелях | `a05_two_tunnels` | Z02 |
| A06 VPN хоста выключен | `a06_app_survives_host_off` | Z01 |
| A07 `kill -9` ядра и контроллера | `a07_no_direct_after_kill`: после `kill -9` ядра (pid из `cm tunnel list`), а затем после `! sudo systemctl kill -s KILL cm-controller.service` запрос из сети приложения — тайм-аут. p0 | F03, F05 |
| A08 узел недоступен | — | F05 |
| A09 DNS, IPv6, UDP | `a09_dns_tcp`, `a09_aaaa_empty`, `a09_ipv6_blocked` | D04 |
| A10 смена сети, сон | из `hq-host`: `uplink_flap`, `suspend_resume` | — |
| A11 потомки и фоновые процессы | `a11_children_in_netns`: через 5 с после запуска у всех потомков запущенного процесса та же сеть | T01 |
| A12 уже работающий экземпляр | `a12_running_elsewhere`: при открытом вне туннеля браузере `cm app run` → код 5 (нужен `--browser`, иначе UNKNOWN) | T14 |
| A13 обновление источника при живой сессии | — | Z06 |
| A14 разные пояс и локаль при одном туннеле | `a14_two_zones`: два приложения группы печатают разный `date +%Z` | Z05 |
| A15 несогласие источников страны | `a15_verify_reports`: `cm tunnel verify` печатает один из исходов и не `verified` без `--country` | R03 |
| A16 произвольная программа и упаковка | `a16_native_runs`; `a16_appimage_partial` (нужен `--appimage`, иначе UNKNOWN); `a16_flatpak_refused`: описание с `flatpak` → `[unsupported_not_verified]` | U01, U02 |
| A17 чужой uid | — | Z02 |
| A18 обновление и откат | из `hq-migrate`, `hq-packages` | — |
| A19 старые правила, обновления системы | `a19_cm_check_ok`: `cm check` завершается как до установки | Z04 |
| A20 интерфейс и экспорт | `a20_export_clean`: `cm diag export --out -` → код 0; `a20_no_verified_state_app`: в `cm tunnel list` оси `data` и `app` не равны `verified` | CD01, CM02 |

Матрица исполнителя (`I13.T05.a`): для каждой проверенной упаковки в evidence — `packaging`, `executor_in_netns` (да/нет), `children_in_netns`.

Ручные разделы — вопросы `ask` (ответ `skip` → UNKNOWN):
- `account`: после входа в аккаунт в приложении с отделёнными данными и перезапуска — вход сохранился? в приложении с существующими данными — прежний профиль на месте?
- `applet`: в панели есть раздел «Туннели»; строки совпадают с `cm tunnel list`; при остановленном контроллере — «контроллер недоступен»; панель не замирает при открытии;
- `tui`: страница туннелей открывается; вопрос при остановке называет последствие; после `u` и `x` состояние меняется без перезапуска TUI; узкое окно (60 колонок) читается.

`rollback`: `cm tunnel down --force` для созданных туннелей, `cm app remove`, `cm group remove`; проверка `store_restored` — ревизия хранилища выросла, но описаний `hq-*` нет.

Контракты, которые проверит роль 2:
- **XQ09 → P.T1:** Каждый критерий приёмки имеет проверку и статус; непроверенное не выдаётся за проверенное.
  - без `--browser` → `a12_running_elsewhere` UNKNOWN; без `--appimage` → `a16_appimage_partial` UNKNOWN
  - подменный ответ: запрос из сети приложения после `kill -9` ядра вернул 204 → `a07_no_direct_after_kill` FAIL с p0
  - подменный ответ: `cm tunnel list` с `app=verified` → `a20_no_verified_state_app` FAIL
  - `a16_flatpak_refused` — PASS, только если вывод содержит `[unsupported_not_verified]` и код равен 2
  - в evidence есть раздел матрицы исполнителя со строкой на каждую проверенную упаковку

### X.Q5 — Измерить разброс качества на настоящем узле и вывести из него размер выборки и допуск.

Файлы: `tests/hostq/steps/pilot.py`. Зависит от: X.Q0.

Шаг `hq-pilot`, не обязателен, меняет `network`. Флаги: `--tunnel ID` (обязателен), `--tunnel-b ID` (второй туннель на другом узле), `--probes N` (по умолчанию 30).

Одна проба — запрос `…/generate_204` из сети туннеля (`cm app run ЭКЗЕМПЛЯР -- /usr/bin/curl -m 5 -o /dev/null -w '%{time_total}'`), пауза 1 с. Показатели серии: медиана, p90, доля потерь, коэффициент вариации.

- Без `--tunnel-b`: три серии на одном узле (A, A, A).
- С `--tunnel-b`: A, B, A.

Вывод в `i16-evidence/pilot/summary.json`:
- показатели каждой серии;
- `noise` — (наибольшая медиана серий A − наименьшая) / наименьшая;
- `min_margin` = 2 × `noise` — меньший разрыв между узлами считать шумом;
- `min_probes` — наименьшее N из {10, 20, 30, 60}, при котором расхождение медиан первых N проб серий A (та же формула) не больше 0.10; нет такого N → наибольшее из набора, не превышающее `--probes`;
- при `--tunnel-b`: `b_differs` — |медиана B − m| / m больше `min_margin`, где m — медиана из медиан серий A.

Проверки: `series_complete` (все пробы выполнены), `noise_measured`. Расхождение серий A больше 50 % → `noise_measured` UNKNOWN, `detail` `unstable`.

Шаг не меняет константы в коде: рекомендованные значения записываются в evidence. Автоматический выбор узла в выпуск не входит (решение D-Q1), поэтому шаг необязателен.

Контракты, которые проверит роль 2:
- **XQ10 → P.T1:** Размер выборки и допуск выводятся из измеренного разброса по записанной формуле.
  - серии A с медианами 100, 110, 104 мс → `noise` = 0.10; `min_margin` = 0.20
  - серия B с медианой 150 мс при этих A → `b_differs` true; с медианой 115 мс → false
  - серии, чьи медианы первых 10 проб расходятся на 0.30, а первых 20 — на 0.08 → `min_probes` = 20
  - расхождение серий A 0.6 → `noise_measured` UNKNOWN, `detail` `unstable`
  - шаг не пишет ни в один файл под `src/`

### P.D1 — Записать принятые решения: отпечаток, адаптер приложений, границы выпуска и порядок возврата.

Файлы: `docs/design/cm-network-manager/ADR-FINGERPRINT.md`, `docs/design/cm-network-manager/I15R-CATALOG.md`, `docs/design/cm-network-manager/ADR-APP-ADAPTER.md`, `docs/design/cm-network-manager/RELEASE-SCOPE.md`, `docs/design/cm-network-manager/QUESTIONS.md`. Зависит от: —.

Документы оформляют решения из раздела «Решения координатора» в `FIN-DAG.md`. Новых решений исполнитель не принимает. Каждый документ начинается со строки статуса: «Принято координатором 2026-10-11; пользователь может отменить».

**`I15R-CATALOG.md`** (`I15-R.T04.a`): каталог устройств не создаётся. Основание — замеры `I15R-LAB.md`: подмена движка и устройства увеличивает уникальность. Таблица «что предлагалось в Q26/Q27 → что принято → источник в лаборатории».

**`ADR-FINGERPRINT.md`** (`I15-R.T05.a`): таблица свойств. Для каждого — статус `go`, `conditional` или `no-go`, механизм, evidence, признак регресса. Строки: UA и Client Hints (no-go: не подменяются), часовой пояс, язык и локаль, `Accept-Language`, Canvas, WebGL, шрифты, WebRTC, экран, история личности (go: 5 записей, смена только командой). Статусы берутся из `ADR-BROWSER-IDENTITY.md` и `bi-evidence`; свойства без замера — `conditional` с указанием, какой замер нужен.

**`ADR-APP-ADAPTER.md`** (`I15.T01.a`): три состояния данных (`existing`, `isolated`, `fresh`), что CM создаёт и чего никогда не удаляет (домашний каталог и существующие профили — никогда), политика сокетов, что остаётся за программой. Источник — `src/env/private.rs` и вершины `I15.E1`, `I15.E2` пакета AB.

**`RELEASE-SCOPE.md`** (`I18.T04.a`), разделы в этом порядке:
1. «Входит»: ядро mihomo закреплённой версии; туннели приложений; группы; туннель хоста командой `cm host`; регион (пояс, локаль); личность браузера (стратегии `local` и `crowd`); TUI, апплет, экспорт.
2. «Не входит»: каждая строка — что, почему, какая подзадача общего плана. Обязательные строки: Xray как ядро туннеля приложения (`I05.T02.b`, `I05.T05.a`, `I10.T02.b`); IPv6 через туннель; Flatpak и Snap; systemd-resolved для туннеля хоста; автоматический выбор узла по качеству; замена прежнего VPN хоста по умолчанию; подтверждённые оси STATE и APP.
3. «Ограничения»: режим хоста — не граница изоляции; AppImage — частично; шина сессии при общих сокетах; каталоги данных сессий `fresh` и `isolated` CM не удаляет — их убирает пользователь; хранилище туннелей — у каждого пользователя своё, подписку прежнего VPN в него нужно импортировать отдельно.
4. «Как вернуть»: три порядка действий — выключить туннель хоста (`cm host set off`); снять всё (`sudo cm controller teardown`, при неработающем контроллере — с `--offline`); откатить снимок. Для каждого — когда применять и как проверить результат.
5. «Не проверено на машине»: заполняется программой приёмки (`P.T1`) из шагов со статусом UNKNOWN; в исходном тексте — метка-заполнитель `<!-- accept:unknown -->`.

**`QUESTIONS.md`**: в разделах Q26 и Q27 первая строка заменяется на «Решено координатором 2026-10-11: …» со ссылкой на `I15R-CATALOG.md`; остальной текст разделов не меняется.

Контракты, которые проверит роль 2:
- **PD01 → P.T1:** Каждое решение записано со статусом, а каждая подзадача вне выпуска названа поимённо.
  - `ADR-FINGERPRINT.md`: таблица не короче 10 строк; в каждой строке статус из набора go, conditional, no-go; строка UA — no-go
  - `RELEASE-SCOPE.md`: пять разделов в заданном порядке; в разделе «Не входит» есть `I05.T02.b`, `I05.T05.a`, `I10.T02.b`; есть обе метки `accept:unknown`
  - раздел «Как вернуть» содержит `cm host set off`, `cm controller teardown`, `--offline` и слово «снимок»
  - `QUESTIONS.md`: разделы Q26 и Q27 начинаются словами «Решено координатором 2026-10-11»; число строк файла изменилось не больше чем на 4
  - ни в одном из четырёх новых документов нет слов «анонимно», «гарантирует анонимность»

### P.T1 — Выводить трассировку требований, отчёт по машине и итог приёмки из evidence.

Файлы: `docs/design/cm-network-manager/tools/accept.py`, `docs/design/cm-network-manager/I18-TRACE.md`, `docs/design/cm-network-manager/X-RUNTIME-REPORT.md`, `docs/design/cm-network-manager/I18-ACCEPTANCE.md`. Зависит от: P.D1, X.Q1, X.Q2, X.Q3, X.Q4, X.Q5.

```
python3 docs/design/cm-network-manager/tools/accept.py [--date ГГГГ-ММ-ДД] [--check]
```

Входы (только чтение):
- `REMAINING-DAG-PLAN.json` — подзадачи и список закрытых;
- все `*-DAG.json` — вершины, рёбра, `covers`;
- `fin-evidence/<дата>/summary.json` — gate, результат `test_lint.py`, рёбра PASS/FAIL, дефекты;
- `x-evidence/<дата>/*.json` — шаги очереди хоста (берётся последний файл каждого шага);
- `tests/fixtures/ab/blocked.json`, `tests/fixtures/fin/blocked.json`.

```python
W_TO_MILESTONES = {
    "W01": ["I04", "I05"], "W02": ["I16"], "W03": ["I11", "I13"], "W04": ["I12"], "W05": ["I03", "I12"],
    "W06": ["I07"], "W07": ["I08", "I09"], "W08": ["I07", "I08"], "W09": ["I14", "BI"], "W10": ["I15", "BI"], "W11": ["I17"],
}
MANDATORY_STEPS = ["hq-install", "hq-migrate", "hq-packages", "hq-host", "hq-apps"]
DECIDED = {"V.02": "D-V1", "I08.T01.a": "D-V1", "H.10": "D-C1", "I05.T02.a": "D-X1"}   # закрыты решением координатора

def task_status(task_id) -> str      # closed | covered | covered_unverified | open | out_of_scope
def verdict(inputs) -> tuple[str, list[Blocker]]   # "ready" | "blocked"
@dataclass
class Blocker: what: str; kind: str; next_action: str; who: str   # who: coder | tester | user | coordinator
```

`task_status`:
- `closed` — в списке закрытых общего плана или в `DECIDED`;
- `out_of_scope` — названа в разделе «Не входит» `RELEASE-SCOPE.md`;
- `covered` — есть в `covers` хотя бы одной вершины, и у каждой такой вершины все исходящие рёбра PASS; если вершина — шаг очереди хоста (направление X, кроме `X.Q0`), её шаги ещё и имеют PASS;
- `covered_unverified` — то же, но шаг очереди хоста не запускался или дал UNKNOWN. На итог влияет только через условие 5 (обязательные шаги);
- иначе `open`.

`verdict` — `ready`, только если выполнено всё:
1. gate: EXIT 0, строк SKIPPED нет;
2. `test_lint.py --dags AB,FIN`: EXIT 0;
3. дефектов с `severity` P0 и P1 нет;
4. все рёбра AB и FIN — PASS; заблокированные значения — в пределах лимита, и шаг очереди хоста, названный у каждого, имеет PASS;
5. каждый шаг из `MANDATORY_STEPS` — PASS; ни в одном шаге нет проверки FAIL с `p0`;
6. подзадач со статусом `open` нет.

Иначе `blocked`, и на каждое невыполненное условие — `Blocker`:

| Причина | `who` | `next_action` |
|---|---|---|
| красный gate, дефект P0/P1 | coder | исправить по записи дефекта |
| ребро не PASS, замечание `test_lint` | tester | дописать проверку значения |
| шаг очереди хоста не запускался, SKIP или UNKNOWN у обязательного | user | `python3 tests/hostq/run.py run <шаг> --yes` |
| шаг FAIL | coordinator | разбор evidence шага |
| подзадача `open` | coordinator | назначить вершину или вывести из выпуска |

Выходы:
- `I18-TRACE.md` — по строке на W01–W11: этапы, подзадачи по статусам, рёбра, шаги очереди хоста; отдельная таблица открытых P0;
- `X-RUNTIME-REPORT.md` — по строке на шаг: статус, дата, число проверок по статусам, проверки UNKNOWN с причиной; статус не повышается (UNKNOWN не становится PASS);
- `I18-ACCEPTANCE.md` — `ready` или `blocked`, условия 1–6 с отметками, таблица `Blocker`. Последний раздел — «Решение пользователя: выпуск — отдельным поручением»; программа его не заполняет;
- в `RELEASE-SCOPE.md` текст между метками `<!-- accept:unknown -->` и `<!-- /accept:unknown -->` заменяется списком проверок UNKNOWN.

`--check` ничего не пишет: печатает итог и возвращает 0 при `ready`, 1 при `blocked`. Программа не запускает ни gate, ни шаги: она только читает их результаты.

Контракты, которые проверит роль 2:
- **PT01 → FIN.Z:** Итог приёмки — функция от evidence: одно невыполненное условие даёт «blocked» и называет, кто действует дальше.
  - набор «всё выполнено» → `--check` код 0, `ready`
  - тот же набор без evidence шага `hq-host` → `blocked`; один `Blocker` с `who` user и командой `run.py run hq-host --yes`
  - шаг `hq-apps` со статусом UNKNOWN → `blocked` (обязательный шаг); шаг `hq-pilot` UNKNOWN → на итог не влияет, проверка попадает в список «Не проверено на машине»
  - дефект P1 в summary → `blocked`, `who` coder; замечание `test_lint` → `who` tester
  - подзадача без вершины и вне «Не входит» → `open`, `blocked`, `who` coordinator; `V.02` и `H.10` → `closed` (решения D-V1, D-C1)
  - подзадача `X.05` при шаге `hq-parity` со статусом UNKNOWN → `covered_unverified`; итог от неё не зависит
  - `X-RUNTIME-REPORT.md`: шаг UNKNOWN остаётся UNKNOWN; число строк равно числу шагов
  - `I18-TRACE.md`: 11 строк W01–W11; у W01 в подзадачах вне выпуска есть `I05.T05.a`
  - `--check` не создаёт и не меняет файлов (времена изменения прежние)

## Сдача

Отчёт:
- по каждой вершине: сделано или открыто с причиной;
- `git diff --stat` по существующим файлам;
- EXIT и счётчики gate;
- список мест, где спецификация допускала толкование, и выбранное толкование.

## Что закрывает пакет в общем плане

| Вершина | Закрывает подзадачи |
|---|---|
| `R.W1` | I12.T05.a |
| `R.W2` | I09.T03.a |
| `R.W3` | I07.T04.a, I09.T04.a |
| `R.T1` | I12.T02.a |
| `R.A9` | I11.T04.a, I13.T04.a, I14.T03.a |
| `R.Z` | I07.T05.a, I08.T05.a, I09.T05.a, I10.T05.a, I11.T05.a, I12.T05.a, I13.T02.a, I14.T04.a, I16.T05.a |
| `I.H1` | I07.T01.b |
| `I.S1` | I07.T02.a |
| `I.T1` | I09.T02.a |
| `C.O1` | I17.T01.a |
| `C.S1` | I12.T01.a |
| `C.E1` | I12.T01.a, I11.T01.a |
| `C.M2` | I17.T04.a |
| `C.M1` | I17.T01.a, I17.T02.a |
| `C.C1` | I17.T02.a |
| `C.T1` | I17.T02.a, I17.T05.a |
| `C.D1` | I17.T03.a |
| `C.A1` | I17.T04.a |
| `X.Q0` | X.07 |
| `X.Q1` | X.02, X.03, X.04, X.06 |
| `X.Q2` | X.05, X.08, V.05 |
| `X.Q3` | I07.T05.a, I09.T05.a, I10.T05.a |
| `X.Q4` | I13.T05.a, I15.T05.a, I17.T04.a, I17.T05.a, I18.T02.a, I18.T03.a |
| `X.Q5` | I16.T03.a |
| `P.D1` | I15-R.T04.a, I15-R.T05.a, I15.T01.a, I18.T04.a, I05.T02.b, I05.T05.a, I10.T02.b |
| `P.T1` | X.07, I18.T01.a, I18.T05.a |
| `FIN.Z` | I17.T05.a, I18.T03.a |

Подзадача закрывается, когда приняты все её вершины; подзадачи уровня L4 — когда пользователь прошёл соответствующий шаг очереди хоста.

Открытые подзадачи общего плана, которых нет ни в AB, ни в FIN:

| Подзадача | Что с ней |
|---|---|
| `H.10` | решение D-C1 |
| `V.02` | решение D-V1 |
| `I05.T02.a` | решение D-X1 |
| `I08.T01.a` | решение D-V1 |

