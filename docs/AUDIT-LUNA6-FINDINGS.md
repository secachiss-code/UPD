# F01–F20: исправления и приёмка

Дата: 2026-10-01. Историческая база: 31d3b7c (2026-09-29).
Архив [BUGLIST.md](BUGLIST.md), B01–B25, не переписывался.
Исходный [DAG](AUDIT-LUNA6-DAG.md) и probe описывают historical baseline,
а этот реестр — текущий результат. «Локально закрыто» означает исправленный
контракт с конечным regression fixture, а не проверку установленной системы.

| F / приоритет | Patch | Tests / result | Состояние |
| --- | --- | --- | --- |
| F01 P1 | D01: постоянный BufReader, bounded framing, явные decode/EOF errors | coalesced Reply+events, byte fragments, malformed/truncated/exact-limit frames passed | Локально закрыто |
| F02 P1 | D04/D05: subscription RAII, socket disconnect, wake/cancel GUI reader | 100 закрытых attach, idle exit, stream cancel; D18 700 attach/drop и точный resource baseline passed | Локально закрыто |
| F03 P1 | D05: byte/count quotas, bounded queues, Partial coalescing, Gap, pre-auth connection/request limits | subscriber/journal/replay/quota guards, slowloris/write; D18 partial burst/slow reader/RSS plateau passed | Локально закрыто |
| F04 P1 | D03/D06: operation_id/prompt_id, одноразовый Input, повторная проверка owner/ID | stale prompt/replay, owner barrier, external fake pkcheck delay, A→B callback storm passed | Локально закрыто; installed polkit не проверен |
| F05 P1 | D03/D04: owned runner/child/reader, rollback, drain, PGID cleared after reap | injected spawn/clone failures, descendant output, cancel/one Exit, old callbacks passed | Локально закрыто |
| F06 P1 | D06: bounded nonblocking PTY input в runner, deadline вне State mutex | full PTY expires, Status/Cancel доступны, input limit/replay/secret tests passed | Локально закрыто |
| F07 P1 | D02: exclusive unique temp/mode, fsync file+directory, cleanup | 16 путей, общий путь, symlink collision, rename/write/fsync failures passed | Локально закрыто |
| F08 P1 | D07: transactional Config merge/flock, validate-before-commit, explicit runtime result | two helpers, 20 CLI/helper fields, CLI/helper VPN YAML, pending/failed apply, wait lock passed | Локально закрыто; real VPN apply остаётся VM-пунктом |
| F09 P1 | D09/D14: single-flight + latest queued, generations/page/query, deadlines/backoff | 100 ticks при 30 s stall, stale query/page, cancelled await retains worker, heartbeat passed | Локально закрыто |
| F10 P1/P2 | D09: I/O внутри spawn_blocking, metadata/job phase/error | worker cancellation/failure/stale tests и suite passed; прямые heavy callbacks заменены | Локально закрыто |
| F11 P1 | D10: current UID direct AUR, root→user, reject foreign UID/root build | fake paru/yay и policy tests passed | Локально закрыто; VM-приёмка снята пользователем, не выполнена |
| F12 P2 | D11: draft/committed, request IDs, busy controls, shared ranges, retain subscription form | deny/stale/unrelated reply/actual saved value/form/ranges tests passed; D17 fixtures passed | Локально закрыто; real polkit UI остаётся VM-пунктом |
| F13 P2 | D12: bounded VecDeque, full available history, follow-tail, honest phases/progress, terminal route | 6000 строк, scroll/unread, error progress, double answer/retry, cancel failure, startup route; D17 focus/Enter passed | Локально закрыто |
| F14 P1/P2 | D13: full UTF-8 decode loop, EOF flush, bounded partial | every split/invalid prefix/tail, ANSI/OSC/CR tests passed | Локально закрыто |
| F15 P2 | D13: tabs/Unicode единый byte cap, bounded stages/history | 1M tabs и 100k stages, alternate screen passed | Локально закрыто |
| F16 P2 | D15: bounded shared reaper, launch/exit errors, safe URL schemes | 100 short children reaped, long child nonblocking, missing/permission launcher and URL tests passed | Локально закрыто |
| F17 P1/P2 | D08: explicit trusted UserContext в inline/CLI/sysproxy, no global env mutation | two UIDs, ambient env isolation, peer identity, fake dconf failure passed | Локально закрыто; VM-приёмка снята пользователем, не выполнена |
| F18 P2 | D16: fresh manifest/staging, targets/versions/--locked, failure logs | 4 build/package fixture tests: stale GUI excluded, enabled, missing libs, failing build passed | Локально закрыто; реальный пакетный install остаётся VM-пунктом |
| F19 P1/P2 | D14: poll both pipes, deadline/cancel/output budget, process group + RAII reap | sleep/overflow stderr/stdout/descendant/cancel/setup failure passed | Локально закрыто |
| F20 P2 | D17: popup scroll, bounded alerts/question, controls/footer retain space | 143 valid PNG, saved checked before/after, operation focus/Enter bounds passed | Локально закрыто; compositor-приёмка снята пользователем, не выполнена |

## Незавершённые критерии

По указанию пользователя от 2026-10-01 обязательная внешняя приёмка снята
как условие сборки и коммита. Disposable Arch VM и Wayland interactive
acceptance не выполнены: F11/F17 (реальный AUR и пользовательский proxy),
F20 (compositor). Также не выполнены сквозные сценарии D19: open/reopen GUI при операции, реальный
denied polkit, prompt/answer, отказ VPN apply, внешний CLI параллельно GUI,
реальное fresh/stale packaging в VM. Их mock/headless части проверены,
но установленная policy и реальный systemd не подменяются этим результатом.
Полный [ручной чеклист](audit-luna6-acceptance/README.md).

Протокольные пары old GUI/new helper и new GUI/old helper проверены fixtures:
new client останавливается на старом Hello до mutation; new server отклоняет
Envelope без версии/legacy Start/Input до изменения состояния. Пользователю
возвращается protocol mismatch с указанием версий и необходимости обновить пару.
Небезопасного fallback нет.

## Воспроизведение

```sh
./tests/check_audit.sh
UPD_FULL_VISUAL=1 UPD_SNAPSHOTS=/tmp/upd-final-screens ./tests/check_audit.sh
```

Core: musl, Cosmic: GNU, --locked --offline. IPC fixtures требуют разрешённых
локальных Unix sockets/PTY. Реальные VPN/прокси/systemd/пакеты не изменяются.
Последние полные suites после prior-work review: 149 lib + 41 CLI + 24 integration; 24 Cosmic;
4 packaging fixtures. Матрица 143 PNG прошла отдельно. Сводка ресурсов:
[RESOURCE-GATES.md](audit-luna6-acceptance/RESOURCE-GATES.md).

Последующее поручение пользователя выполнено: [ревью исходной базы и четыре дополнительных исправления](AUDIT-LUNA6-PRIOR-REVIEW.md). Общий gate повторён после них и прошёл.
