# Ревью блока 3, часть 2: I03.T05.c, I04, H.05.b, документы, 2026-10-06

Исполнитель — Grok 4.7 (коммиты `66845cd`…`54fc650`), ревью и доводка — координатор. Решения D6–D10 и Q01 — [отдельный документ](DECISIONS-D6-D10-Q01-2026-10-06.md).

## Найдено и исправлено

| Где | Проблема | Серьёзность | Исправление |
|---|---|---|---|
| `H.05.b` YAML | Импорт разбирался **двумя парсерами**: saphyr только проверял, дерево строил `serde_yaml` (FFI `unsafe-libyaml`). Критерий «путь импорта без unsafe_libyaml» не выполнен; риск расхождения парсеров (проверено не то, что загружено) | **P1** | `yaml_guard::load`: один проход по событиям saphyr строит дерево с дублями ключей, глубиной и лимитом значений; core schema YAML 1.2 (ведущие нули и `yes` — строки, `.inf`/`.nan` — отказ); `parse_yaml` на serde_yaml удалён. serde_yaml остался только в legacy `src/vpn.rs` |
| `I04.T02.c` unit | Ядро запускалось из `instances/<id>/bin/mihomo` внутри `ReadWritePaths` службы, работающей от root: скомпрометированное ядро могло подменить свой бинарник | **P1** | бинарник в `/var/lib/cm/cores/mihomo/` (read-only через `ProtectSystem=strict`); добавлены `ProtectKernelTunables/Logs/Clock/Hostname`, `RestrictNamespaces/Realtime/SUIDSGID`, `LockPersonality`, `SystemCallArchitectures=native`, `RestrictAddressFamilies`, `DevicePolicy=closed`+`DeviceAllow=/dev/net/tun` (или `PrivateDevices`) |
| `I04.T03.a` worker | «Чужой listener — ошибка» проверялась только для `listen`/`port`/`listeners`: `mixed-port`, `socks-port`, `external-controller`, `allow-lan`, `tun`, `dns.listen` молча отбрасывались; порт строкой не проверялся | P1 | все ключи-listener mihomo отвергаются; любой `port` ≠ арендованного (в т.ч. строкой) — отказ; настройки резолвера без `listen` разрешены |
| `I04.T02.b` leases | `flock(LOCK_EX)` без таймаута: один зависший держатель останавливал выдачу ресурсов всем | P2 | `LOCK_NB` с таймаутом 5 с → `LeaseError::Busy`; временный файл с `O_NOFOLLOW` |

Регрессии: `yaml_guard::tests::values_follow_the_core_schema_and_duplicates_are_refused`, `audit_i04_adapter`: `worker_refuses_every_host_listener_key`, `core_unit_runs_binary_outside_writable_paths`, `lease_lock_gives_up_instead_of_hanging`.

## Приняты без изменений

`I03.T05.c` (транспорт на 127.0.0.1, классы отказа), `I04.T01.a–c` (trait, дескриптор из таблиц I03, fake-адаптер), `I04.T02.a` (каталоги instance), `I17-D.T01.a` (fixtures осей), документы `V.01`/`H.08`/`Q01` — как материалы к решениям.

## Находки вне кода Grok

- Legacy `cm-vpn.service` (`src/main.rs`) тоже запускает ядро из записываемого `/var/lib/cm/vpn/bin`. Не изменено: unit у работающих установок; исправить в `I04.T02.d` (legacy wrapper) вместе с доставкой ядер `I05.T04`.
- Пин Xray (`I05.T01.a`) не записан: GitHub вернул 403. Остаётся за Grok при доступе.

## Проверки

Полный gate после доводки: **EXIT=0, 473 passed, 0 failed** (включая COSMIC, rustfmt, clippy новых модулей, H.11 harness). Режим CI (`CM_SKIP_COSMIC=1`): EXIT=0. Installation runtime: NOT_RUN (план — [INSTALL-PLAN](INSTALL-PLAN.md)).
