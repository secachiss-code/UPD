# I03: закрытие кодовой части

Дата: 2026-10-06. Задача `I03.T05.f`. Кодовая приёмка I03 закрыта. Runtime-приёмка (настоящие подписки, провайдер, сеть хоста) не выполнялась и ждёт трека X. Ниже эти два статуса разделены.

## Что реализовано

| Часть | Где | Документ |
|---|---|---|
| Native-парсер mihomo YAML/JSON, строгие опции по протоколам, TLS/REALITY | `src/sources/parser/native/` | [I03-PARSER-FINAL.md](I03-PARSER-FINAL.md) |
| URI-ссылки и списки (plain/base64, не больше 2 слоёв), частичный импорт по D3 | `src/sources/parser/uri.rs`, `list.rs` | [I03-DECISIONS-2026-10-06.md](I03-DECISIONS-2026-10-06.md) |
| YAML через события saphyr (лимиты глубины и размера, без serde_yaml в пути импорта) | `src/sources/parser/yaml_guard.rs` | D5 |
| UA negotiation, транспорт ureq + resolver с общим дедлайном, редиректы запрещены | `src/sources/negotiation.rs`, `transport.rs`, `pipeline.rs` | D4, [I03-UA-NEGOTIATION](I03-UA-NEGOTIATION-2026-10-04.md) |
| Пропуски с подтверждением, граница авто-обновления, `TlsVerification` | `src/sources/artifact.rs`, `import_confirmation.rs` | D1, D2 |
| Ручной сервер: секрет не в argv, файл секрета не шире 0600 и принадлежит своему uid | `src/sources/manual.rs` | — |
| Provenance, private fetch settings, private artifacts | `src/sources/` | контракты I03 от 2026-10-04 |
| Проверка корпуса ядром (при `CM_TEST_MIHOMO`) | `tests/audit_i03_core_check.rs` | — |

## Кодовая приёмка — PASS

- Gate `sh tests/check_audit.sh` (с COSMIC): EXIT=0, 474 passed, 0 failed. Clippy по всему крейту: `-D warnings -D clippy::undocumented_unsafe_blocks`.
- [Инвентарь регрессий](I03-REGRESSION-INVENTORY.md): 458 проверок I01–I03 с ID, уровнем и статусом.
- Сборка ([манифест](i03-evidence/build-2026-10-06/summary.json)) на чистом дереве `38cc5be`:
  - CLI musl `cm 0.2.8`, sha256 `1a527fe1ee6568a7fc3e184e4a46cc058001282f732abda872d33d3d78799f88`;
  - COSMIC, sha256 `1178ed21aa051139d95f87e9cdb2c468df0e7cfbedd99986de6a2752f1440213`;
  - пересборка CLI в чистом target дала тот же хеш (**MATCH**).
- Драйвер `tests/check_i03_regressions.py --run` против этого бинарника: 21 набор, 21 PASS ([сводка](i03-evidence/driver-local-2026-10-06/summary.json)). Это локальные фикстуры, не установка.

## Runtime-приёмка — NOT_RUN

| Что | Где выполняется | Статус |
|---|---|---|
| Импорт настоящих подписок пользователя, actual UA провайдера, refresh, отказы | `X.04` по [INSTALL-PLAN.md](INSTALL-PLAN.md), после `V.02` (CLI `cm source …`) | NOT_RUN |
| Сверка корпуса с живым ядром mihomo 1.19.32 | `CM_TEST_MIHOMO`, нужен бинарник | NOT_RUN |
| Процесс ядра в harness-сценарии (`H.11`, часть с ядром) | после появления бинарника | NOT_RUN |
| 112 проверок «реальный провайдер», 40 «установка», 37 «реальная ФС» из инвентаря | треки X.02–X.04 | NOT_RUN |

Утверждения «подписки импортируются у провайдера» и «VPN исправлен» не делаются: для них нужны `X.04` и `X.05`.

## Что дальше

- `V.02` — CLI `cm source …` поверх нового импорта (Grok, блок 4).
- `H.10` — зелёный CI появится после push.
- Установочное окно по [INSTALL-PLAN.md](INSTALL-PLAN.md): драйвер запускать с этим манифестом. После любой правки кода нужен новый манифест — драйвер проверяет хеши исходников.
