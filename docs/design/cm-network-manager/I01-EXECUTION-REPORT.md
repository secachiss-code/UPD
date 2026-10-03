# I01: отчёт отдельного исполнителя

Дата исполнения: 2026-10-03 UTC. Scope: [контракт C01–C19](I01-TEST-CONTRACT.md), [parity protocol](I01-PARITY-PROTOCOL.md), [решения](I01-DECISIONS.md). Первые проверки исполнил отдельный Sol executor; продолжение C19 и консолидацию — отдельный Astra executor. Координатор тесты не выполнял. Ниже прежние запуски обозначают сохранённое evidence этих запусков, а не повторное исполнение Astra. Пакет, установка на хост и миграция живых данных не выполнялись.

I01 целиком не принят: автоматическая миграция заблокирована guard, full transaction/recovery не проверены; network pilots не доказывают работоспособность host TUN или преимущество ядра. Product code исполнитель продолжения не менял.

## Версии и привязка к исходникам

HEAD: `07ef2381fb962fe74958c504a896c584f6a4e4be`, dirty source CM 0.2.8. HEAD недостаточен: первоначальный [source manifest](i01-evidence/source-manifest.json) имеет tree SHA-256 `1351a458c8e46f0bdcc2a380c0ee31b19d13a1fa36d3a7d7a188a647170a0599`; после C17 и C18 исходники изменялись. Каждый `final-*-run.json` фиксирует полный собственный before/after manifest и digests compiled executables.

| Исполняемое состояние | Версия / SHA-256 | Evidence |
|---|---|---|
| Установленный UPD | 0.2.7 / `58520fbe70796d5bd018d56ad4ebbb120cea92c8edcb2cb52f5983ea694b14c2` | [baseline](i01-evidence/baseline.json), [versions](i01-evidence/final-journals.json) |
| Установленный native Mihomo, также копия A | v1.19.32 / `98f50f0fdc498a9be1aa65ee991cda1c5854650d948e9001d7e13b703ab6cc3d` | [copy](i01-evidence/native-core-copy.json) |
| FlClash package / FlClashCore B | package 0.8.98-1, private core API 1.10.0 / `b5014e62eda428795056221ce304990be30512f821d803a3b4bd8b0abafa1059` | [versions](i01-evidence/final-journals.json), [private core](i01-evidence/c19-flclash-private-config-probe.json) |
| Тестовый CM CLI после C18 | `22b00cb906e430ea689faeea4f09e0641a2f6e9e073ba5ef71ec89435dc695d4` | [final bin run](i01-evidence/final-bin-run.json) |
| Dirty source после C18 | `7332368fe4eccb26901cc9215ef97f8fe2031133974edb76d1989983313730c0` | [final bin manifest](i01-evidence/final-bin-run.json) |

CM не установлен. UPD VPN/helper inactive; живой FlClashCore работает с root UID. Native A — копия установленного ядра UPD, не новое ядро, собранное из CM source. API version B и release tag не доказывают исходный commit bundled core. Matching v0.8.98 использован для четырёхбайтового little-endian JSON IPC только собственных workers.

## Результат по контракту

