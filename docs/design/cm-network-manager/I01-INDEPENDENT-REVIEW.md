# I01: независимое заключение

Дата: 2026-10-03 UTC. Рецензент проверил исходники, контракты, test/harness code и сохранённые результаты отдельных исполнителей. Рецензент не запускал тесты, сетевые запросы через VPN, сборку пакета, установку или миграцию и не менял product code. Источники: [контракт](I01-TEST-CONTRACT.md), [отчёт исполнителя](I01-EXECUTION-REPORT.md), [решения](I01-DECISIONS.md).

**Общий verdict: I01 целиком не принят; реальная миграция и заявление «VPN исправлен» — NO-GO.** Принимаются фактический baseline с явно указанными пробелами, результаты локальных регрессий и ограниченные защитные изменения C17/C18. Сетевые пилоты исполнены, но критерий устойчивой connectivity не пройден; преимущество одной стороны не установлено.

## Вердикт по задачам

| Задача | Независимый verdict | Что принято / что остаётся |
|---|---|---|
| I01.T01 | Частично принято | C01/C02 дают версии, digests, происхождение снимков, процессы и состояние служб. C04 BLOCKED: действующая цепочка выбора живого FlClash неизвестна; private worker selection её не заменяет. |
| I01.T02 | Принята диагностика с unknown | Исторические журналы реально классифицированы; contemporaneous config отсутствует. Причинность INCONCLUSIVE. Текущий curl35 локализует симптом клиента, но не DNS/dial/REALITY/WS этап внутри core. |
| I01.T03 | Принято исполнение ограниченных pilots; приёмка network outcome не пройдена | Подготовка одинаковых node/config и A1/B/A2 подтверждена. WS: 4/10, 2/10, 0/10; TCP/REALITY: 0/10 во всех фазах. Это host-conditioned proxy, без доказанной независимости от живого Fl TUN и без performance winner. |
| I01.T04 | BLOCKED / NO-GO | C17 безопасно запрещает известный legacy install. Перенос, backup/journal/recovery, ownership и busy-operation concurrency не реализованы и не приняты. |
| I01.T05 | Приняты выбранные local regressions и baseline-report; scope ограничен | 175 library, 56 binary, 24 integration, 15 COSMIC; отдельные guard/help проверки. Это не package installation, графический end-to-end, host VPN или full migration regression. Короткие launcher/preparation logs имеют менее полную provenance. |

## Provenance и конфигурации

HEAD `07ef2381fb962fe74958c504a896c584f6a4e4be` относится к dirty tree и не идентифицирует испытанные изменения самостоятельно. [Final bin manifest](i01-evidence/final-bin-run.json) фиксирует source before/after; текущие `main.rs`, `lib.rs`, `migration.rs`, `i18n_table.rs` и CLI digest сверены рецензентом с сохранённым evidence. CLI после C18: SHA-256 `22b00cb906e430ea689faeea4f09e0641a2f6e9e073ba5ef71ec89435dc695d4`. Первые и поздние запуски различаются в отчёте; прежние результаты не названы повторным исполнением нового агента.

Установленный UPD 0.2.7 и исходники CM 0.2.8 — разные состояния. Сторона A — неизменённая копия установленного native ядра UPD, SHA-256 `98f50f0fdc498a9be1aa65ee991cda1c5854650d948e9001d7e13b703ab6cc3d`, API v1.19.32. Сторона B — установленный FlClashCore, SHA-256 `b5014e62eda428795056221ce304990be30512f821d803a3b4bd8b0abafa1059`, package 0.8.98-1, private API 1.10.0. Это не сравнение нового ядра, собранного проектом CM, с upstream mihomo другой версии.

