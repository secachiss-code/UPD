"""FIN, часть 3: очередь хоста (направление X) и приёмка (направление P).

Шаги очереди хоста меняют настоящую машину. Их пишет роль 1, проверяет роль 2 (без изменения хоста),
а запускает пользователь, когда ему удобно. Этапы X и I18 общего плана, плюс проверки, отложенные на установку.
"""

DIRECTIONS = [
    ("X", "Очередь хоста", "Проверки на настоящей машине — одной командой каждая, с планом, защитой от потери сети и откатом."),
    ("P", "Приёмка", "Решения по открытым вопросам записаны; итог «готово или нет» выводится из evidence программой, а не оценкой."),
]

V = [
    dict(id="X.Q0", direction="X", title="Дать общий исполнитель шагов очереди хоста с планом, защитой и откатом.", size="M", level="L2",
         deps=["I.H1", "I.T1", "C.D1"], files=["tests/hostq/run.py", "tests/hostq/lib.py", "tests/hostq/steps/__init__.py"],
         covers=["X.07"], external=[],
         spec="""\
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
- `CM_HOSTQ_STAND=1`: шаг выполняется по-настоящему внутри пространства имён стенда; снимок и `sudo` не требуются, каталог evidence — из `CM_HOSTQ_EVIDENCE`."""),

    dict(id="X.Q1", direction="X", title="Проверить установку, миграцию, хранилище, источники и пакетные сценарии на машине.", size="M", level="L4",
         deps=["X.Q0"], files=["tests/hostq/steps/install.py", "tests/hostq/steps/migrate.py", "tests/hostq/steps/store.py",
                               "tests/hostq/steps/source.py", "tests/hostq/steps/packages.py"],
         covers=["X.02", "X.03", "X.04", "X.06"], external=[],
         spec="""\
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

`hq-packages`: `uninstall_leaves_no_net` выполняется только с флагом `--with-uninstall` (иначе UNKNOWN): `! sudo cm uninstall`, затем `ip rule`, `ip link`, `nft list tables` без объектов CM, затем `! sudo <бинарник> install`."""),

    dict(id="X.Q2", direction="X", title="Сравнить прежний и новый путь на одном узле по схеме A/B/A.", size="M", level="L4",
         deps=["X.Q0"], files=["tests/hostq/steps/parity.py"], covers=["X.05", "X.08", "V.05"], external=[],
         spec="""\
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

Формулировки «VPN исправлен» в выводе и evidence нет: шаг сообщает только совпадение или первую отличающуюся стадию."""),

    dict(id="X.Q3", direction="X", title="Проверить туннель хоста на настоящей машине и вернуть её в исходное состояние.", size="L", level="L4",
         deps=["X.Q0", "R.Z", "I.S1"], files=["tests/hostq/steps/host.py"], covers=["I07.T05.a", "I09.T05.a", "I10.T05.a"], external=[],
         spec="""\
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

У проверок 4, 7, 8, 9, 10, 11, 14 задан `stand_ref` — номер значения ребра уровня L2 (`AB-DAG.json` или `FIN-DAG.json`), которое проверяет то же на стенде: например `IS02.1` для проверки 4, `RW03.2` для 10, `IT03.3` для 14. Метки стенда совпадают с номерами значений, поэтому их можно назвать до того, как стенд написан."""),

    dict(id="X.Q4", direction="X", title="Пройти критерии A01–A20 на настоящих приложениях и проверить интерфейс глазами.", size="L", level="L4",
         deps=["X.Q0", "R.Z", "C.C1"], files=["tests/hostq/steps/apps.py"],
         covers=["I13.T05.a", "I15.T05.a", "I17.T04.a", "I17.T05.a", "I18.T02.a", "I18.T03.a"], external=[],
         spec="""\
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

`rollback`: `cm tunnel down --force` для созданных туннелей, `cm app remove`, `cm group remove`; проверка `store_restored` — ревизия хранилища выросла, но описаний `hq-*` нет."""),

    dict(id="X.Q5", direction="X", title="Измерить разброс качества на настоящем узле и вывести из него размер выборки и допуск.", size="M", level="L4",
         deps=["X.Q0"], files=["tests/hostq/steps/pilot.py"], covers=["I16.T03.a"], external=[],
         spec="""\
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

Шаг не меняет константы в коде: рекомендованные значения записываются в evidence. Автоматический выбор узла в выпуск не входит (решение D-Q1), поэтому шаг необязателен."""),

    dict(id="P.D1", direction="P", title="Записать принятые решения: отпечаток, адаптер приложений, границы выпуска и порядок возврата.", size="M", level="D",
         deps=[], files=["docs/design/cm-network-manager/ADR-FINGERPRINT.md", "docs/design/cm-network-manager/I15R-CATALOG.md",
                         "docs/design/cm-network-manager/ADR-APP-ADAPTER.md", "docs/design/cm-network-manager/RELEASE-SCOPE.md",
                         "docs/design/cm-network-manager/QUESTIONS.md"],
         covers=["I15-R.T04.a", "I15-R.T05.a", "I15.T01.a", "I18.T04.a", "I05.T02.b", "I05.T05.a", "I10.T02.b"], external=[],
         spec="""\
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

**`QUESTIONS.md`**: в разделах Q26 и Q27 первая строка заменяется на «Решено координатором 2026-10-11: …» со ссылкой на `I15R-CATALOG.md`; остальной текст разделов не меняется."""),

    dict(id="P.T1", direction="P", title="Выводить трассировку требований, отчёт по машине и итог приёмки из evidence.", size="M", level="L0",
         deps=["P.D1", "X.Q1", "X.Q2", "X.Q3", "X.Q4", "X.Q5"],
         files=["docs/design/cm-network-manager/tools/accept.py", "docs/design/cm-network-manager/I18-TRACE.md",
                "docs/design/cm-network-manager/X-RUNTIME-REPORT.md", "docs/design/cm-network-manager/I18-ACCEPTANCE.md"],
         covers=["X.07", "I18.T01.a", "I18.T05.a"], external=[],
         spec="""\
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

`--check` ничего не пишет: печатает итог и возвращает 0 при `ready`, 1 при `blocked`. Программа не запускает ни gate, ни шаги: она только читает их результаты."""),
]