| ID | Статус | Реально проверено и граница |
|---|---|---|
| C01 | PASS | Root inventory source/installed binaries, отсутствие CM, process ownership, состояния служб и API availability. |
| C02 | PASS | Полное локальное сравнение credentials-inclusive node definitions; только counts/field names/digests экспортированы. |
| C03 | INCONCLUSIVE | Журналы реально классифицированы; историческая config generation unknown, причинность не установлена. |
| C04 | BLOCKED | UPD inactive; live Fl REST GET discovery не дал resolver chain. Файловый selection не выдан за live selection. |
| C05 | PASS, proxy scope | Одинаковый полный WS node, минимальный config, native validate и actual own Fl init/setup/startListener в offline namespace. C19 отдельно повторил prerequisites для другого транспорта. |
| C06 | FAIL connectivity; INCONCLUSIVE performance | Единственный WS pilot: A1 4/10, B 2/10, A2 0/10, все 24 measured failures curl 35. Повторного WS pilot нет. |
| C07 | BLOCKED | Full migration old layout не реализована/не исполнена; C17 отклоняет legacy install. |
| C08 | PASS только отказ | Actual production preflight и CLI old/new conflict отказывают до записей/команд. Успешный migration path отсутствует. |
| C09 | BLOCKED | Нет проверенной persisted transaction/recovery и fault injection полного install boundary. |
| C10 | BLOCKED полный scope | Guard fixtures сохраняют foreign sentinels при отказе; ownership-safe перенос и subsequent fresh CM target handling не проверены. |
| C11 | NOT_RUN migration | Local env/guard assertions проходят; idempotency фактически выполняемой миграции не проверена. |
| C12 | PASS local | 175 library tests; актуальная C18 i18n subset 8/8. Fixtures не доказывают remote VPN. |
| C13 | PASS local | 56 binary, 24 helper/backend integration, 15 COSMIC, launcher и private VPN preparation checks. |
| C14 | PASS в измеренном scope | Root before/after выбранных host assets и cleanup собственных workers; ограничения покрытия описаны ниже. |
| C15 | BLOCKED полный scope | Helper socket presence fail-closed проверен; actual busy/update operation locks и migration concurrency/recovery не проверены. |
| C16 | BLOCKED полный scope | Known legacy entrypoints блокируют install; реальный переход CLI/desktop/applet/cron и arbitrary user home references не исполнены. |
| C17 | PASS guard scope | 9 actual production-code integration tests + 9 actual CLI disposable cases; zero fixture writes, no external mutating commands. |
| C18 | PASS | Actual `help`/`--help` в ru/en/de/it/zh/ar: 12 cases, private config unchanged, localized notice; i18n 8 tests. |
| C19 | FAIL connectivity; INCONCLUSIVE cause/performance | Новый TCP/REALITY experiment и четыре live proxy controls; пределы причинности/performance сохранены. |
| DC01 | См. последний document-check | Markdown links/fences, DAG counts/acyclicity, overlays и authorization проверяет исполнитель после последнего обновления документов координатором. |

## Baseline, расхождения конфигураций и исторические ошибки

Root [baseline](i01-evidence/baseline.json): 19:01:49–19:01:51 UTC. UPD source/runtime: 62/62 полных узла одинаковы. UPD/Fl saved: 62 общих имени, только 1 полное совпадение; изменены `reality-opts` у 55, `servername` у 49, `server` у 18, `ws-opts` у 5. Global DNS/rules также различаются. Поэтому исходные приложения не составляли same-config comparison.

[Final journals](i01-evidence/final-journals.json): 18 681 UPD events, 17 518 classified error events, 2026-10-01 19:17:45 — 2026-10-03 14:45:18 UTC. Классы не взаимоисключающие: DNS 1 234, dial 14 574, WS 26, HTTP 11 274, HTTP 502 11 214, timeout 2 054, TUN 302, unknown 2 649; TLS/REALITY textual matches 0. Это не доказывает отсутствие TLS ошибок: classifier текстовый. Более ранний расширенный classifier пропустил bare `ws`; его evidence сохранено, поздний regex учитывает его. Contemporary historical config отсутствует.

[Live API discovery](i01-evidence/flclash-api.json): own-process socket discovery выполнено read-only, Unix API unavailable, owned loopback GET вернул HTTP 400. Никаких mutating requests к live desktop IPC не было. Private workers имеют доказанный fixed selection, что не раскрывает live selected node.

## Локальные тесты, команды и первые неудачи

Сохранённые реальные команды с UTC, target, revisions, argv и executable digests: [root](i01-evidence/final-root-run.json), [guard](i01-evidence/final-guard-run.json), [bin](i01-evidence/final-bin-run.json), [i18n](i01-evidence/final-i18n-run.json), [COSMIC](i01-evidence/final-cosmic-run.json). `root` в имени отчёта означает корневой Cargo project: uid этих тестов 1000, не root privileges.

```sh
python3 /tmp/cm-i01-test-runner.py root
python3 /tmp/cm-i01-test-runner.py guard
python3 /tmp/cm-i01-test-runner.py cosmic
python3 /tmp/cm-i01-test-runner.py bin
python3 /tmp/cm-i01-test-runner.py i18n
python3 /tmp/cm-i01-cli-guard.py
python3 tests/test_i01_help.py
```

