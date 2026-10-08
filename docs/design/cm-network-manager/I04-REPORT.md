# I04: контракт ядер и адаптер mihomo — отчёт кодовой части (I04.T05.c)

Дата: 2026-10-08. Координатор: Claude. Ядро: mihomo 1.19.32 ([I05-PIN.md](I05-PIN.md)).

**Итог:** кодовая часть I04 закрыта: все подзадачи до `I04.T05.a` приняты с evidence уровня L1–L2 (unit, `mihomo -t`, harness в `unshare -rn`). Runtime-приёмка адаптера на установке (L4) не выполнялась; она вынесена в X.08 → X.07 и не блокирует I06/I07.

## Статусы

| Подзадача | Что | Уровень | Статус | Evidence |
|---|---|---|---|---|
| I04.T01.a | trait `CoreAdapter`, ошибки, три оси готовности | L1 | закрыта | [ADR-CORE-ADAPTER.md](ADR-CORE-ADAPTER.md), `tests/audit_i04_adapter.rs` |
| I04.T01.b | дескриптор возможностей pinned mihomo | L1 | закрыта | `src/core/mihomo/mod.rs` (те же срезы, что I03) |
| I04.T01.c | fake adapter | L1 | закрыта | `src/core/fake.rs`, `audit_i04_adapter` |
| I04.T02.a–c | каталоги instance, реестр аренд, `cm-core@.service` | L1 | закрыты | `src/core/{instance,leases,unit}.rs` (параметры `cm-core@.service` генерирует `unit.rs`) |
| I04.T02.d | обёртка legacy host | L1 | закрыта | `src/core/legacy_host.rs`, `audit_i04_legacy_host` |
| I04.T03.a | запрет конкурирующих настроек в worker | L1 | закрыта | `src/core/worker.rs` (`HOST_LISTENER_KEYS`), `audit_i04_mihomo_config` |
| I04.T03.b | ADR владения TUN | L2 | закрыта | [ADR-TUN-OWNERSHIP.md](ADR-TUN-OWNERSHIP.md), `audit_i04_tun` |
| I04.T04.a–b | генератор config и проверка `mihomo -t` | L2 | закрыты | `src/core/mihomo/{config,validate}.rs`, `audit_i04_mihomo_config` |
| I04.T04.c | lifecycle через API на unix-сокете | L2 | закрыта | `src/core/mihomo/{lifecycle,api}.rs`, `audit_i04_lifecycle`, [E1](stage2-evidence/2026-10-07/E1/summary.json) |
| I04.T04.d | статистика instance | L2 | закрыта | `src/core/mihomo/stats.rs`, `audit_i04_e2` (±5% к nft), [E2](stage2-evidence/2026-10-07/E2/summary.json) |
| I04.T04.e | N worker против одного ядра | L2 | закрыта | [ADR-WORKER-TOPOLOGY.md](ADR-WORKER-TOPOLOGY.md), `audit_i04_topology`, [замеры](stage2-evidence/2026-10-08/I04.T04.e/summary.json) |
| I04.T04.f | группы и правила издателя (D1, вариант 3) | L1+`mihomo -t` | закрыта | `src/core/mihomo/policy.rs`, `audit_i04_policy`, [G](stage2-evidence/2026-10-07/G/summary.json) |
| I04.T05.a | два worker + legacy host одновременно | L2 | закрыта | `audit_i04_e2`, раздельный egress по nft |
| **runtime (L4)** | паритет адаптера на установке | L4 | **NOT_RUN** | X.08 → X.07 |

Ревью волн: [этап 2, волна 1](REVIEW-2026-10-06-STAGE2-WAVE1.md), [этап 2, волна 2](REVIEW-2026-10-08-STAGE2-WAVE2.md). Gate на дату отчёта: EXIT 0, 522 passed, 0 failed, без `SKIPPED`.

## Что получилось