Driver передаёт `initClash.version = 0`; в matching-tag `core/hub.go` это Android `sdkVersion`, а не подстановка API version. FlClash `go.mod` заменяет mihomo локальным `Clash.Meta`, связанным с fork. API 1.10.0 нельзя выдавать за установленную upstream revision или воспроизводимое соответствие release source бинарнику. Первичные источники: [hub.go](https://raw.githubusercontent.com/chen08209/FlClash/v0.8.98/core/hub.go), [go.mod](https://raw.githubusercontent.com/chen08209/FlClash/v0.8.98/core/go.mod), [.gitmodules](https://raw.githubusercontent.com/chen08209/FlClash/v0.8.98/.gitmodules).

[Baseline](i01-evidence/baseline.json) показывает 62/62 одинаковых полных определений UPD source/runtime; UPD/Fl saved имеют 62 общих имени, но только одно полное совпадение. Credentials включены в локальное сравнение, значения не экспортированы. Это доказывает текущие расхождения, а не их причину и не параметры живого Fl selected node. Последняя историческая ошибка предшествует mtime сохранённых конфигов; исторические DNS/dial/WS/HTTP502/timeout/TUN counts нельзя связывать с текущим node задним числом. Текстовые классы пересекаются; ноль TLS/REALITY matches не означает отсутствие таких отказов.

## Проверки C01–C19

| Проверки | Независимая оценка evidence |
|---|---|
| C01–C02 | PASS в scope read-only inventory и текущего локального сравнения. |
| C03 | INCONCLUSIVE для исторической причинности; сохранённый [final journals](i01-evidence/final-journals.json) подтверждает исполненную классификацию. |
| C04 | BLOCKED: live REST discovery HTTP400/Unix error не разрешил selection. Это не доказательство неисправности live VPN. |
| C05 | PASS подготовки выбранных workers: одинаковые config bytes/full node hashes, native validate и actual own Fl IPC init/setup/startListener, проверенные effective fields и fixed selection. Все engine defaults и upstream build provenance полностью не установлены. |
| C06 | FAIL connectivity / INCONCLUSIVE performance. [WS observations](i01-evidence/pilot.json) и [summary](i01-evidence/pilot-summary.json): 30 measured + 3 warm-up; 24 measured failures curl35. |
| C07/C09 | BLOCKED: фактический перенос и fault-injected persisted recovery отсутствуют. |
| C08 | PASS только отказ при old/new conflict, подтверждённый actual guard/CLI. Не приёмка исполняющей миграции. |
| C10/C15/C16 | BLOCKED полного scope. Сохранение foreign sentinels и отказ по helper/entrypoint presence подтверждены; ownership-safe migration, busy locks, переход entrypoints не проверены. |
| C11 | NOT_RUN миграции; env precedence/no-bypass и повторный отказ не заменяют idempotency переноса. |
| C12–C13 | PASS выбранных local suites с указанной revision. Initial sandbox failures сохранены; последующие PASS относятся к новым фактическим запускам. |
| C14 | Финальная сверка после C19 ожидается; см. раздел ниже. |
| C17 | PASS safety barrier: [9 Rust fixtures](i01-evidence/final-guard-tests.log), [9 actual CLI cases](i01-evidence/cli-install-guard.json). Новое permission-error case проверено отдельным final guard run; старый log из 8 tests не является текущим полным suite. |
| C18 | PASS: [12 actual help/--help cases](i01-evidence/help-locales.json) в шести локалях и [8 i18n tests](i01-evidence/final-i18n-tests.log). |
| C19 | FAIL connectivity / INCONCLUSIVE performance: [TCP/REALITY pilot](i01-evidence/c19-pilot.json) 0/30 measured и 0/3 warm-up; [live controls](i01-evidence/c19-controls.json) 0/4. Все failures curl35. |

C17 source ordering проверен независимо: production guard в `main()` до `Config::load/save`; direct `cmd_install()` также первым действием вызывает guard; прежние stop/remove/rename из `migrate_upd()` удалены. Root задаётся явно, production использует `/`, env bypass отсутствует. Final dangling symlinks, неоднозначные ancestors и ошибки чтения приводят к отказу. Наличие `/usr/bin/upd` не называется доказательством ownership; foreign entry сохраняется. Guard не защищает subsequent fresh CM target ownership и не является транзакцией/TOCTOU guarantee. Fixture root предполагается доверенным.

C18 исправил найденную координатором регрессию: дописывание предупреждения внутрь `USAGE` ломало exact translation lookup. Сейчас старый полный ключ восстановлен, warning переводится отдельным ключом с пятью переводами. Строки source и dictionary сверены; локализованные CLI результаты относятся к текущему digest. Раннее source-review C17 не выявило этот смежный дефект; финальная приёмка учитывает его исправление и отдельную проверку.

## Что доказывают сетевые опыты

C06 и C19 используют одинаковый полный выбранный узел для A/B внутри каждого опыта. TCP/REALITY C19 — новый диагностический бюджет, а не повтор C06. В C19 по пять measured запросов на каждый из двух targets в каждой фазе; последовательность, request count и timeout укладываются в контракт. Ноды WS и TCP/REALITY выбраны из сохранённого Fl source детерминированно, не объявлены активными узлами живого клиента.

Offline preparation имеет private net namespace. Public pilots используют общий host network namespace; живой Fl TUN мог нести outer traffic всех workers. Равенство route/rule snapshots и namespace не устанавливает физическую независимость пути, конкретный upstream выбранный живым Fl узел или равенство всех engine defaults. Допустим только host-conditioned proxy результат.

Live controls направлены на подтверждённый root FlClashCore socket: executable digest + fd inode + LISTEN address/port; перед каждым запросом повторяется проверка inode в socket table. Они не меняют live config/selection. Результат 0/4 не подтверждает предположение «рабочий контроль», но не доказывает отказ всех приложений или всего VPN.

curl35 — симптом TLS negotiation на стороне curl при запросе через HTTP proxy. Точный CONNECT status, OpenSSL stderr и contemporaneous error stage ядра не сохранены; REALITY/WS/root cause определить нельзя. Значения `time_starttransfer` при HTTP000 не являются подтверждённым TTFB успешного ответа target. [Passive curl audit](i01-evidence/c19-curl-passive-audit.json) исключает обнаруженную текущую curlrc/proxy-env подмену; это последующее наблюдение, не ретроспективная гарантия окружения каждого запроса. Дополнительных запросов для реконструкции ошибки не выполнялось.

Latency только успешных WS запросов имеет survivor bias; A1/A2 различаются. Нельзя выбирать winner, заявлять p95, долгосрочную надёжность либо исправленный host TUN. Полный минимальный experiment config отличается от исходных application configs; перенос вывода на них не обоснован.

## C14: окончательная проверка сохранности

До C19 подтверждены root [baseline](i01-evidence/baseline.json) и [after](i01-evidence/final-host-baseline.json) с равными 11 выбранными snapshot entries и errors0 защищённых trees. Полнота ограничена этими assets; firewall, все unit files и вся ОС не снимались. Own workers reaped, private config trees removed подтверждаются lifecycle evidence.

Финальная root after-сверка C19 ещё ожидается. Пустой/непрочитанный JSON не принимается как успешный snapshot. До финализации этого раздела заключение по C14 после C19 остаётся pending.

## Блокирующие пробелы и следующий шаг

1. **Миграция:** сохранить C17 barrier. Реализовать production-used `MigrationPlan`, private backup и durable journal; ownership источников/назначений и effective units; блокировку legacy helpers/new operations; переход cron/desktop/CLI. В транзакцию включить subsequent binary/GUI/units/install failures. Отдельный executor проверяет fault injection, прерывание процесса и recovery новым процессом; до этого C07–C11/C15–C16 и I01.T04 не закрывать.
2. **VPN:** не менять UA/DNS/credentials и не повторять запросы без нового ограниченного контракта. Следующий опыт должен сохранять redacted curl CONNECT/TLS evidence и связывать ошибки собственного core с request/time/config generation; first local checks не требуют remote requests. Для независимого outer path нужен отдельный разрешённый disposable environment/route design; живую сеть текущий scope менять не разрешает.
3. **Live selection:** при необходимости получить поддержанный read-only способ чтения живого Fl selection; private worker RPC уже доказан, но к живому IPC mutating methods не применять. Исторический config не реконструировать догадками.
4. **Приёмка:** фиксировать финальные artifacts/digests, ограничения коротких launcher/preparation logs и failed harness attempts. Scoped PASS C17/C18/local regressions не переводит I01.T04 или весь I01 в complete и не разрешает I02–I18, пакет или host install.

## Последующий проход чтения, 2026-10-03

Этот проход не является повторным заключением прежнего рецензента и не запускал тесты. Он прочитал C20, которого раздел выше не видел, и сверил утверждения [baseline](I01-BASELINE-REPORT.md) с JSON. Прежний вердикт о C19 и о NO-GO миграции сохраняется.

C20 добавляет локализацию, а не приёмку сети. В `c20-workers.json` обе фазы FAIL: curl 35, CONNECT 200, `unexpected_eof`, `core_error_stage` UNKNOWN. В `c20-controls.json` live proxy и direct — PASS, HTTP 204; файл сам помечает, что host TUN мог нести оба пути, и подтверждает тот же listening inode. Снимок `c19-host-after.json` 20:30 UTC сохраняет digest деревьев UPD, routes, resolv.conf и состояния служб относительно 19:58. Дерево из 15 записей, сопоставленное отчётом исполнителя с данными FlClash, digest сменило. C14 поэтому не получает полный PASS неизменности хоста, и смена не приписывается конкретному файлу.

Расхождение source и runtime по rules и DNS в `baseline.json` реально: 62 одинаковых узла при `rules_equal` false и `dns_full_equal` false. Синтез это учитывает.

Чтение `src/main.rs` и `src/common.rs` подтверждает пробел C17a: guard стоит на `install`, fallback каталогов — на общих помощниках путей. Это открытый NO-GO применения, не доказанная уже случившаяся порча данных.

Итог прохода совпадает с baseline: I01 не принят, проектирование I02.T01–T03 допустимо, реализация и следующие направления не открыты. Материального усиления статусов относительно файлов не осталось.