Runner выполняет сначала `cargo test --locked --offline --target ... --no-run`, затем тот же selected suite с `-- --test-threads=1`. Root suite: `--lib --bin cm --test audit_contracts --test audit_i01_install_guard`, target `x86_64-unknown-linux-musl`; COSMIC из `cosmic/`, target `x86_64-unknown-linux-gnu`. Final root 19:13:17–19:14:14: 175+56+24+8 PASS; после добавления отдельного permission case final guard 19:16:51–19:16:52: 9 PASS. Final bin 19:32:43–19:32:47: 56 PASS; i18n 19:32:52–19:32:53: 8 PASS. COSMIC 19:15:16–19:15:53: 15 PASS. Assertions/counts читаются из соответствующих `*-tests.log`, а не выводятся из exit code одного runner.

[CLI guard](i01-evidence/cli-install-guard.json) 19:32:52: old config, simultaneous old/new config, old data, dangling config symlink, foreign unit, enablement link, helper socket, cron, desktop. Actual binary запускается через bwrap с disposable `/etc`, `/var/lib`, `/run`, `/usr/local`; synthetic command sentinels фиксируют любую попытку внешнего mutating command. CM/UPD environment diversion специально задана и не обходит production root guard. Сравниваются полное fixture содержимое и permissions.

[Help](i01-evidence/help-locales.json) 19:32:12: 12 actual CLI cases; fixture `CM_CONF`, выбранный lang и команды-сентинелы. [Launcher log](i01-evidence/tui-launcher.log) и [VPN preparation log](i01-evidence/vpn-prepare-bwrap.log) фиксируют PASS. Их старые отдельные wrapper argv/time/digests не записаны столь полно, как final Cargo suites; команды воспроизводятся соответствующими scripts `tests/test_tui_launcher.py` и `tests/check_vpn_sandbox.py`, но из коротких логов нельзя восстановить точный shell invocation.

Первые отказы не удалены:

- [cargo-lib.log](i01-evidence/cargo-lib.log): 169 PASS, 6 FAIL в sandbox (Unix socket/network `Operation not permitted`, timeout и связанные deadline assertions). [Повтор с разрешённым local IPC](i01-evidence/cargo-lib-unrestricted.log) и final root дают 175 PASS. Это отдельные фактические запуски; первичные ошибки не превращены в PASS.
- [c18-bin-unit.log](i01-evidence/c18-bin-unit.log): 55 PASS, 1 FAIL `install_reloads_helper_and_reports_restart_failure`, `PermissionDenied/Operation not permitted`; final bin после разрешения local IPC 56 PASS.
- [CLI first attempt](i01-evidence/cli-install-guard-first-attempt.json): все 9 FAIL, fixture bytes/permissions unchanged, expected guard output отсутствовал. Это setup/harness failure, не evidence guard refusal. По handoff прежнего исполнителя причинами исправлений были read-only mount/work path; JSON сам точную причину не сохраняет. Последний actual CLI run PASS хранится отдельно.
- [Fl standalone -h attempt](i01-evidence/flclash-isolated-cli-probe.json): exit 2, recognized help markers false. Это несовместимый desktop IPC bootstrap, не транспортный VPN fail. Затем actual own RPC bootstrap/config прошли.

Отдельный [первый запуск консолидации](i01-evidence/consolidation-first-attempt.json) завершился JSONDecodeError: after snapshot файл уже создан shell redirection, но root read ещё ожидал GUI. Это обработка отчёта без новых requests/tests; финальная консолидация ожидает непустой JSON.

`tests/check_audit.sh` целиком, `test_build_artifacts.py`, `verify_real_packages.py`, package/build/install commands не выполнялись: транзитивно достигают package workflow. Offline Cargo dependency build.rs допустимы контрактом. Unit mocks внешних команд не являются full migration executor.

## C06: прежний WS pilot

[Observations](i01-evidence/pilot.json), [summary](i01-evidence/pilot-summary.json): 19:39:17–19:41:59 UTC, один полный WS node, один общий config, fresh core/private home на каждой фазе. A1/B/A2 по 10 measured и одному warm-up, всего 33 requests, concurrency 1, curl timeout 10 s.

| Фаза | Measured successes | Warm-up | Median successful latency |
|---|---:|---|---:|
| A1 native | 4/10 | FAIL | 4860 ms |
| B Fl | 2/10 | PASS | 3748 ms |
| A2 native | 0/10 | FAIL | — |

