"""FIN, часть 1: доводка пакетов A и B (направление R) и возврат системы в исходное состояние (направление I).

Основание — ревью 2026-10-11: REVIEW-2026-10-11-AB.md.
"""

import json
from pathlib import Path

AB = json.loads((Path(__file__).resolve().parent.parent / "AB-DAG.json").read_text())


def ab_spec(vid):
    return next(v["spec"] for v in AB["vertices"] if v["id"] == vid)


DIRECTIONS = [
    ("R", "Доводка пакетов A и B", "Закрыть дефекты ревью и заново принять все 55 рёбер AB с проверкой каждого значения."),
    ("I", "Установка и возврат", "Проверка готовности машины, туннель хоста из хранилища, гарантированное снятие всего, что поставил CM."),
]

V = [
    dict(id="R.P1", direction="R", title="Отклонять политику туннеля, которая не является объектом JSON.", size="S", level="L1",
         deps=[], files=["src/net/policy.rs"], covers=[], external=[],
         spec="""\
Дефект D01: `parse(b"[]")` возвращает политику по умолчанию. Причина — serde разбирает структуру и из массива.

```rust
pub fn parse(bytes: &[u8]) -> Result<TunnelPolicy, NetError>;
```

Порядок:
1. `bytes.len() > MAX_POLICY_BYTES` → `NetError::Unsupported`;
2. разбор в `serde_json::Value`; ошибка или не объект → `NetError::Unsupported`;
3. разбор объекта в `TunnelPolicy` (как сейчас: неизвестное поле — отказ, отсутствующее — значение по умолчанию).

Сигнатура и остальные типы файла не меняются."""),

    dict(id="R.W1", direction="R", title="Давать каждой автоматической остановке туннеля свой ключ операции.", size="S", level="L1",
         deps=[], files=["src/controller/watch.rs"], covers=["I12.T05.a"], external=[],
         spec="""\
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
- Обе операции прошли → как сейчас: `Stopped`, признаки сброшены."""),

    dict(id="R.W2", direction="R", title="Повторять перезапуск ядра по расписанию, пока не исчерпан лимит.", size="M", level="L1",
         deps=[], files=["src/controller/watch.rs", "src/controller/core_ops.rs"], covers=["I09.T03.a"], external=[],
         spec="""\
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

Окно `WINDOW_MS` и лимиты в `failure.rs` не меняются."""),

    dict(id="R.W3", direction="R", title="Сверять и чинить сеть туннеля хоста на каждом проходе сторожа.", size="M", level="L1",
         deps=[], files=["src/net/host_audit.rs", "src/net/mod.rs", "src/controller/watch.rs", "src/controller/boot.rs"],
         covers=["I07.T04.a", "I09.T04.a"], external=[],
         spec="""\
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

Весь шаг хоста в стороже (смена внешних интерфейсов, сверка, починка) идёт под блокировкой экземпляра `host` его владельца (`workers.try_begin`). Экземпляр занят операцией клиента (`host_apply`, `host_revert`) → шаг пропускается целиком. Без этого сторож восстанавливает правила, которые в этот момент снимает `cm host set off`."""),

    dict(id="R.T1", direction="R", title="Снимать туннель по фактическим поколениям ядра и сети.", size="M", level="L2",
         deps=["C.O1"], files=["src/app/tunnel.rs"], covers=["I12.T02.a"], external=[],
         spec="""\
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

Остальное в `up`, `status`, `verify` не меняется (формат файла региона — вершина `C.M1`)."""),

    dict(id="R.A9", direction="R", title="Реализовать `cm app run` с проверками упаковки, региона, чужого экземпляра и передачи управления.", size="L", level="L2",
         deps=[], files=["src/app/cli.rs", "src/i18n_table.rs"], covers=["I11.T04.a", "I13.T04.a", "I14.T03.a"], external=[],
         spec="Вершина `I11.A9` пакета AB не выполнена: в `src/app/cli.rs` добавлено только поле `session: None`. Модули, на которые она опирается, готовы: `app::assign`, `app::packaging`, `app::instance`, `app::env`, `env::private`, `env::apply::check_preset`.\n\nСпецификация — прежняя, дословно:\n\n"
              + ab_spec("I11.A9")
              + "\n\nДополнения:\n- путь сокета — `client::socket_path()` (как в `cm tunnel`), а не разбор переменной в этом файле;\n- упаковка проверяется раньше любого обращения к контроллеру: отказ по упаковке даёт код 2 и при недоступном контроллере;\n- сообщение справки `cm app` дополняется новыми флагами;\n- уборка: перед созданием файла сессии, если `app_list` показал пустой список процессов, удаляются все файлы `config/session-*.json` этого экземпляра (живых сессий нет). Каталоги данных сессий команда не удаляет никогда."),

    dict(id="R.Z", direction="R", title="Заново принять пакеты A и B: проверка у каждого значения каждого ребра.", size="L", level="L2",
         deps=["R.P1", "R.W1", "R.W2", "R.W3", "R.T1", "R.A9"],
         files=["tests/audit_ab_*.rs", "tests/ab_stand.py", "tests/fixtures/ab/blocked.json", "docs/design/cm-network-manager/fin-evidence/<дата>/ab.json"],
         covers=["I07.T05.a", "I08.T05.a", "I09.T05.a", "I10.T05.a", "I11.T05.a", "I12.T05.a", "I13.T02.a", "I14.T04.a", "I16.T05.a"], external=[],
         spec="Вершина роли 2, кода продукта нет. Ревью показало: стенд и часть тестов AB проверяют не значения рёбер, а константы и заведомо истинные условия. Работа — привести проверки всех 55 рёбер `AB-DAG.json` к правилам раздела «Рамки прохождения» и пройти `tools/test_lint.py --dags AB`."),

    dict(id="I.H1", direction="I", title="Показать без изменений системы, можно ли на этой машине включить туннель хоста.", size="S", level="L1",
         deps=[], files=["src/net/cli.rs", "src/net/host.rs", "src/i18n_table.rs"], covers=["I07.T01.b"], external=[],
         spec="""\
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

Решение о включении по-прежнему принимает контроллер (`host::preflight`). Команда ничего не обещает сверх него: это подсказка до запроса пароля."""),

    dict(id="I.S1", direction="I", title="Включать туннель хоста из хранилища, без файла конфига.", size="S", level="L2",
         deps=["C.E1"], files=["src/net/cli.rs", "src/app/assign.rs", "src/i18n_table.rs"], covers=["I07.T02.a"], external=[],
         spec="""\
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
- Переходы, откат и коды выхода `cm host set` не меняются."""),

    dict(id="I.T1", direction="I", title="Снимать всё, что поставил CM, одной командой и при удалении пакета.", size="M", level="L2",
         deps=["R.W3"], files=["src/controller/teardown.rs", "src/controller/mod.rs", "src/controller/protocol.rs", "src/controller/dispatch.rs",
                                "src/controller/server.rs", "src/main.rs"], covers=["I09.T02.a"], external=[],
         spec="""\
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

Команды сети выполняются тем же исполнителем, что у контроллера. На хосте разработки команда не запускается: проверка — только в пространствах имён."""),
]