- **Один worker — один процесс mihomo** ([ADR-WORKER-TOPOLOGY](ADR-WORKER-TOPOLOGY.md)):
  - свой каталог 0700, арендованные порт `127.0.0.1:20000+` и сокет API;
  - запуск в собственной группе процессов (`setsid`); `stop` завершает всю группу и освобождает аренды;
  - после kill -9 — явное `Down`, аренды возвращает `reclaim_if_absent`.
- **Готовность по осям:**
  - `ApiReady` — API-сокет отвечает;
  - `RouteReady` — арендованный порт принимает соединения (после ревью; mihomo открывает API раньше listener-а);
  - `RemoteReachable` — delay выбранного узла.
- **Reload** — `PUT /configs` только после `mihomo -t`. При отказе прежний config остаётся байт в байт. Установленные соединения переживают reload (замер I04.T04.e).
- **Config worker** собирает только генератор:
  - listener `mixed` на арендованном порту;
  - путь API-сокета берётся из каталога instance (`external-controller-unix` из входного документа — `Forbidden`);
  - правила `GEOIP`/`GEOSITE` отклоняются, пока нет офлайн-доставки баз (I05.T04.a);
  - группы и правила издателя: `select`/`url-test`/`fallback`/`load-balance`, `sub-rules` без циклов, только inline rule-providers;
  - порядок правил: правила пользователя CM, правила издателя, один `MATCH`.
- **Проверки peer и владельца API-сокета** в одном месте (`core/mihomo/api.rs`). Хостовый VPN вызывает их же.
- **Владение TUN:** CM создаёт persistent TUN с owner uid worker-а, ядро открывает его без `CAP_NET_ADMIN` ([ADR-TUN-OWNERSHIP](ADR-TUN-OWNERSHIP.md)).

## Ограничения

- Офлайн-базы геоданных для worker нет: `GEOIP`/`GEOSITE` в worker — явная ошибка до I05.T04.a. На хосте (путь V.04) они работают через legacy-геоданные.
- Proxy-providers издателя не импортируются (D1). Группа издателя с `use` — явная ошибка `PublisherProvider`.
- `relay`-группы не поддерживаются, как и в pinned ядре.
- Память: около 16 МиБ PSS на туннель. Больше 10 одновременных туннелей — повод пересмотреть топологию ([ADR-WORKER-TOPOLOGY](ADR-WORKER-TOPOLOGY.md#решение)).
- Старт worker-ов последовательный (около 66 мс на туннель вместе с `mihomo -t`). Параллельный старт — задача контроллера.
- Xray: нет reload без рестарта, нет delay-запроса, нет TUIC ([I05-GAPS.md](I05-GAPS.md)). Это вход для I05, а не дефект I04.

## Влияние на следующие рубежи

| Рубеж | Что берёт из I04 |
|---|---|
| I05 (Xray, загрузчик ядер) | trait и fake. Для Xray `reload_without_restart = false`, поэтому вызывающему нужен явный рестарт. Загрузчик баз снимает запрет GEO в worker. |
| I06 (контроллер) | набор `MihomoWorker` на туннель, аренды из `core::leases`. Привилегированная часть создаёт TUN (вариант A) и запускает worker без capabilities. |
| I07 (режимы хоста) | legacy host остаётся отдельным instance (`legacy_host`). Два worker и legacy работают одновременно с раздельным egress. |
| I08 (сети приложений) | worker запускается в netns приложения, listener на арендованном порту внутри него. |
| I09 (fail-closed) | `Down` после падения, изоляция отказа по туннелю (19 из 19 в замере), аренды не зависают. |
| I16 (оценка качества) | `RemoteReachable` из delay и счётчики `/connections` на туннель. |

## Что не закрыто

- **L4:** паритет адаптера на установке (X.08 → X.07). Выполняет пользователь.
- **V.04/V.05:** ручная проверка `cm vpn use source:ID` и приёмка A/B/A. Это рубеж V, не I04.