Все 24 measured failures curl exit 35. Это TLS symptom curl через HTTP CONNECT, не доказанная ошибка WS handshake самого core. Выборочная latency только успешных запросов даёт survivor bias; winner/p95 не объявляются. Все workers разделяли host network namespace; активный Fl TUN мог нести outer traffic. C06 не повторялся.

## C19: TCP/REALITY и live controls

Prerequisites действительно исполнены offline: [Fl](i01-evidence/c19-flclash-private-config-probe.json) 19:57:13–14, [native](i01-evidence/c19-native-private-config-probe.json) 19:57:22. Полный node выбран детерминированно первым в порядке локального Fl source: VLESS, TCP/default TCP, tls=true, REALITY public-key present. Никакие credentials/transport fields не преобразовывались. Source generation SHA-256 `c3b6c713edb056b91b81c3cc54c7d6e534a3e4f0fcded6e4fd164e4358f802e7`, mtime 14:45:40 UTC; node SHA-256 `26bb6fda04b54182c8e5489fa5819fb87c02034512def88816a3ccdf66dff523`; общий config SHA-256 `cf43811a7e3cf738da53bbdfcec10dedc71dbc6612207899ab8aa2efa71584cb`.

Global config построен заново одинаково для обеих сторон: один node, fixed select group и MATCH; ipv6/allow-lan false, bind127.0.0.1, mixed48123, другие inbound ports0, TUN/auto-route/auto-redirect false, providers/listeners/tunnels пустые, geo-auto-update/NTP/write-to-system false; DNS redir-host без listener, явный 1.1.1.1, system hosts false; store-selected/store-fake-ip false. Это отличается от полного Fl/UPD application config и не проверяет все effective engine defaults. Native validate exit0, Fl actual validate/init/setup/startListener и raw node equality подтверждены; REST fixed selection и доступные общие fields совпали.

Workers работают в user/pid/mount namespaces, CAP all dropped, readonly host filesystem, writable private `/mnt`, private `/tmp` и `/dev` без TUN. Offline подготовка использует private net namespace; pilot использует общий host net namespace. Перед запросами фактически проверяются собственный listening socket, fixed selection, no-TUN и нулевые capabilities. Worker config не экспортируется.

[Root ownership before](i01-evidence/c19-host-before.json) доказывает PID43845, root FlClashCore binary digest, fd→inode275973→LISTEN127.0.0.1:7890 и совпадение saved mixed-port. Controls перед каждым запросом повторно проверяют тот же socket inode. Действующий config/groups/connections через API не менялись.

[Controls](i01-evidence/c19-controls.json) 19:59:25–19:59:37: **0/4 PASS**, все curl35/HTTP000, по два на gstatic и Cloudflare trace. Следовательно предположение «working live proxy» этим опытом не подтверждено. Нельзя распространить этот результат на все приложения, все сайты или весь VPN.

Команды C19, выполненные последовательно, с собственной временной конфигурацией:

```sh
python3 /tmp/cm-i01-c19-fl-config-probe.py
python3 /tmp/cm-i01-c19-native-config-probe.py
SUDO_ASKPASS=/tmp/cm-sudo-askpass.sh sudo -A /usr/bin/python3 /tmp/cm-i01-c19-root-read.py
python3 /tmp/cm-i01-c19-controls.py
python3 /tmp/cm-i01-c19-pilot.py
```

Root read stdout сохранён в before/after JSON. Read-only root выполняет только inventory/socket discovery; сами network cores/curl запускаются uid1000 без CAP. Для всех запросов TLS certificate verification curl по умолчанию, без insecure option; тело всегда `/dev/null`, Cloudflare trace body не экспортируется. Targets: `https://www.gstatic.com/generate_204` (HTTP204, zero body) и `https://www.cloudflare.com/cdn-cgi/trace` (HTTP200). В каждой фазе пять measured на каждый target, warm-up gstatic. Curl measured values — собственные `time_total`/`time_starttransfer` и wall elapsed, не инструментирование внутренних handshake stages ядра.