E = [
    dict(id="RP01", source="R.P1", target="R.Z", level="L1",
         contract="Политика читается только из объекта JSON ограниченного размера.",
         check="`tests/audit_ab_policy.rs`.",
         values=["`parse(b\"[]\")`, `parse(b\"null\")`, `parse(b\"1\")`, `parse(b\"\\\"x\\\"\")` → `Err(NetError::Unsupported)`",
                 "`parse(b\"{}\")` → политика по умолчанию: ipv6 Block, fake_ip Forbidden, lifetime Pinned, failure Block",
                 "объект длиной `MAX_POLICY_BYTES + 1` (допустимый JSON, дополненный пробелами) → `Err`",
                 "`{\"lifetime\":\"refcount\",\"extra\":1}` → `Err`: неизвестное поле"]),
    dict(id="RW01", source="R.W1", target="R.Z", level="L1",
         contract="Автоматическая остановка срабатывает каждый раз, а не только первый.",
         check="`tests/audit_ab_watch.rs`: подменные ядро, `NetExec`, `NetQuery`; время задаёт тест; держатели — дерево-фикстура `/proc` и каталог `netns` в тестовом режиме.",
         values=["политика refcount: запуск, держатели ушли, прошло 5000 мс → ядро остановлено, в записи исполнителя есть команды снятия сети, состояние stopped",
                 "тот же экземпляр поднят снова, держатели ушли, прошло 5000 мс → остановлен второй раз; в журнале владельца две завершённые операции с разными ключами вида `auto-stop-<число>-<число>`",
                 "`auto_txn(\"revert\", i64::MAX, u32::MAX)` — длина 42, только `a-z0-9-`; `auto_txn(\"stop\", -5, 1)` = `auto-stop-0-1`",
                 "подменное ядро отвечает ошибкой на остановку → команд снятия сети в записи нет, состояние blocked",
                 "политика pinned при тех же событиях → ни одной команды, состояние up"]),
    dict(id="RW02", source="R.W2", target="R.Z", level="L1",
         contract="Перезапуски ядра идут по расписанию до лимита, затем туннель объявляется отказавшим.",
         check="`tests/audit_ab_watch.rs`: подменное ядро считает запуски; проходы — вызовы `tick` с заданным временем.",
         values=["ядро не запускается, проходы каждые 2000 мс от 0 до 160000: попытки запуска ровно на проходах 2000, 4000, 10000, 40000, 160000 — всего 5",
                 "после прохода 160000: `tunnel_state` → state failed, restarts 5; на проходах 162000 и 164000 попыток нет",
                 "между проходами 0 и 160000 состояние blocked; команд `ip` и `nft` от шага «ядра» в записи исполнителя нет",
                 "ядро запускается с третьей попытки (проход 10000) → на проходе 12000 состояние up, restarts 3",
                 "`worker_start` после failed → состояние starting, restarts 0"]),
    dict(id="RW03", source="R.W3", target="R.Z", level="L1",
         contract="Каждое недостающее звено сети хоста находится и чинится своими командами, приёмник — раньше туннеля.",
         check="`tests/audit_ab_host.rs` (чистые функции), `tests/audit_ab_watch.rs` (проход сторожа с подменным `NetQuery`).",
         values=["полное состояние при одном внешнем интерфейсе с одним адресом → `host_audit` пуст",
                 "нет правила IPv4 2001 → `[MissingRule{v6:false, priority:2001}]`; починка — одна команда `ip rule add priority 2001 lookup 91`",
                 "нет правила IPv6 2001 → `[MissingRule{v6:true, priority:2001}]`; починка — `ip -6 rule add priority 2001 lookup 91`",
                 "нет `cmtunh` → `[MissingTun, MissingTunRoute]`; починка — четыре команды: tuntap add, addr add, link set mtu up, `ip route replace default dev cmtunh table 90`",
                 "нет `cmsink` и правила 2000 → в командах починки создание приёмника стоит раньше `ip rule add priority 2000 lookup 90`",
                 "`firewall_present = false` → `[MissingFirewall]`; починка — одна команда `nft -f -` с текстом `render_host_table`",
                 "проход сторожа при `MissingTun`: после починки ядру `host` назначен перезапуск на этот же момент времени",
                 "режим хоста `proxy` или `host.json` нет → запросов наблюдения хоста и команд нет",
                 "экземпляр `host` занят операцией клиента → шаг хоста пропущен: запросов наблюдения и команд нет, смена внешних интерфейсов тоже не выполняется"]),
    dict(id="CO01", source="C.O1", target="R.T1", level="L1",
         contract="Обзор отдаёт клиенту поколения ядра и сети отдельно.",
         check="`tests/audit_fin_overview.rs`.",
         values=["после `net_apply` с поколением 1 и `worker_start` с поколением 2: строка экземпляра — worker_generation 2, net_generation 1",
                 "после `worker_stop`: worker_generation пусто, net_generation 1",
                 "после `net_revert`: каталог экземпляра остаётся, оба поколения пусты, netns пусто"]),
    dict(id="RT01", source="R.T1", target="R.Z", level="L2",
         contract="`cm tunnel down` всегда снимает то, что поднял `up`, и повторяется без ошибки.",
         check="`tests/ab_stand.py`, раздел `tunnel`.",
         values=["`up t1` → `down t1 --keep-net` → `up t1` → `down t1`: все четыре — код 0; после последней нет `cm-<i>` в `ip netns list`, нет правила `iif cmv<i>h`, нет ядра",
                 "`down t1` второй раз → код 0, в журнале контроллера новых операций нет",
                 "`down t1` при живом процессе в сети → `[in_use]`, код 5; с `--force` → код 0",
                 "`kill -9` ядра, затем `up t1` до восстановления → `[busy]`, код 3",
                 "`up` для экземпляра, у которого сеть создана другим клиентом того же владельца (`net_generation` задано) → код 0; для экземпляра без сети при ответе conflict → `[conflict]`, код 3"]),
    dict(id="RA01", source="R.A9", target="R.Z", level="L2",
         contract="`cm app run` выполняет проверки по порядку и останавливается на первом отказе.",
         check="`tests/audit_ab_cli.rs` (отказы до контроллера), `tests/ab_stand.py`, разделы `apps` и `private`.",
         values=["`cm app run t1 -- /usr/bin/systemd-run --user x` без контроллера → `[unsupported_executor_outside]`, код 2 (не 4)",
                 "описание с Flatpak → `[unsupported_not_verified]`, код 2",
                 "пояс региона отсутствует в `/usr/share/zoneinfo` → `[zone_missing]`, код 2; локаль не установлена → `[locale_not_installed]`, код 2, с `--allow-missing-locale` запуск идёт",
                 "программа уже работает вне сети туннеля → `[running_elsewhere]`, код 5; с `--allow-running` → код 0",
                 "программа-фикстура `tests/brokers/single_instance.py`, завершившаяся сразу, без новых процессов в сети → `[handed_off]`, код 6",
                 "обычный запуск → в stdout pid, код 0; через 1 с процесс с этим pid в сети туннеля (`readlink /proc/<pid>/ns/net` совпадает с netns туннеля)",
                 "`--data isolated --ipc private` → файл `config/session-s<12 шестнадцатеричных>.json` с правами 0600; у процесса `XDG_RUNTIME_DIR` — каталог сессии, `DBUS_SESSION_BUS_ADDRESS` не задан"]),
    dict(id="RZ01", source="R.Z", target="FIN.Z", level="L2",
         contract="У каждого значения каждого ребра AB есть проверка, и ни одна проверка не пройдёт на сломанном коде по построению.",
         check="`tools/test_lint.py --dags AB`; `tests/check_audit.sh`.",
         values=["`python3 docs/design/cm-network-manager/tools/test_lint.py --dags AB` → EXIT 0; «без проверки 0», «замечаний 0»",
                 "в `tests/audit_ab_*.rs` нет `#[ignore]`: D01, U02 сняты после `R.P1` и `R.A9`; F01 и F02 — ошибка фикстуры (каталог `leases` не создан до `LeaseRegistry::create`), исправляется в тесте",
                 "в `tests/ab_stand.py` нет циклов ожидания без `sleep`: функция `app()` опрашивает файл каждые 100 мс до срока",
                 "стенд — 5 прогонов подряд, каждый EXIT 0; gate — 3 прогона подряд, каждый EXIT 0 без строк SKIPPED",
                 "`tests/fixtures/ab/blocked.json` — не больше 12 значений, все уровня L2, у каждого названы причина и шаг очереди хоста"]),
    dict(id="RZ02", source="R.Z", target="X.Q3", level="L2",
         contract="Шаг очереди хоста повторяет на машине то, что уже прошло на стенде, а не проверяет впервые.",
         check="`tests/ab_stand.py`, раздел `host`; сверка списков в `tests/audit_fin_hostq.rs`.",
         values=["каждый `stand_ref` проверок шага `hq-host` — номер значения ребра уровня L2 из `AB-DAG.json` или `FIN-DAG.json` (вид `IT03.3`), и проверка с этой меткой есть в `tests/ab_stand.py` или `tests/fin_stand.py`",
                 "проверок шага `hq-host` без `stand_ref` не больше трёх, и у каждой указана причина (`needs_real_uplink`, `needs_real_dns`, `needs_suspend`)"]),
    dict(id="RZ03", source="R.Z", target="X.Q4", level="L2",
         contract="Каждый критерий A01–A20 привязан либо к проверке стенда, либо к проверке шага на машине.",
         check="`tests/audit_fin_hostq.rs`: таблица `ACCEPTANCE` шага `hq-apps`.",
         values=["в таблице 20 строк A01–A20; у каждой — `stand` (метки рёбер) или `host` (имена проверок шага), либо оба",
                 "строк только со `stand` не больше восьми; A01, A02, A06, A07, A11, A12, A16, A20 имеют `host`",
                 "каждое ребро из `stand` есть в `AB-DAG.json` или `FIN-DAG.json` и имеет хотя бы одну проверку с меткой в `tests/ab_stand.py` или `tests/fin_stand.py`"]),
    dict(id="IH01", source="I.H1", target="X.Q0", level="L1",
         contract="Проверка готовности хоста — чистая функция от наблюдений с фиксированным порядком строк.",
         check="`tests/audit_fin_install.rs`.",
         values=["чистая машина (nft есть, один внешний интерфейс, сокет есть, режим off) → 10 строк, все `ok`, код 0",
                 "интерфейс `cm-vpn` → строка 2: `check=legacy_vpn result=block [legacy_vpn_active]`, код 1",
                 "правило с таблицей 2022 → строка 3 block `[foreign_autoroute]`; resolved → строка 4 block `[resolved_unsupported]`",
                 "firewalld и NetworkManager → строки 5 и 6 `warn`, код 0",
                 "нет маршрута по умолчанию → строка 7 block `[no_uplink]`; 9 внешних интерфейсов → block `[too_many_uplinks]`",
                 "режим off и существует `cmsink` → строка 9 block `[stale_host_net]`",
                 "строка 10: `check=rp_filter result=ok all=0 default=2` при таких значениях в фактах",
                 "команда не обращается к сокету контроллера и не выполняет `ip … add|del|set|replace`, `nft -f` (запись подменного исполнителя пуста)"]),
    dict(id="IS01", source="I.S1", target="C.T1", level="L1",
         contract="Профиль туннеля хоста разрешается из хранилища до обращения к контроллеру.",
         check="`tests/audit_ab_assign.rs`, `tests/audit_fin_install.rs`.",
         values=["снимок с политикой хоста, профилем и источником → `assign::host` возвращает источник и выбор узла этого профиля",
                 "политики хоста нет → `Err(UnknownProfile)`; профиль ссылается на отсутствующий источник → `Err(UnknownSource)`",
                 "`cm host set tunnel` без источника конфига, с обоими источниками, `cm host set proxy --stored` → код 2, обращений к контроллеру нет"]),
    dict(id="IS02", source="I.S1", target="X.Q3", level="L2",
         contract="`cm host set tunnel --stored` поднимает туннель хоста на узле из хранилища.",
         check="`tests/fin_stand.py`, раздел `host`: хранилище с источником, узлы — подставные прокси.",
         values=["`cm host define --source s1`, затем `cm host set tunnel --stored` → код 0; `cm host status` → `mode=tunnel`; запрос хоста — HTTP 200, счётчик соединений подставного узла вырос",
                 "в файле поколения экземпляра `host` нет ключа `dns`; права файла 0600",
                 "без `cm host define` → `[unknown_profile]`, код 2; режим остался off"]),
    dict(id="IT01", source="R.W3", target="I.T1", level="L1",
         contract="Снятие удаляет ровно те объекты, которые сверяет сторож хоста.",
         check="`tests/audit_fin_install.rs`: подменные `NetExec` и `NetQuery`.",
         values=["`host.json` с одним внешним интерфейсом → команды: `host_firewall_commands(None)`, затем `host_destroy_commands`; файл удалён; запись `net:host` очищена",
                 "`host.json` нет, наблюдаются `cmsink` и правила 1990, 2000, 2001 → те же команды плюс по одной `ip rule del priority N` на каждое наблюдаемое правило схемы",
                 "чужое правило с приоритетом 1990 и таблицей 200 и правило с приоритетом 3000 → не удаляются",
                 "две аренды сетей приложений → для каждой команды снятия, аренда освобождена, каталог `etc_root/cm-<i>` удалён; отчёт `nets = 2`",
                 "повтор на чистом состоянии → отчёт `{cores:0, nets:0, host:false, errors:0}`"]),
    dict(id="IT02", source="I.T1", target="X.Q0", level="L2",
         contract="Кадр снятия доступен только суперпользователю и не пересекается с проходом сторожа.",
         check="`tests/audit_fin_install.rs` (кадр), `tests/fin_stand.py`, раздел `teardown`.",
         values=["кадр `teardown` от uid 1000 → `denied`; состояние не изменилось",
                 "кадр от uid 0 → ответ `ReplyData::Teardown`; следующий `overview` → host mode off, список туннелей без поколений",
                 "`cm controller teardown --offline` вне тестового режима не от uid 0 → код 4; при `--base` вне тестового режима → код 2",
                 "после снятия проход сторожа не выполняет ни одной команды (запись исполнителя пуста два прохода подряд)"]),
    dict(id="IT03", source="I.T1", target="FIN.Z", level="L2",
         contract="После снятия на стенде не остаётся ни одного объекта CM, и сеть хоста работает напрямую.",
         check="`tests/fin_stand.py`, раздел `teardown`.",
         values=["до снятия: туннель хоста включён, два туннеля приложений двух владельцев работают",
                 "`cm controller teardown` → код 0, отчёт `cores=3 nets=2 host=true errors=0`",
                 "после: `ip rule` без приоритетов 1000–2001; `ip link` без `cmtunh`, `cmsink`, `cmv*`, `cmtun*`; `ip netns list` без `cm-*`; `nft list tables` без `cm_host`; таблица `inet cm` без правил для интерфейсов",
                 "запрос хоста — HTTP 200 напрямую; процессов ядра нет",
                 "повтор команды → код 0, отчёт из нулей"]),
]