E = [
    dict(id="XQ01", source="X.Q0", target="X.Q1", level="L2",
         contract="План ничего не меняет, а запуск невозможен без согласия и точки возврата.",
         check="`tests/audit_fin_hostq.rs` запускает `tests/hostq/run.py` с `CM_HOSTQ_FAKE`.",
         values=["`run.py plan hq-install` → код 0; журнал команд пуст; в выводе строки с `! ` перед `sudo`",
                 "`run.py run hq-install` без `--yes` → `[needs_yes]`, код 3; журнал пуст",
                 "`run.py run hq-install --yes` без записи о снимке → `[no_snapshot]`, код 3; с `--no-snapshot` без `--backup-dir` → код 2",
                 "`run.py run hq-host --yes` без PASS у `hq-install` → `[requires hq-install]`, код 3",
                 "`run.py list` → по строке на каждый шаг из `tests/hostq/steps/`; у шага без evidence статус `-`",
                 "запуск от uid 0 (подменённый `os.geteuid`) → код 2"]),
    dict(id="XQ02", source="X.Q0", target="X.Q2", level="L2",
         contract="В evidence попадают только статусы и слова из набора шага.",
         check="`tests/audit_fin_hostq.rs`.",
         values=["проверка с `detail`, содержащим URL или заглавные буквы → в файле `detail_rejected`",
                 "файл evidence содержит ключи `step`, `rev`, `binary_sha256`, `started`, `finished`, `status`, `checks`, `rollback` и ни одной строки из подменных ответов команд",
                 "две фазы A с разным `reach` → проверка сравнения UNKNOWN, не FAIL и не PASS",
                 "фазы A равны, B отличается на стадии `tls` → FAIL, `detail` `stage=tls`"]),
    dict(id="XQ03", source="X.Q0", target="X.Q3", level="L2",
         contract="Откат выполняется при любом исходе шага, а потеря сети после него — отдельная тревога.",
         check="`tests/audit_fin_hostq.rs` (подменные ответы); `tests/fin_stand.py`, раздел `hostq`, с `CM_HOSTQ_STAND=1`.",
         values=["`run` шага бросает исключение → `rollback` вызван, в evidence `rollback` заполнен, статус шага FAIL",
                 "подменные ответы: связь до шага есть, после отката нет → в журнале есть `cm controller teardown`; проверка `network_restored` FAIL с p0; в выводе блок «как вернуть сеть» с номером снимка",
                 "связи нет до шага → `[no_network]`, код 3; `run` не вызывался",
                 "стенд: `run.py run hq-host --yes` внутри пространства имён → статус PASS или UNKNOWN; проверки 3, 4, 7, 8, 9, 10, 14 — PASS; после шага `ip rule` без приоритетов 1900–2001",
                 "стенд: шаг прерван `SIGTERM` после включения туннеля → не позже чем через 20 с правил 1900–2001 нет"]),
    dict(id="XQ04", source="X.Q0", target="X.Q4", level="L2",
         contract="Ручная проверка без ответа — UNKNOWN, и шаг убирает за собой всё, что создал.",
         check="`tests/audit_fin_hostq.rs`; `tests/fin_stand.py`, раздел `hostq`.",
         values=["`ask` при закрытом stdin → `skip`; все проверки разделов `account`, `applet`, `tui` — UNKNOWN",
                 "ответ `n` на вопрос → проверка FAIL",
                 "стенд: `run.py run hq-apps --yes --source s1` → проверки a04, a05, a06, a07, a11, a14, a16_native_runs, a20_export_clean, a20_no_verified_state_app — PASS",
                 "стенд: после шага в `cm app list` и `cm group list` нет идентификаторов `hq-*`; `cm tunnel list` без экземпляров `t-hq-*` с поколениями"]),
    dict(id="XQ05", source="X.Q0", target="X.Q5", level="L1",
         contract="Статус шага выводится из проверок одним правилом, код выхода — из статуса.",
         check="`tests/audit_fin_hostq.rs`.",
         values=["проверки [PASS, UNKNOWN] → шаг UNKNOWN, код 0",
                 "проверки [PASS, FAIL, UNKNOWN] → шаг FAIL, код 1",
                 "проверки [SKIP, SKIP] → шаг SKIP; `preflight` с FAIL → `run` не вызывался, шаг SKIP, код 3",
                 "`run.py all --yes`: шаг с FAIL p0 → следующие шаги не запускались"]),
    dict(id="XQ06", source="X.Q1", target="P.T1", level="L2",
         contract="Шаги установки и миграции проверяют состояние, а не только код возврата команд.",
         check="`tests/audit_fin_hostq.rs` (подменные ответы).",
         values=["`hq-install`: подменный `systemctl is-enabled cm-controller.socket` → `disabled` → проверка `controller_socket_enabled` FAIL",
                 "sha256 бинарника отличается от записанного → `binary_matches_manifest` FAIL",
                 "состояние службы VPN хоста до и после разное → `legacy_vpn_untouched` FAIL с p0",
                 "`hq-source` при пустом хранилище → все проверки UNKNOWN, `detail` `no_sources`",
                 "`hq-packages` без `--with-uninstall` → `uninstall_leaves_no_net` UNKNOWN; в журнале нет `cm uninstall`",
                 "план `hq-install`: `rollback` не содержит команд удаления"]),
    dict(id="XQ07", source="X.Q2", target="P.T1", level="L2",
         contract="Сравнение путей называет первую отличающуюся стадию и не делает выводов сверх измеренного.",
         check="`tests/audit_fin_hostq.rs` (подменные ответы).",
         values=["FlClash не найден → `flclash_vs_cm` UNKNOWN, `detail` `flclash_absent`; остальные сравнения выполняются",
                 "нет активной подписки прежнего VPN → все три проверки UNKNOWN, `detail` `no_legacy_subscription`; команд, меняющих систему, в журнале нет",
                 "после шага в журнале есть возврат прежней подписки и режима; проверка `legacy_restored` PASS",
                 "в выводе и evidence нет слов «исправлен», «fixed»"]),
    dict(id="XQ08", source="X.Q3", target="P.T1", level="L2",
         contract="Шаг туннеля хоста отличает утечку от недоступности и не объявляет успех по косвенным признакам.",
         check="`tests/audit_fin_hostq.rs` (подменные ответы); настоящий прогон — `tests/fin_stand.py` (ребро `XQ03`).",
         values=["подменный ответ: после `kill -9` ядра запрос вернул 204 → `core_kill_blocks` FAIL с p0",
                 "хеш выхода совпал с прямым → `exit_differs` UNKNOWN, не FAIL",
                 "`cm host check` с `Block` `resolved_unsupported` → шаг SKIP, `detail` содержит токен; команд, меняющих систему, нет",
                 "`Block` только `legacy_vpn_active` → шаг идёт; в журнале `cm vpn stop` раньше `cm host set tunnel`, в откате — запуск прежнего VPN",
                 "без `--with-link-flap` и `--manual-suspend` → `uplink_flap` и `suspend_resume` UNKNOWN с причинами `needs_real_uplink`, `needs_suspend`"]),
    dict(id="XQ09", source="X.Q4", target="P.T1", level="L2",
         contract="Каждый критерий приёмки имеет проверку и статус; непроверенное не выдаётся за проверенное.",
         check="`tests/audit_fin_hostq.rs`.",
         values=["без `--browser` → `a12_running_elsewhere` UNKNOWN; без `--appimage` → `a16_appimage_partial` UNKNOWN",
                 "подменный ответ: запрос из сети приложения после `kill -9` ядра вернул 204 → `a07_no_direct_after_kill` FAIL с p0",
                 "подменный ответ: `cm tunnel list` с `app=verified` → `a20_no_verified_state_app` FAIL",
                 "`a16_flatpak_refused` — PASS, только если вывод содержит `[unsupported_not_verified]` и код равен 2",
                 "в evidence есть раздел матрицы исполнителя со строкой на каждую проверенную упаковку"]),
    dict(id="XQ10", source="X.Q5", target="P.T1", level="L1",
         contract="Размер выборки и допуск выводятся из измеренного разброса по записанной формуле.",
         check="`tests/audit_fin_hostq.rs`: функции расчёта шага на заданных сериях.",
         values=["серии A с медианами 100, 110, 104 мс → `noise` = 0.10; `min_margin` = 0.20",
                 "серия B с медианой 150 мс при этих A → `b_differs` true; с медианой 115 мс → false",
                 "серии, чьи медианы первых 10 проб расходятся на 0.30, а первых 20 — на 0.08 → `min_probes` = 20",
                 "расхождение серий A 0.6 → `noise_measured` UNKNOWN, `detail` `unstable`",
                 "шаг не пишет ни в один файл под `src/`"]),
    dict(id="PD01", source="P.D1", target="P.T1", level="L0",
         contract="Каждое решение записано со статусом, а каждая подзадача вне выпуска названа поимённо.",
         check="`tests/audit_fin_docs.rs`: разбор текста документов.",
         values=["`ADR-FINGERPRINT.md`: таблица не короче 10 строк; в каждой строке статус из набора go, conditional, no-go; строка UA — no-go",
                 "`RELEASE-SCOPE.md`: пять разделов в заданном порядке; в разделе «Не входит» есть `I05.T02.b`, `I05.T05.a`, `I10.T02.b`; есть обе метки `accept:unknown`",
                 "раздел «Как вернуть» содержит `cm host set off`, `cm controller teardown`, `--offline` и слово «снимок»",
                 "`QUESTIONS.md`: разделы Q26 и Q27 начинаются словами «Решено координатором 2026-10-11»; число строк файла изменилось не больше чем на 4",
                 "ни в одном из четырёх новых документов нет слов «анонимно», «гарантирует анонимность»"]),
    dict(id="PT01", source="P.T1", target="FIN.Z", level="L0",
         contract="Итог приёмки — функция от evidence: одно невыполненное условие даёт «blocked» и называет, кто действует дальше.",
         check="`tests/audit_fin_docs.rs` запускает `accept.py` на наборах входов из `tests/fixtures/fin/accept/`.",
         values=["набор «всё выполнено» → `--check` код 0, `ready`",
                 "тот же набор без evidence шага `hq-host` → `blocked`; один `Blocker` с `who` user и командой `run.py run hq-host --yes`",
                 "шаг `hq-apps` со статусом UNKNOWN → `blocked` (обязательный шаг); шаг `hq-pilot` UNKNOWN → на итог не влияет, проверка попадает в список «Не проверено на машине»",
                 "дефект P1 в summary → `blocked`, `who` coder; замечание `test_lint` → `who` tester",
                 "подзадача без вершины и вне «Не входит» → `open`, `blocked`, `who` coordinator; `V.02` и `H.10` → `closed` (решения D-V1, D-C1)",
                 "подзадача `X.05` при шаге `hq-parity` со статусом UNKNOWN → `covered_unverified`; итог от неё не зависит",
                 "`X-RUNTIME-REPORT.md`: шаг UNKNOWN остаётся UNKNOWN; число строк равно числу шагов",
                 "`I18-TRACE.md`: 11 строк W01–W11; у W01 в подзадачах вне выпуска есть `I05.T05.a`",
                 "`--check` не создаёт и не меняет файлов (времена изменения прежние)"]),
]