[Pilot observations](i01-evidence/c19-pilot.json): 19:59:58–20:02:44 UTC, A1/B/A2 **0/10, 0/10, 0/10**, warm-up **0/3**. В каждой фазе ровно пять measured на каждый target; всего C19 **30 measured + 3 warm-up + 4 controls = 37**. От первого control до окончания pilot прошло около 200 s, меньше 15 min. Все 37 неуспехов имеют curl exit35 и HTTP000; это не curl timeout28. Median wall elapsed measured failures: A1 5012 ms, B 5012 ms, A2 5015 ms; successful latency отсутствует. Performance и причина общего отказа **INCONCLUSIVE**, connectivity этого experiment **FAIL**.

[Пассивный audit curl](i01-evidence/c19-curl-passive-audit.json), без новых requests: curl8.22.0/OpenSSL3.6.5; `~/.curlrc` и default XDG curlrc отсутствуют, CURL_HOME/XDG_CONFIG_HOME и перечисленные proxy/CA/SSL override variables отсутствуют. Drivers не указывали `--disable/-q`, controls наследовали окружение, pilot удалял proxy env suffixes. Post-run audit не обнаружил конкретной помехи; это не contemporaneous snapshot environment каждого subprocess. Raw curl stderr был классифицирован и отброшен; CONNECT response code и точная OpenSSL причина не записаны. Поэтому curl35 обозначает только TLS symptom клиента через проверенный proxy, а не доказанный отказ REALITY. Дополнительных запросов для уточнения не делалось.

## C14: cleanup и полнота снимков

Прежние root [before](i01-evidence/baseline.json) и [after](i01-evidence/final-host-baseline.json) непустые, uid0; 19:01:49 и 19:42:56 UTC. Все 11 snapshot entries равны: `/etc/upd/vpn`, `/etc/cm/vpn`, `/var/lib/upd/vpn`, `/var/lib/cm/vpn`, FlClash data tree, маршруты, resolv.conf, четыре UPD/CM VPN/helper service states. UPD trees читаются с root: 3 и 13 entries, errors0. Fl tree 15 entries, errors0. Unprivileged initial snapshot имел недостаточное покрытие и не использован для PASS защищённых directories.

Это проверка выбранных assets, не полный snapshot всей ОС: отдельные произвольные unit files вне этих деревьев, firewall и все mutable host files полностью не инвентаризированы. Права sandbox запрещали worker writes вне собственной области; read-only host commands не меняют services/routes/DNS. В финальных journals есть реальные события, а не пустой успешный stdout.

<!-- C14_FINAL_RESULTS -->

## Воспроизводимость и оставшиеся границы

[Execution consolidation](i01-evidence/execution-consolidation.json) содержит digests evidence logs/harness, assertions current source vs final manifest, сохранённые first failures, C14 comparisons и C19 summaries. [Копии безопасных harness scripts](i01-evidence/harness-manifest.json) сохранены в evidence/harness с digests. Временные scripts/native binary сохранены для review; их удаление согласует координатор. Секретные temporary worker configs удалены вместе с private directories; retained scripts не содержат профилей, URLs подписок или credentials.

Нужны отдельные последующие рубежи: full migration transaction/recovery/ownership/legacy operation locking, active selected node provenance, controlled host TUN test и причинная локализация network failure. I02–I18 этим отчётом не разрешены. До нового bounded contract дополнительных external requests не выполняется.

## Дополнение 2026-10-03: синтез после C20

Этот раздел написан после отчёта исполнителя. Исполнитель C20 здесь повторно не запускался. Каноническая сводка чисел, разделение четырёх утверждений, проект миграции и вердикт G0: [I01-BASELINE-REPORT.md](I01-BASELINE-REPORT.md) и [I01-MIGRATION-DESIGN.md](I01-MIGRATION-DESIGN.md).

C20 уже лежит в evidence и в таблицу C01–C19 выше не вставлялся, чтобы прежний прогон не выглядел так, будто он с самого начала содержал эти поля. Кратко по файлам 20:16 UTC: собственные native и FlClash worker на узле C19 — FAIL, curl 35, CONNECT 200, `unexpected_eof`, стадия ядра UNKNOWN. Живой proxy и запрос без явного proxy на gstatic — HTTP 204. Это не отмена C19 0/4 и не приёмка host TUN. Новый удалённый контракт не добавлен. Уточнения будущей миграции C10a, C16a и C17a в контракте имеют статус NOT_RUN. I01 целиком по-прежнему не принят.
