#!/usr/bin/env python3
"""Источник детального DAG оставшейся работы CM (с 2026-10-06).

Единственное место, где редактируются подзадачи. Скрипт проверяет ссылки и ацикличность,
считает волны готовности и критический путь, затем пишет:
  ../REMAINING-DAG-PLAN-2026-10-06.md
  ../REMAINING-DAG-PLAN.json
Запуск: python3 docs/design/cm-network-manager/tools/remaining_dag.py [--check]
"""

import json
import sys
from pathlib import Path

DATE = "2026-10-06"
HERE = Path(__file__).resolve().parent
OUT_MD = HERE.parent / "REMAINING-DAG-PLAN-2026-10-06.md"
OUT_JSON = HERE.parent / "REMAINING-DAG-PLAN.json"

# Относительный вес размера: S ≤ 0.5 дня, M 1–2 дня, L 3–5 дней, XL > недели (разбить при старте).
SIZE = {"S": 1, "M": 2, "L": 4, "XL": 8}
# Уровни доказательства из DETAILED-DAG-PLAN §7.
LEVEL = {
    "D": "решение пользователя/ADR",
    "L0": "чтение первичных источников, документ",
    "L1": "unit/fixture, compile",
    "L2": "Linux network harness (netns, локальный test endpoint)",
    "L3": "реальное приложение/executor",
    "L4": "сквозная проверка: fault injection, миграция, установка",
}

# Кодово завершённые задачи (код/compile/unit). Runtime-приёмка у них отложена до X-трека.
DONE = [
    "I01.T01", "I01.T02", "I01.T03", "I01.T04", "I01.T05",
    "I02.T01", "I02.T02", "I02.T03", "I02.T04", "I02.T05",
    "I03.T01", "I03.T02", "I03.T03",
    "I03.T04.p",  # решение D1 принято 2026-10-06, I03-DECISIONS-2026-10-06.md
    # Блок 1 (Composer 2.5 + Grok 4.7), ревью и доводка координатором 2026-10-06:
    # REVIEW-2026-10-06-BLOCK1.md. H.04 реализован до формального D4 — подтвердить.
    "H.01", "H.02", "H.04", "H.05", "I03.T04.a", "I03.T04.b", "I03.T04.y",
    # Блок 2: Composer (.c–.r) + координатор (.s–.w), REVIEW-2026-10-06-BLOCK2.md.
    # .x документирован (I03-PARSER-FINAL.md), закрытие ждёт H.05.b.
    "I03.T04.c", "I03.T04.d", "I03.T04.e", "I03.T04.f", "I03.T04.g", "I03.T04.h",
    "I03.T04.i", "I03.T04.j", "I03.T04.k", "I03.T04.l", "I03.T04.m", "I03.T04.n",
    "I03.T04.o", "I03.T04.q", "I03.T04.r", "I03.T04.s", "I03.T04.t", "I03.T04.u",
    "I03.T04.v", "I03.T04.w",
    # Блок 3, часть 1 (Grok + доводка координатором): REVIEW-2026-10-06-BLOCK3-PART1.md.
    "H.12", "I03.T05.b",
    # Блок 3, часть 2 (Grok + доводка координатором): REVIEW-2026-10-06-BLOCK3-PART2.md.
    "I03.T05.c", "H.05.b", "I03.T04.x", "I04.T01.a", "I04.T01.b", "I04.T01.c",
    "I04.T02.a", "I04.T02.b", "I04.T02.c", "I04.T03.a", "I17-D.T01.a",
    # Решения D6–D10, Q01 (DECISIONS-D6-D10-Q01-2026-10-06.md).
    "V.01", "H.08", "I15-R.T01.a", "X.01",
]

MILESTONES = []  # (id, title, priority, lane, deps_text, goal, questions, gate)
TASKS = []       # dicts


def M(mid, title, prio, lane, goal, questions="", gate=""):
    MILESTONES.append(dict(id=mid, title=title, priority=prio, lane=lane, goal=goal,
                           questions=questions, gate=gate))


def T(tid, title, size, level, deps, what, out, done, decision=None):
    parent = tid.rsplit(".", 1)[0]
    full = []
    for d in deps:
        if d == "^":  # предыдущая подзадача того же рубежа
            prev = [t["id"] for t in TASKS if t["milestone"] == tid.split(".")[0]]
            full.append(prev[-1])
            continue
        # короткая ссылка «c» означает соседнюю подзадачу того же родителя
        full.append(f"{parent}.{d}" if "." not in d and d.islower() and len(d) <= 2 else d)
    TASKS.append(dict(id=tid, milestone=tid.split(".")[0], title=title, size=size, level=level,
                      depends_on=full, what=what, outputs=out, done_when=done,
                      decision=decision))


# ───────────────────────────── H: гигиена и инфраструктура ─────────────────────────────
M("H", "Гигиена кода и инфраструктура проверок", "P0", "infra",
  "Снять технические блокеры до роста кода: HTTP-транспорт с честным deadline, судьба YAML, "
  "декомпозиция крупных модулей, сетевой harness, актуальные статусы evidence.",
  gate="H.04 и H.11 блокируют реальные fetch и все L2-проверки")

T("H.01", "Обновить статусы evidence после локального прогона", "S", "L1", [],
  "2026-10-06 `cargo test --offline --locked` прошёл: 372 теста, 0 отказов. Записать прогон с "
  "revision, rustc, хешем Cargo.lock. Разделить в статусах «compile/unit PASS» и «installation runtime NOT_RUN».",
  "i03-evidence/local-tests-2026-10-06/summary.json; правки WORK-STOP, CODER-QUEUE, DAG-PLAN.json",
  "Ни один документ не утверждает NOT_RUN для фактически выполненных unit/fixture проверок; runtime-статусы не повышены.")
T("H.02", "Расширить локальный gate", "S", "L1", [],
  "В `tests/check_audit.sh`: `cargo clippy --all-targets -- -D warnings` для новых модулей "
  "(`src/sources`, `src/profiles`, `src/migration`), `rustfmt --check` для них же, отдельный шаг для `cosmic`.",
  "tests/check_audit.sh", "Gate падает на новом предупреждении в новых модулях; старые модули пока в allowlist.")
T("H.03", "Убрать предупреждения clippy (~100)", "M", "L1", ["H.02"],
  "54× collapsible_if, лишние касты, field_reassign_with_default, too_many_arguments (ввести структуры параметров), "
  "items_after_test_module. Отдельные коммиты по файлам, без изменения поведения.",
  "чистый `cargo clippy --all-targets`", "0 предупреждений; все 372 теста проходят; diff не меняет логику (ревью по файлу).")
T("H.04", "HTTP-транспорт с общим deadline (ADR + реализация)", "L", "L1", [],
  "Известный блокер: ureq 2.12 не ограничивает DNS lookup таймаутом запроса. Сравнить: ureq 3 с собственным "
  "Resolver; резолв в отдельном потоке с отменой по deadline; minreq/свой клиент на rustls. Реализовать "
  "`FetchTransport` для `sources::negotiation`: общий deadline (DNS+connect+TLS+body), лимит тела, redirect-политика "
  "(лимит, запрет https→http, повторная проверка лимитов), опциональный локальный proxy FlClash по прежним правилам.",
  "ADR-HTTP-TRANSPORT.md; src/sources/transport.rs; тесты на локальном сервере 127.0.0.1",
  "Медленный DNS/slow-loris/бесконечный redirect/большое тело завершаются в пределах budget с типизированной ошибкой; "
  "секретный URL не попадает в ошибки и логи.", decision="D4")
T("H.05", "Решение по YAML-стеку", "M", "L0", [],
  "serde_yaml 0.9 больше не сопровождается; yaml_guard ходит в FFI `unsafe_libyaml =0.2.11`. Варианты: оставить "
  "с pin и аудитом; serde_yaml_ng/serde_norway; парсер на saphyr с собственным guard. Критерии: duplicate keys, "
  "лимиты, отсутствие alias expansion, поддержка, unsafe.",
  "ADR-YAML.md; при смене — миграция parser/native и yaml_guard", "Решение записано; все fixtures native/guard проходят на выбранном стеке.",
  decision="D5")
T("H.05.b", "Миграция пути импорта на saphyr (D5)", "M", "L1", ["H.05"],
  "По ADR-YAML-draft: разбор событий `saphyr_parser::Parser::next_event` с собственным guard (дубли ключей, алиасы, теги, "
  "лимиты) вместо `serde_yaml` + FFI `unsafe_libyaml`; `Yaml::load_from_str` не использовать. `src/vpn.rs` остаётся на "
  "serde_yaml до H.06. Крейта нет в локальном кэше: нужен разовый `cargo fetch` с разрешения пользователя.",
  "src/sources/parser/native/common.rs, yaml_guard.rs; Cargo.toml/lock (fetch разрешён 2026-10-06)", "4 guard + 3 YAML native fixtures проходят на "
  "saphyr; `unsafe_libyaml` не используется путём импорта.")
T("H.06", "Декомпозиция vpn.rs (3807 строк)", "L", "L1", ["H.03"],
  "Без изменения поведения разнести на модули: `vpn/subs.rs` (Sub/Subs, хранение), `vpn/fetch.rs` (ureq, UA), "
  "`vpn/profile_files.rs` (stage/install/rollback), `vpn/config.rs` (генерация YAML, budget), `vpn/state.rs` "
  "(VpnState, last failure), `vpn/conflicts.rs` (FlClash/чужие TUN), `vpn/core.rs` (ядро, geo).",
  "src/vpn/*.rs", "Публичный API для main/tui/helper не изменился; тесты проходят; ни один файл не длиннее ~900 строк.")
T("H.07", "Декомпозиция helper.rs (2842 строки)", "L", "L1", ["H.03"],
  "Выделить `helper/protocol.rs` (Request/Envelope/Reply/Event), `helper/auth.rs` (Peer, polkit), "
  "`helper/ops.rs` (runner, PTY, очередь событий), `helper/settings.rs`. Это подготовка к I06.",
  "src/helper/*.rs", "Протокол байт-в-байт совместим (тест сериализации старых сообщений); тесты проходят.")
T("H.08", "Политика evidence в git", "S", "D", [],
  "Сейчас в git 148 файлов логов evidence. Предложение: в git только summary.json с хешами и командой, сырые логи — "
  "в каталоге вне репозитория или в архиве релиза.",
  "правило в MODEL-WORKFLOW.md; .gitignore", "Решение пользователя записано и применяется к новым evidence.", decision="D8")
T("H.12", "PTY без CLOEXEC в helper и TUI", "S", "L1", [],
  "`openpty` в `src/helper.rs` (spawn_pty) и `src/tui/process.rs` создаёт master и slave без `O_CLOEXEC`; флаг ставится позже "
  "и только на master, исходный slave остаётся наследуемым. Fork/exec в другом потоке в это окно уносит копию PTY в чужой "
  "процесс, и долгоживущий потомок держит терминал: reader не получает EOF. Сразу после `openpty` ставить `FD_CLOEXEC` на оба "
  "fd (stdio потомка ставится через dup2, флаг не мешает) или открывать через `posix_openpt(O_CLOEXEC)`.",
  "src/helper.rs, src/tui/process.rs; тест", "Тест: параллельный spawn во время открытия PTY не получает PTY-fd (проверка /proc/<pid>/fd потомка).")
T("H.09", "Аудит unsafe", "M", "L1", ["H.06", "H.07", "H.12"],
  "~190 вхождений `unsafe` (helper 48, store 24, common 24, tui/process 21). Каждому блоку — комментарий SAFETY; "
  "`#![deny(unsafe_op_in_unsafe_fn)]` и `clippy::undocumented_unsafe_blocks` для src/sources, src/profiles, src/helper; "
  "по возможности заменить libc-вызовы безопасными обёртками (rustix/nix).",
  "UNSAFE-AUDIT.md; правки кода", "Нет unsafe-блока без обоснования; lint включён в gate H.02.")
T("H.10", "CI на GitHub (опционально)", "M", "D", ["H.02"],
  "Workflow: build musl, test, clippy, rustfmt новых модулей, сборка cosmic. Без установки и сети хоста. "
  "Внешнее действие — только по решению пользователя.",
  ".github/workflows/check.yml", "Зелёный прогон на ветке; секреты не нужны.", decision="D9")
T("H.11", "Сетевой test harness", "XL", "L2", ["H.04"],
  "Rootless user+net namespace (unshare) или VM: veth-пары, «внешний» namespace с тестовыми серверами "
  "(mihomo/xray inbound VLESS/VMess/Trojan/SS/Hy2/TUIC/WG, DNS-сервер, HTTP subscription server), tcpdump-capture "
  "на каждом интерфейсе, сценарии отказов (drop, delay, link down). Прошлый W0-1 упёрся в запрет NETLINK_ROUTE "
  "в песочнице — нужно согласовать среду.",
  "tests/netharness/ (скрипты + Rust-обвязка), README-NETHARNESS.md",
  "Один сценарий «клиент → worker → тестовый сервер» проходит с capture; среда воспроизводима одной командой.",
  decision="D6")

# ───────────────────────────── I03: подписки и свои серверы ─────────────────────────────
M("I03", "Подписки и собственные серверы (остаток T04–T05)", "P1", "import",
  "Довести импорт до полного заявленного subset без silent dropping и закрыть итерацию evidence.",
  questions="Q04, Q06, Q24", gate="Неподдержанное → UnsupportedFeature, а не молчаливое удаление")

T("I03.T04.a", "Разбить parser/native.rs на модули", "S", "L1", [],
  "849 строк вырастут втрое. Разнести: `native/mod.rs` (вход, лимиты), `native/common.rs` (строгий visitor, "
  "строки/числа/enum-наборы), `native/tls.rs`, `native/transport.rs`, `native/proto/{vless,vmess,ss,trojan,hy2,tuic,wg,http,socks}.rs`.",
  "src/sources/parser/native/*", "Поведение не изменилось: 9 native + 4 guard + 3 pipeline теста проходят.")
T("I03.T04.b", "Общий валидатор вложенных опций", "M", "L1", ["a"],
  "Типизированный builder вложенных map: строгий набор ключей, отказ на неизвестный ключ без эха имени, ограниченные "
  "строки (длина, управляющие символы), списки с лимитом, enum-наборы, взаимоисключающие и парные поля. "
  "Результат сохраняется в full_definition без потерь, digest стабилен (каноническая сортировка).",
  "native/common.rs", "Fixture: неизвестный вложенный ключ, дубль, превышение длины, неверный тип → фиксированные коды ошибок.")
T("I03.T04.c", "TLS-опции", "L", "L1", ["b"],
  "По pinned v1.19.32: `tls`, `servername`/`sni`, `alpn`, `client-fingerprint` (закрытый набор значений), "
  "`fingerprint` (pin сертификата, формат hex sha256), `skip-cert-verify` (политика D2), `certificate`/`private-key` "
  "(пути хоста — отказ), `ech-opts` → UnsupportedFeature.",
  "native/tls.rs; fixtures tls_*", "Каждое поле: позитивный и негативный fixture; ссылка на строку первичного источника в doc.",
  decision="D2")
T("I03.T04.d", "REALITY", "M", "L1", ["c"],
  "`reality-opts`: `public-key` (base64url, 32 байта), `short-id` (hex, ≤16 символов, чётная длина), прочие ключи "
  "pinned версии. Разрешённые сочетания: VLESS + TCP/gRPC/XHTTP (сверить с источником), обязателен TLS-блок с servername.",
  "native/tls.rs (reality)", "Неверная длина ключа, REALITY без servername, REALITY с WS → отказ с кодом.")
T("I03.T04.e", "Расширения VLESS/VMess", "M", "L1", ["c"],
  "VLESS: `flow` (`xtls-rprx-vision` только с TLS/REALITY на TCP), `packet-encoding` (`packetaddr`/`xudp`). "
  "VMess: TLS, `packet-addr`, `global-padding`, `authenticated-length`; `alterId` > 0 — решение по источнику.",
  "native/proto/vless.rs, vmess.rs", "Матрица flow × transport покрыта fixtures.")
T("I03.T04.f", "Транспорт WS и HTTPUpgrade", "M", "L1", ["b"],
  "`network: ws`, `ws-opts`: `path`, `headers` (HTTP token, без дублей регистра), `max-early-data`, "
  "`early-data-header-name`, `v2ray-http-upgrade`, `v2ray-http-upgrade-fast-open`.",
  "native/transport.rs (ws)", "Path без ведущего `/`, нечисловой early-data, конфликтующие флаги → отказ.")
T("I03.T04.g", "Транспорты HTTP и H2", "M", "L1", ["b"],
  "`http-opts`: `method`, `path` (список), `headers` (map → список строк). `h2-opts`: `host` (список), `path`. "
  "H2 требует TLS — проверка в I03.T04.i.",
  "native/transport.rs (http, h2)", "Fixtures для пустых списков, лишних ключей, header-инъекций CR/LF.")
T("I03.T04.h", "Транспорты gRPC и XHTTP", "M", "L1", ["b"],
  "`grpc-opts.grpc-service-name`. XHTTP: сверить, есть ли он в pinned v1.19.32 и какие ключи (`path`, `host`, "
  "`mode`, extra); если в pinned ядре нет — UnsupportedFeature и правка capability matrix.",
  "native/transport.rs (grpc, xhttp); правка I03-CAPABILITY-MATRIX", "Решение по XHTTP подтверждено ссылкой на pinned source.")
T("I03.T04.i", "Перекрёстная матрица protocol × transport × security", "M", "L1", ["d", "e", "f", "g", "h"],
  "Единая таблица допустимых сочетаний (из `capabilities::supported_transports` + требования TLS для grpc/h2, "
  "REALITY только для перечисленных транспортов, flow только TCP). Валидация после разбора узла.",
  "native/matrix.rs; генерированная таблица в capability doc", "Таблица в doc генерируется из кода (без ручной копии); fixture на каждый запрещённый класс.")
T("I03.T04.j", "Trojan", "M", "L1", ["i"],
  "`password`, `sni`, `alpn`, `skip-cert-verify`, `client-fingerprint`, `network` tcp/ws/grpc, `reality-opts` если "
  "поддержан, `ss-opts` → UnsupportedFeature, `udp`.",
  "native/proto/trojan.rs", "Позитивные tcp/ws/grpc; негатив без password, с h2, с ss-opts.")
T("I03.T04.k", "Hysteria2", "M", "L1", ["c"],
  "`password`, `ports` (диапазоны, лимит количества) и `hop-interval`, `up`/`down` (единицы), "
  "`obfs: salamander` + `obfs-password`, `sni`, `alpn`, `skip-cert-verify`, `fingerprint`, `udp`.",
  "native/proto/hy2.rs", "Port-hopping диапазон 1–65535, перевёрнутый диапазон, obfs без пароля → отказ.")
T("I03.T04.l", "TUIC", "M", "L1", ["c"],
  "v5: `uuid` + `password`; v4 `token` — UnsupportedFeature (или отдельное решение). `congestion-controller` "
  "(cubic/new_reno/bbr), `udp-relay-mode` (native/quic), `reduce-rtt`, `alpn`, `sni`, `disable-sni`, `request-timeout`.",
  "native/proto/tuic.rs", "Fixtures для v4/v5, неизвестного congestion-controller.")
T("I03.T04.m", "WireGuard", "L", "L1", ["b"],
  "`private-key`/`public-key`/`pre-shared-key` (base64, 32 байта), `ip`/`ipv6` (CIDR), `allowed-ips`, `mtu`, "
  "`reserved`, `peers` (список с лимитом), `persistent-keepalive`. `amnezia-wg-option`, `dialer-proxy`, "
  "`remote-dns-resolve`, `dns` → Restricted/Unsupported. Private key — только в закрытом blob.",
  "native/proto/wg.rs", "Неверная длина ключа, пересекающиеся peers, host-control поле → отказ; ключ не встречается в status DTO.")
T("I03.T04.n", "Расширения Shadowsocks", "M", "L1", ["b"],
  "Шифры 2022 (`2022-blake3-*`, длина ключа base64 под шифр, мультипользовательский `:`), `udp-over-tcp` (+version), "
  "плагины: `obfs`, `v2ray-plugin` — поддержать или явно UnsupportedFeature; `shadow-tls`, `restls` — Unsupported по матрице.",
  "native/proto/ss.rs", "Ключ 2022 неверной длины → отказ; неизвестный plugin → UnsupportedFeature.")
T("I03.T04.o", "Классификация секретов узлов", "M", "L1", ["j", "k", "l", "m", "n"],
  "Перечень секретных полей каждого протокола (uuid, password, private-key, psk, obfs-password, auth). Проверить, что "
  "секреты есть только в закрытом blob; status DTO, Debug, ошибки и diagnostic export их не содержат.",
  "native/secrets.rs; тест-сканер по всем fixtures", "Сканер находит 0 секретов вне закрытого blob на всём корпусе fixtures.")
T("I03.T04.p", "Решение: группы, правила, провайдеры (принято)", "S", "D", [],
  "Почти все реальные native-подписки содержат `proxy-groups` и `rules`. Сейчас они отвергаются целиком, значит "
  "большинство подписок не импортируется. Варианты: (1) отказ — как сейчас; (2) импорт узлов с явным, сохранённым в "
  "provenance списком «не импортировано: groups/rules» и подтверждением пользователя; (3) сохранять groups/rules типизированно для I04.",
  "запись в QUESTIONS.md / I03-DECISIONS", "Выбран вариант; он не противоречит запрету silent dropping.", decision="D1")
T("I03.T04.y", "Схема пропусков, TLS-отметка, подтверждение", "M", "L1", ["b"],
  "По I03-DECISIONS-2026-10-06: `ImportOmissions` в provenance (имена секций, классы строк, число узлов без проверки "
  "сертификата), `tls_verification` узла (Verified/Pinned/Disabled, в digest), новая версия схемы Store с миграцией, "
  "`PendingConfirmation` + `accept_omissions(digest)`, автообновление не шире подтверждённого, > 0 узлов, падение ≤ 50%.",
  "profiles/model.rs, profiles/store.rs, sources/pipeline.rs; тесты",
  "Старый Store мигрирует; без подтверждения не публикуется; автообновление с новым пропуском, нулём узлов или падением > 50% "
  "оставляет прежнее поколение; ни текста строк, ни значений в сводке.")
T("I03.T04.q", "Реализовать политику групп/правил/провайдеров", "L", "L1", ["p", "i", "y"],
  "По D1 (вариант 2): узлы из `proxies` и `inline` proxy-providers; groups/rules/sub-rules/rule-providers, http и file "
  "провайдеры — в `ImportOmissions` без сети и путей хоста; 0 узлов → отказ. Содержимое секций не пишется в диагностику.",
  "native/sections.rs", "Fixture реальной структуры подписки (обезличенный) импортируется или отвергается ровно по D1.")
T("I03.T04.r", "URI-парсер", "L", "L1", ["i", "j", "k", "l", "n"],
  "`vless://`, `vmess://` (base64-JSON формат v2rayN), `ss://` (SIP002 и legacy base64), `trojan://`, "
  "`hysteria2://`/`hy2://`, `tuic://`, `socks5://`, `http(s)://`. Percent-decoding, fragment → имя, query → тот же "
  "типизированный node, что и в native (одна каноническая форма, один digest). Неизвестный query-параметр → отказ.",
  "src/sources/parser/uri.rs", "Для каждого протокола URI и эквивалентный native дают одинаковый definition digest.")
T("I03.T04.s", "base64-подписка и смешанные списки", "M", "L1", ["r", "y"],
  "Std/urlsafe, с padding и без, перевод строк; лимит после декодирования; по D3: неподдержанные и испорченные строки — "
  "пропуск с классом и номером, порча base64-слоя или 0 узлов — отказ источника.",
  "src/sources/parser/base64.rs", "Fixtures: мусор, двойной base64, пустые строки, неизвестная схема.", decision="D3")
T("I03.T04.t", "Определение формата и negotiation для всех форматов", "M", "L1", ["s", "q"],
  "Классификатор тела: native YAML/JSON, URI-список, base64, HTML/заглушка, пустой. Обобщить "
  "`negotiate_native_source` → `negotiate_source`: HTML/непригодное тело даёт bounded retry следующим UA, "
  "unsupported semantics — terminal отказ.",
  "src/sources/parser/detect.rs; pipeline.rs", "Таблица «тело → решение» покрыта fixtures, включая HTML-заглушку провайдера.")
T("I03.T04.u", "Ручной собственный сервер", "M", "L1", ["r", "o"],
  "Вход: URI или структурированные поля CLI (`cm source add-server --protocol … --server … --port …`, секреты из stdin/файла, "
  "не из argv). SourceKind Manual, origin local, без fetch settings. Изменение = новое поколение, Node identity сохраняется.",
  "src/sources/manual.rs; CLI-подкоманда за флагом", "Секрет не попадает в argv/историю shell; правка сервера сохраняет Node ID.")
T("I03.T04.v", "Независимые источники для профилей", "M", "L1", ["u", "t"],
  "ConnectionProfile ссылается на (source, node) любого источника; несколько источников сосуществуют; удаление "
  "используемого источника → staged removal из I02; refresh одного источника не трогает другие.",
  "profiles/store: тесты сценариев", "Fixtures: два источника, профиль на каждом, удаление одного с активным профилем.")
T("I03.T04.w", "Сверка с ядром на корпусе fixtures", "M", "L1", ["t", "o"],
  "Для каждого принятого fixture сгенерировать минимальный config и прогнать `mihomo -t -d <tmp>` pinned версии, "
  "если бинарник доступен (без сети). Расхождение «мы приняли — ядро отвергло» = баг парсера.",
  "tests/audit_i03_core_check.rs (skip без бинарника, с явным статусом)", "0 расхождений на корпусе; skip отмечается в evidence как SKIPPED, не PASS.")
T("I03.T04.x", "Закрыть T04 в документации", "S", "L0", ["v", "w", "H.05.b"],
  "Обновить capability matrix (из кода), parser contract, I03-NATIVE-PARSER-FIRST → итоговый документ T04.",
  "I03-PARSER-FINAL.md", "Все пункты списка остатка из WORK-STOP закрыты или явно Unsupported со ссылкой.")

T("I03.T05.a", "Инвентарь регрессий I01–I03", "S", "L0", ["I03.T04.x", "H.01", "H.08"],
  "Каждая проверка → тест-ID, уровень доказательства, статус (PASS локально / NOT_RUN install / SKIPPED).",
  "I03-REGRESSION-INVENTORY.md", "Нет проверки без ID и статуса.")
T("I03.T05.b", "Fixtures отказов источника", "M", "L1", ["I03.T04.t"],
  "HTML/заглушка, `subscription-userinfo` с исчерпанной квотой/истёкшим сроком, timeout (mock-транспорт), "
  "исчезновение узла, обновление активного источника, битый import не заменяет рабочее состояние.",
  "tests/audit_i03_failures.rs", "Каждый сценарий: состояние Store до и после совпадает при отказе.")
T("I03.T05.c", "Реальный транспорт на локальном сервере", "M", "L1", ["H.04", "I03.T04.t"],
  "Интеграционный тест: локальный HTTP-сервер отдаёт разные тела по UA, задержки, redirect-цепочки, огромные тела.",
  "tests/audit_i03_transport.rs", "Deadline соблюдается с допуском; ответы классифицируются как в mock-тестах.")
T("I03.T05.d", "Сборка и манифест", "S", "L1", ["a", "b", "c"],
  "`build.sh` musl CLI + compile COSMIC. Манифест: git revision, хеш дерева, Cargo.lock sha256, rustc, sha256 бинарников.",
  "i03-evidence/build-<date>/manifest.json", "Манифест воспроизводится повторной сборкой (хеши совпадают или отличие объяснено).")
T("I03.T05.e", "Installation driver I03", "M", "L0", ["d"],
  "Скрипт проверок для установки: реальный импорт подписок пользователя (секреты остаются локально), negotiation с "
  "реальным провайдером, refresh, отказ. Только сценарий — выполнение в X-треке.",
  "tests/check_i03_regressions.py", "Driver проходит dry-run без сети.")
T("I03.T05.f", "Отчёт закрытия I03", "S", "L0", ["e"],
  "Итог кодовой части I03, ссылки на evidence, обновление DAG-PLAN.json статусов.",
  "I03-CODER-COMPLETION.md", "Статусы кодовой и runtime-приёмки разделены.")

# ───────────────────────────── V: вертикальный срез ─────────────────────────────
M("V", "Вертикальный срез: новая модель → пользователь", "P0", "slice",
  "Предложение (решение D7): подключить Store/sources к CLI и текущему singleton mihomo до I04+, "
  "чтобы новая архитектура получила живую проверку. Соответствует «Поставке 1» из DETAILED-DAG-PLAN §8.",
  gate="Не подменяет I04: генератор V.03 становится основой I04.T04.a")

T("V.01", "ADR сосуществования legacy Subs и Store", "M", "D", ["I03.T04.p"],
  "Одноразовый импорт `subs.json` в Store (dry-run → apply), источник истины на переходный период, откат, "
  "что видит TUI во время перехода.",
  "ADR-LEGACY-STORE-BRIDGE.md", "Решение пользователя по D7 записано.", decision="D7")
T("V.02", "CLI только для чтения", "M", "L1", ["V.01", "I03.T04.t", "H.04"],
  "`cm source list|show|import --dry-run|doctor`: новый pipeline с реальным транспортом, диагностика без изменения "
  "работающего VPN, вывод без секретов.",
  "src/main.rs подкоманды; i18n строки", "Dry-run реальной подписки печатает узлы и отказы; файлы legacy не изменены.")
T("V.03", "Генератор config mihomo из Store (ранний I04.T04.a)", "L", "L1", ["V.02", "I03.T05.f", "H.06"],
  "Узлы Store → `proxies`, авто-группа, пользовательские правила `rules.txt`, DNS как в текущем legacy. "
  "Проверка `validate_candidate` (`mihomo -t`).",
  "src/core/mihomo/config.rs", "Для обезличенных fixtures config принят ядром; diff с legacy-генерацией объяснён.")
T("V.04", "Переключение singleton на источник из Store", "L", "L1", ["V.03"],
  "`cm vpn use source:ID` за флагом: кандидат → проверка → атомарная замена → reload, откат при ошибке; "
  "legacy-путь остаётся по умолчанию. До I04.T04.f для источника с пропущенными groups/rules — предупреждение (D1).",
  "src/vpn/* интеграция", "Unit: отказ проверки оставляет прежний config; флаг выключен по умолчанию.")
T("V.05", "Приёмка среза на установке", "L", "L4", ["V.04", "X.01"],
  "По parity-протоколу I01: та же подписка через legacy и через Store, одинаковый узел, A/B/A.",
  "v-evidence/acceptance/summary.json", "Паритет подтверждён или различие объяснено; без утверждения «VPN исправлен» при неполной parity.")

# ───────────────────────────── I04: контракт ядер и mihomo ─────────────────────────────
M("I04", "Контракт ядер и адаптер mihomo", "P0", "core",
  "Typed CoreAdapter, instance-owned ресурсы, A+C разделение ответственности, рабочий mihomo-адаптер.",
  questions="Q02, Q03, Q07", gate="G1 (вместе с I02/I06)")

T("I04.T01.a", "Trait CoreAdapter и таксономия ошибок", "M", "L1", [],
  "capabilities(), validate(config), start/stop/reload, health(), statistics(); раздельные состояния ApiReady / "
  "RouteReady / RemoteReachable; ошибки без секретов. Может идти параллельно с I03.",
  "src/core/adapter.rs; ADR-CORE-ADAPTER.md", "Trait компилируется с fake-реализацией; ADR перечисляет, что не гарантируется.")
T("I04.T01.b", "Дескриптор возможностей pinned ядра", "S", "L1", ["a", "I03.T04.i"],
  "Capabilities mihomo 1.19.32 из матрицы I03 (одна таблица, без копии).", "src/core/mihomo/caps.rs", "Тест: матрица I03 и дескриптор согласованы.")
T("I04.T01.c", "Fake adapter для тестов", "S", "L1", ["a"],
  "In-memory адаптер с управляемыми отказами для I06/I07/I09 до реальных ядер.", "src/core/fake.rs", "Используется хотя бы одним тестом lifecycle.")
T("I04.T02.a", "Каталоги и права instance", "M", "L1", ["I04.T01.a"],
  "InstanceId → `/var/lib/cm/instances/<id>/{config,cache,run}`, владелец, режимы, очистка.",
  "src/core/instance.rs", "Fixture: два instance не видят файлов друг друга; удаление не трогает соседей.")
T("I04.T02.b", "Реестр выделенных ресурсов", "M", "L1", ["a"],
  "Leases в Store через CAS: socket, порты, fwmark, номер таблицы маршрутов, имя TUN; освобождение и утечки после crash.",
  "src/core/leases.rs", "Конкурентное выделение (тест с потоками) не даёт дублей.")
T("I04.T02.c", "systemd template `cm-core@.service`", "M", "L1", ["a"],
  "На основе ограничений cm-vpn: CAP_NET_ADMIN/RAW/BIND только при нужде, ReadWritePaths только каталог instance, "
  "NoNewPrivileges, отдельный RuntimeDirectory.", "packaging/systemd/cm-core@.service", "`systemd-analyze verify` и `security` без регресса к cm-vpn.")
T("I04.T02.d", "Legacy host wrapper", "M", "L1", ["a", "H.06"],
  "Текущий cm-vpn как instance `host-legacy` за CoreAdapter, без изменения поведения пользователя.",
  "src/core/legacy_host.rs", "Все VPN-тесты legacy проходят через wrapper.")
T("I04.T03.a", "Запрет конкурирующих настроек в worker", "M", "L1", ["I04.T02.a", "I04.T02.b"],
  "Генератор app-worker конфигов не включает `tun.auto-route`, `auto-redirect`, `dns-hijack`, внешние listeners; "
  "worker слушает только выделенные ресурсы.", "src/core/policy.rs", "Fixture: попытка включить запрещённое → ошибка генерации.")
T("I04.T03.b", "ADR владения TUN-устройством", "M", "L2", ["a", "H.11"],
  "Эксперимент: CM заранее создаёт persistent TUN (owner uid) и передаёт ядру vs ядро создаёт TUN без маршрутов. "
  "Критерии: права, восстановление после crash, MTU, IPv6.", "ADR-TUN-OWNERSHIP.md + capture", "Выбран вариант, подтверждённый на harness.")
T("I04.T04.a", "Генератор config mihomo (обобщение V.03)", "L", "L1", ["I04.T03.a", "I03.T04.i"],
  "Узлы поддержанных протоколов (растёт вместе с I03.T04.j–n), DnsPolicy профиля, listeners instance; если V.03 сделан — обобщить, иначе написать.",
  "src/core/mihomo/config.rs", "Корпус I03 → config → `mihomo -t` без расхождений.")
T("I04.T04.b", "Валидация и безопасные ошибки", "M", "L1", ["a"],
  "`mihomo -t -d <sandbox>`, разбор stderr в фиксированные коды (обобщить `core_validation_reason`).",
  "src/core/mihomo/validate.rs", "Ни одна ошибка не содержит строк конфига.")
T("I04.T04.c", "Lifecycle через API на unix-сокете", "L", "L2", ["a", "I04.T02.c", "H.11"],
  "start/stop/reload (`PUT /configs`), health (`/version`, delay), проверки владельца сокета и peer из текущего vpn.rs.",
  "src/core/mihomo/lifecycle.rs", "На harness: start → ApiReady → reload → stop без утечек процессов и сокетов.")
T("I04.T04.d", "Статистика instance", "S", "L2", ["c"],
  "Трафик и соединения через API; без суммирования физического и TUN трафика.", "src/core/mihomo/stats.rs", "Счётчики совпадают с capture ±5%.")
T("I04.T04.e", "Эксперимент: N workers vs один multi-outbound", "M", "L2", ["c"],
  "RAM/CPU, изоляция отказов, независимость reload на 1/5/20 туннелях.", "ADR-WORKER-TOPOLOGY.md + замеры", "Решение подкреплено числами.")
T("I04.T04.f", "Типизированные группы и правила издателя (D1, вариант 3)", "L", "L1", ["I04.T04.a", "I03.T04.q"],
  "Разбор `proxy-groups` (select/url-test/fallback/load-balance; relay — отказ, как в ядре), `rules` с проверкой цели, "
  "`sub-rules` и циклов, `inline` rule-providers; повторный разбор из сохранённого сырого тела без загрузки; генерация в config.",
  "src/core/mihomo/policy.rs", "Подписка с группами и правилами издателя даёт config, принятый `mihomo -t`; пропуски D1 для групп/правил исчезают.")
T("I04.T05.a", "Два instance + legacy host одновременно", "M", "L2", ["I04.T04.c", "I04.T02.d"],
  "Независимые конфиги/узлы, остановка одного не влияет на другой и на host.", "i04-evidence/two-instances", "Capture показывает раздельный egress.")
T("I04.T05.c", "Отчёт кодовой части и ADR I04", "S", "L0", ["a", "I04.T01.b", "I04.T01.c", "I04.T03.b", "I04.T04.b", "I04.T04.d", "I04.T04.e", "I04.T04.f"], "Итог кодовой части, ограничения, downstream-воздействие. Parity адаптера (L4) — X.08 → X.07 и не блокирует I06/I07.", "I04-REPORT.md", "Статусы кодовой/runtime-приёмки разделены.")

# ───────────────────────────── I05: Xray ─────────────────────────────
M("I05", "Адаптер Xray и доставка ядер", "P1", "core",
  "Второе ядро за тем же контрактом и безопасное обновление/откат обоих ядер.",
  questions="Q03, Q07", gate="G3 частично (DNS на обоих ядрах)")

T("I05.T01.a", "Pin Xray", "S", "L0", ["I04.T01.a"],
  "Версия, URL релиза, `.dgst`-хеши, архитектура x86_64, geo data, лицензия.", "I05-PIN.md", "Хеши проверены по двум источникам.")
T("I05.T01.b", "Capability gaps относительно CoreAdapter", "M", "L0", ["a", "I04.T01.b"],
  "Чего нет у Xray: reload без рестарта, delay API, протоколы (Hy2/TUIC/WG в pinned версии) → Unsupported.",
  "I05-GAPS.md", "Каждый gap имеет решение: unsupported / обход / вне scope.")
T("I05.T02.a", "Прототип TCP/UDP/TUN в A+C", "L", "L2", ["I05.T01.b", "H.11", "I04.T03.b"],
  "Проверить native TUN inbound pinned версии; иначе tproxy/dokodemo-door с маршрутизацией CM. UDP и QUIC отдельно.",
  "ADR-XRAY-DATAPATH.md + capture", "TCP+UDP проходят; SOCKS-only путь не назван полным туннелем.")
T("I05.T02.b", "DNS-путь Xray", "M", "L2", ["a"],
  "DNS inbound → routing → DNS outbound; qtypes сверх A/AAAA; truncation/TCP retry.", "capture + таблица qtypes", "Gap зафиксирован, forwarder не добавлен без доказательства.")
T("I05.T03.a", "Генератор config Xray", "L", "L1", ["I05.T02.a", "I03.T04.x"],
  "Маппинг узлов Store → outbounds с сохранением семантики; несопоставимые → Unsupported.",
  "src/core/xray/config.rs", "Корпус I03 → config → `xray run -test` без расхождений; таблица неэквивалентностей.")
T("I05.T03.b", "Validate, lifecycle, ошибки", "L", "L2", ["a"],
  "`-test`, readiness, start/stop/reload (рестарт, если нет hot reload), безопасный маппинг ошибок.",
  "src/core/xray/*.rs", "Fake и harness-тесты lifecycle проходят.")
T("I05.T04.a", "Загрузчик ядер", "M", "L1", ["H.04", "I05.T01.a"],
  "Через транспорт H.04: лимиты, sha256, распаковка с лимитом, общий для mihomo и Xray (заменить текущий `cm vpn core update`).",
  "src/core/delivery.rs", "Битый хеш/архив-бомба/подмена имени → отказ.")
T("I05.T04.b", "Кандидат, атомарная замена, откат", "M", "L1", ["a", "I05.T03.b"],
  "Кандидат проверяется `version` и validate текущих конфигов; предыдущий бинарник сохраняется; работающие instance "
  "остаются на своём бинарнике до рестарта.", "src/core/delivery.rs", "Прерывание на каждом шаге оставляет рабочий бинарник.")
T("I05.T05.a", "Проверка на тестовом endpoint", "M", "L2", ["I05.T04.b", "I05.T02.b", "I03.T05.f", "I04.T05.c"],
  "Транспорты, DNS, lifecycle; список случаев, где сравнение с mihomo невозможно.", "i05-evidence", "Матрица pass/unsupported заполнена.")

# ───────────────────────────── I06: привилегированный контроллер ─────────────────────────────
M("I06", "Привилегированный контроллер", "P0", "root",
  "Typed root-протокол с проверкой владельца, транзакциями и минимальными правами.",
  questions="Q08, Q11, Q16, Q25", gate="G1")

T("I06.T01.a", "ADR Q08: helper или отдельный сервис", "M", "D", ["H.07", "I04.T01.a"],
  "Сравнить расширение helper (сериализация пакетных операций) и `cm-netd` для параллельных сетевых операций.",
  "ADR-CONTROLLER.md", "Решение с учётом Q16 (несколько пользователей).")
T("I06.T01.b", "Typed протокол и polkit-действия", "M", "L1", ["a", "H.09"],
  "Versioned serde enum, классы операций → отдельные polkit actions, лимиты размера сообщения.",
  "src/controller/protocol.rs; polkit policy", "Fuzz-цель декодера готова.")
T("I06.T02.a", "Идентификация peer", "M", "L1", ["I06.T01.b"],
  "SO_PEERCRED + pidfd + start time + cgroup + logind session + netns inode; правила передачи FD.",
  "src/controller/peer.rs", "PID reuse ловится через pidfd в тесте.")
T("I06.T02.b", "Возврат к исходному пользователю", "S", "L1", ["a"],
  "AppLaunch выполняется под uid/gid/groups вызывающего, не root.", "src/controller/drop.rs", "Тест: дочерний процесс имеет uid вызывающего и пустые capabilities.")
T("I06.T03.a", "Журнал владения и транзакции", "L", "L1", ["I06.T02.a", "I04.T02.b"],
  "allocate/apply/check/stop/reconcile; журнал в Store; compensation при частичном отказе; ключи идемпотентности.",
  "src/controller/txn.rs", "Fault injection на каждом шаге → консистентное состояние после reconcile.")
T("I06.T03.b", "Интеграция с CoreAdapter", "M", "L2", ["a", "I04.T05.a"],
  "Контроллер управляет instance через адаптер; generation ownership.", "src/controller/core_ops.rs", "Harness: старт/стоп instance только через контроллер.")
T("I06.T04.a", "Ужесточение", "M", "L1", ["I06.T03.b"],
  "Bounding capabilities, close_range для унаследованных FD, секреты через memfd/FD (не argv/env), rlimits, квоты instance, "
  "лимит конкурентных операций, таймауты, редакция диагностики.", "src/controller/*", "Чек-лист с тестом на каждый пункт.")
T("I06.T05.a", "Отрицательные проверки", "M", "L2", ["I06.T04.a", "I06.T02.b", "I04.T05.c"],
  "Чужой uid, PID reuse, argv/path injection, устаревший запрос, crash посреди транзакции, fuzz декодера (cargo-fuzz, ≥1 ч).",
  "i06-evidence", "0 обходов; найденные креши исправлены и превращены в регрессии.")

# ───────────────────────────── I07: режимы хоста и outer egress ─────────────────────────────
M("I07", "Режимы хоста и outer egress", "P0", "net",
  "CM владеет маршрутами и firewall; worker выходит наружу без петель; host off/proxy/tunnel независимы.",
  questions="Q02, Q03, Q09", gate="G1 → I08")

T("I07.T01.a", "Packet-flow спецификация", "M", "L0", ["I06.T05.a", "I04.T05.a"],
  "TCP/UDP/DNS для host tunnel и app worker; схема fwmark, диапазон приоритетов `ip rule`, номера таблиц, "
  "таблица nftables `inet cm` и sets; чужие rules сохраняются.", "I07-PACKET-FLOW.md + схема", "Ревью: каждый пакетный путь имеет владельца.")
T("I07.T01.b", "nftables / iptables-nft", "S", "L0", ["a"],
  "Определение доступного backend, сосуществование с firewalld/ufw.", "ADR-FIREWALL-BACKEND.md", "Матрица дистрибутивов из «Поддерживаемых систем».")
T("I07.T02.a", "Outer egress worker", "L", "L2", ["I07.T01.b"],
  "Сокеты worker помечаются (SO_MARK задаёт CM, не импорт) → обходная таблица → физический интерфейс; "
  "нет chaining через host VPN и рекурсии.", "src/net/egress.rs", "Capture: endpoint ядра идёт мимо TUN при включённом host tunnel.")
T("I07.T03.a", "Политика host off/proxy/tunnel", "L", "L2", ["I07.T02.a"],
  "Переходы между режимами транзакционно; host proxy явно не означает полный охват.", "src/net/host.rs", "Все 6 переходов проверены на harness.")
T("I07.T04.a", "NetworkManager, firewalld, чужие VPN", "L", "L2", ["I07.T03.a"],
  "TUN unmanaged в NM, реакция на смену маршрутов через netlink-монитор, существующее обнаружение конфликтов (vpn.rs), "
  "транзакционное применение с rollback.", "src/net/integration.rs", "Включение/выключение Wi-Fi и чужого VPN не ломает состояние.")
T("I07.T05.a", "Проверка смены сети и stop host", "M", "L2", ["I07.T04.a"],
  "Link down/up, смена default route, stop host при работающих app workers.", "i07-evidence (для I08)", "App workers сохраняют egress; утечек нет на capture.")

# ───────────────────────────── I08: сетевые окружения приложений ─────────────────────────────
M("I08", "Сетевые окружения приложений", "P0", "net",
  "Netns на приложение/сессию с единственным разрешённым выходом через worker.",
  questions="Q09, Q10, Q11", gate="G2 (с I09)")

T("I08.T01.a", "Решение Q09: локальные сети и сервисы", "S", "D", ["I07.T01.a"],
  "Что доступно приложению в туннеле: loopback, LAN, принтеры, mDNS, D-Bus системный.", "QUESTIONS.md Q09", "Решение пользователя.")
T("I08.T01.b", "Дизайн netns/veth/TUN/cgroup", "M", "L0", ["a", "I07.T05.a"],
  "Связь app netns с worker, cgroup v2 на сессию, межприложенная изоляция; abstract unix sockets привязаны к netns "
  "(X11, некоторые IPC) — учесть.", "ADR-APP-NETNS.md", "Все пути выхода перечислены и закрыты либо разрешены по Q09.")
T("I08.T02.a", "Выделение сети через контроллер", "L", "L2", ["I08.T01.b", "I06.T05.a"],
  "Создание netns, nft default-drop до запуска приложения, подключение к worker.", "src/net/appns.rs", "До подключения worker любой выход блокируется (capture).")
T("I08.T03.a", "Тестовый launcher под исходным uid", "M", "L2", ["I08.T02.a"],
  "Ограниченный запуск процесса в netns до универсального I11.", "tests/netharness/run-in-ns", "Процесс получает uid пользователя и только этот netns.")
T("I08.T04.a", "Проверки обхода", "M", "L2", ["I08.T03.a"],
  "fork/exec, IPv4/IPv6, прямые socket calls, raw sockets, унаследованный сетевой FD; GUI (Wayland), audio (PipeWire).",
  "i08-evidence", "Каждый путь: прошёл через tunnel или заблокирован.")
T("I08.T05.a", "Два namespace на двух tunnels", "M", "L2", ["I08.T04.a"],
  "Независимость, общий транспорт без неявного доступа, критерии cleanup.", "i08-evidence", "Удаление одного не задевает другого.")

# ───────────────────────────── I09: fail-closed ─────────────────────────────
M("I09", "Fail-closed, lifecycle и восстановление", "P0", "net",
  "Никакой прямой утечки при сбоях; восстановление после crash и перезагрузки.", questions="Q12, Q13", gate="G2")

T("I09.T01.a", "Машина состояний", "M", "L1", ["I08.T05.a"],
  "allocating/blocked/starting/verified/degraded/stopping/reconciling, хранение в Store, порядок firewall → worker → launch.",
  "src/net/state.rs", "Property-тест: недопустимые переходы невозможны.")
T("I09.T02.a", "Постоянная блокировка", "L", "L2", ["^"],
  "Правила в ядре не зависят от процессов CM; остановка tunnel не снимает блок с живого приложения.", "src/net/block.rs", "kill всех процессов CM → приложение без сети, не напрямую.")
T("I09.T03.a", "Reconcile после crash/reboot", "L", "L2", ["I09.T02.a"],
  "Ранний systemd unit до сетевых сервисов; жизнь сети приложения не привязана к TUI.", "packaging/systemd/cm-reconcile.service", "Reboot посреди reload → консистентное состояние.")
T("I09.T04.a", "Политика отказа endpoint", "M", "L1", ["I09.T03.a"],
  "Block или разрешённый резерв с constraints; stop all; освобождение ресурсов.", "src/net/failure.rs", "Решение по Q13 применено.")
T("I09.T05.a", "Fault matrix", "L", "L4", ["I09.T04.a"],
  "kill -9 worker/helper/UI, невалидный reload, network flap, suspend/resume, ошибки cleanup; capture негативных путей.",
  "i09-evidence/fault-matrix", "0 прямых утечек.")

# ───────────────────────────── I10: DNS/IPv6/UDP ─────────────────────────────
M("I10", "DNS, IPv6, UDP и утечки", "P0", "net",
  "Доказанный DNS-путь на обоих ядрах, IPv6 pass/block, UDP/QUIC.", questions="Q03, Q09, Q14", gate="G3")

T("I10.T01.a", "Спецификация DnsPolicy/DnsRuntime", "M", "L0", ["I04.T05.c"],
  "Можно начинать раньше: bootstrap endpoint через outer egress, разрешение приложений, fake-IP инвалидация на поколение.",
  "I10-DNS-SPEC.md", "Ревью спецификации.")
T("I10.T02.a", "Пробы DNS mihomo", "M", "L2", ["I10.T01.a", "I08.T05.a"],
  "UDP/TCP, A/AAAA/TXT/MX/SRV/HTTPS/SVCB, NXDOMAIN, truncation.", "i10-evidence/mihomo", "Таблица qtype × результат.")
T("I10.T02.b", "Пробы DNS Xray", "M", "L2", ["I10.T01.a", "I05.T05.a"],
  "Та же матрица для Xray.", "i10-evidence/xray", "Таблица qtype × результат.")
T("I10.T03.a", "ADR: встроенный resolver или forwarder", "S", "D", ["I10.T02.a", "I10.T02.b"],
  "Forwarder только при доказанном gap.", "ADR-DNS.md", "Решение по данным проб.")
T("I10.T04.a", "Per-tunnel resolution и IPv6", "L", "L2", ["I10.T03.a", "I09.T05.a"],
  "resolv.conf в mount ns netns, IPv6 pass/block, UDP/STUN/QUIC, DoH приложения остаётся в tunnel.", "src/net/dns.rs", "Capture без DNS вне tunnel.")
T("I10.T05.a", "Матрица протоколов на обоих ядрах", "L", "L2", ["I10.T04.a"],
  "Отказ DNS, смена поколения, два кэша tunnel.", "i10-evidence/matrix", "Pass или явный block на обоих адаптерах.")

# ───────────────────────────── I11–I13: приложения ─────────────────────────────
M("I11", "Универсальный запуск приложений", "P1", "apps",
  "Произвольный исполняемый файл в готовом окружении без shell и без списка продуктов в коде.", questions="Q10, Q11")

T("I11.T01.a", "Поля запуска ApplicationDefinition", "S", "L1", ["I10.T05.a"],
  "Абсолютный путь (без PATH-подмены), argv массивом, cwd, allowlist env, data profile.", "profiles/model.rs", "Валидация с отказами на относительный путь/NUL.")
T("I11.T02.a", "Typed launch через контроллер", "L", "L2", ["^"],
  "setns + сброс в uid/gid/groups, transient scope в cgroup сессии, возврат pidfd.", "src/controller/launch.rs", "Harness: процесс в нужном netns/cgroup под uid пользователя.")
T("I11.T03.a", "Desktop entry и CLI", "M", "L1", ["I11.T02.a"],
  "`cm app run ID`, генерация .desktop, экранирование пробелов и спецсимволов.", "src/app/*.rs", "Fixtures имён с пробелами, кавычками, юникодом.")
T("I11.T04.a", "Уже запущенные и single-instance", "M", "L3", ["I11.T03.a"],
  "Handoff в работающий экземпляр (D-Bus, unix-сокет) уводит в сеть хоста → обнаружить и отказать/предупредить.", "src/app/instance.rs", "Сценарий браузера: второй запуск не уходит в host-сеть молча.")
T("I11.T05.a", "Проверка на тестовых бинарниках", "M", "L3", ["I11.T04.a"],
  "Ошибки executable/permissions, повторный запуск, фоновые потомки, привязка Session.", "i11-evidence", "Все сценарии воспроизводимы.")

M("I12", "Группы и независимые назначения", "P1", "apps",
  "Своё туннельное подключение или общая группа; независимые источники.", questions="Q05, Q12, Q16")
T("I12.T01.a", "App → own tunnel / shared group", "M", "L2", ["I11.T05.a"], "Каждый профиль выбирает свой source/server/core.", "src/app/assign.rs", "Две подписки одновременно на разных apps.")
T("I12.T02.a", "Refcount и lifetime общего tunnel", "M", "L1", ["^"], "Завершение последней сессии ≠ host lifecycle (Q12).", "src/app/shared.rs", "Property-тест refcount.")
T("I12.T03.a", "Окружение на сессию при общем выходе", "M", "L2", ["^"], "Per-session EnvironmentProfile, per-data-profile identity.", "src/app/env.rs", "Private state не смешивается.")
T("I12.T04.a", "Смена назначения и поколения", "M", "L2", ["^"], "Без тихой переадресации живого соединения; явный restart/reconnect (Q13).", "src/app/reassign.rs", "Сценарий исчезновения узла посреди сессии.")
T("I12.T05.a", "Проверки групп", "M", "L4", ["^"], "Закрытие одного из двух apps, несколько групп, stop host.", "i12-evidence", "Все сценарии pass.")

M("I13", "Brokers, supervisors и упаковки", "P1", "apps",
  "Честная матрица: какой процесс на самом деле ходит в сеть для native/AppImage/Flatpak/systemd user/D-Bus.", questions="Q10, Q11, Q17")
T("I13.T01.a", "Матрица фактических executors", "M", "L0", ["I11.T05.a"], "native/terminal/GUI/AppImage/Flatpak, Wayland/X11, portals, systemd user services.", "I13-MATRIX.md", "Для каждой ячейки известен executor.")
T("I13.T02.a", "Fixtures broker-сценариев", "M", "L3", ["I13.T01.a", "I12.T05.a"], "Single-instance браузер, общий supervisor, делегированные операции.", "tests/brokers/", "Fixtures воспроизводимы.")
T("I13.T03.a", "Адаптеры упаковок", "L", "L3", ["^"], "Flatpak (bwrap в netns), AppImage, отказ для systemd user/D-Bus activation, если нельзя охватить.", "src/app/packaging/*.rs", "Каждый адаптер: supported/partial/unsupported с причиной.")
T("I13.T04.a", "Ограничения брокеров", "M", "L3", ["^"], "Унаследованные сокеты хоста, геолокация, GUI/audio/files; побочные эффекты видны пользователю.", "src/app/broker_policy.rs", "UI/CLI показывает последствия.")
T("I13.T05.a", "Подтверждение executor", "M", "L3", ["^"], "netns/cgroup фактического executor для каждой упаковки и версии.", "i13-evidence", "Матрица заполнена доказательствами.")

# ───────────────────────────── I14, I15-R, I15: окружение ─────────────────────────────
M("I14", "Страна, timezone и locale", "P1", "env", "Региональное окружение процесса без изменения хоста.", questions="Q01, Q15, Q18")
T("I14.T01.a", "Наблюдения страны exit-IP", "M", "L2", ["I10.T05.a"], "Несколько источников через tunnel, свежесть, расхождения; имя узла не доказательство.", "src/env/geo.rs", "Политика расхождений проверена fixtures.")
T("I14.T02.a", "EnvironmentProfile и пресеты", "M", "L1", ["^"], "IANA timezone, DST, locale, languages; несколько допустимых комбинаций на страну.", "src/env/preset.rs", "Таблица пресетов с источником данных.")
T("I14.T03.a", "Применение к процессу", "M", "L2", ["^", "I11.T02.a"], "TZ env, приватный /etc/localtime в mount ns, проверка наличия locale.", "src/env/apply.rs", "Хост не меняется.")
T("I14.T04.a", "Проверки libc/процесса/браузера", "M", "L3", ["^"], "Две apps на общем tunnel с разными пресетами.", "i14-evidence", "Значения совпадают с пресетом.")
T("I14.T05.a", "Инвалидация после failover", "S", "L2", ["^"], "Повторная проверка constraint; без тихого рестарта приложения.", "src/env/geo.rs", "Сценарий смены страны.")

M("I15-R", "Раннее исследование отпечатка (можно начинать сейчас)", "P1", "env",
  "Отделить доказуемое от желаемого до большой интеграции.", questions="Q01, Q26, Q27", gate="G4")
T("I15-R.T01.a", "Наблюдаемые свойства и scope", "M", "L0", [], "Network location, app environment, device identity, browser state, account link.", "I15R-SCOPE.md", "Решение по Q01.", decision="Q01")
T("I15-R.T02.a", "Coverage matrix механизмов", "L", "L0", ["^"], "Политики/prefs, расширения, CDP, патчи движка; UA, Client Hints, workers, Canvas/WebGL/fonts/audio, TLS.", "I15R-COVERAGE.md", "Каждая ячейка со ссылкой на источник.")
T("I15-R.T03.a", "Лабораторный эксперимент", "L", "L3", ["^"], "Отдельный тестовый профиль: controls до первого запроса, новые вкладки/workers, cold restart.", "i15r-evidence", "Воспроизводимый протокол.")
T("I15-R.T04.a", "Каталог устройств: происхождение и правомерность", "M", "L0", ["^"], "Согласованность с реальным движком; 28 дней и 5 записей истории — гипотезы (Q27).", "I15R-CATALOG.md", "Решение по Q26/Q27.")
T("I15-R.T05.a", "ADR go/conditional/no-go по свойствам", "S", "D", ["^"], "Evidence, цена поддержки, критерии регрессии.", "ADR-FINGERPRINT.md", "Каждое свойство имеет статус.")

M("I15", "Приватное окружение и адаптеры приложений", "P1", "env", "Только доказанные в I15-R механизмы.", questions="Q11, Q17, Q18")
T("I15.T01.a", "Контракт адаптера", "M", "L0", ["I13.T05.a", "I14.T05.a", "I15-R.T05.a"], "isolated/fresh/existing data state без авто-удаления HOME.", "ADR-APP-ADAPTER.md", "Scope согласован с G4/G5.")
T("I15.T02.a", "Границы XDG/HOME/keyring/SSH-agent/IPC", "L", "L3", ["^"], "С учётом подтверждённых packaging adapters.", "src/env/private.rs", "Каждая граница проверена.")
T("I15.T03.a", "BrowserIdentityProfile", "L", "L3", ["^"], "Липкая согласованная личность, явная смена, история, version policy.", "src/env/identity.rs", "Только свойства со статусом go.")
T("I15.T04.a", "Интеграция до первого запроса", "L", "L3", ["^"], "Frames/workers/restarts; identity не меняется в живом процессе.", "src/env/adapters/*", "Capture первого запроса.")
T("I15.T05.a", "Проверка surfaces и сохранности аккаунта", "M", "L4", ["^"], "Частичные/unsupported свойства показываются.", "i15-evidence", "Нет неподтверждённых гарантий в UI.")

# ───────────────────────────── I16: качество ─────────────────────────────
M("I16", "Оценка качества и автоматический выбор", "P1", "quality", "Объяснимый выбор узла: сначала ограничения, потом score.", questions="Q13, Q18, Q19")
T("I16.T01.a", "Constraints-before-score", "M", "L1", ["I03.T05.f", "I04.T05.c", "I05.T05.a", "I09.T05.a", "I10.T05.a", "I12.T05.a", "I14.T05.a"],
  "Capabilities, allowlist источников, страна, NET/DNS, лимиты провайдера, stable-IP.", "src/quality/constraints.rs", "Fixtures отсечения.")
T("I16.T02.a", "Метрики и budget проб", "M", "L1", ["^"], "pinned/failover/adaptive; success/latency/TTFB/jitter/обрывы/CPU/RAM; privacy cost.", "src/quality/metrics.rs", "Budget не превышается.")
T("I16.T03.a", "A/B/A пилот", "L", "L2", ["^"], "Размер выборки и допуски из пилота (30 проб — не статистика).", "i16-evidence/pilot", "Параметры выведены из данных.")
T("I16.T04.a", "Score, гистерезис, pinning сессий", "L", "L1", ["^"], "Cooldown, drain/reconnect; без обещания переноса открытого stream.", "src/quality/select.rs", "Property-тест отсутствия флаппинга.")
T("I16.T05.a", "Проверки деградации", "M", "L2", ["^"], "Флаппинг, unavailable, metered/battery, смена страны; причина каждого выбора.", "i16-evidence", "Каждое решение объяснимо.")

# ───────────────────────────── I17-D, I17: интерфейс ─────────────────────────────
M("I17-D", "Ранний проект TUI/апплета (можно начинать сейчас)", "P1", "ui", "Макеты на fixtures без обещаний гарантий.", questions="Q22")
T("I17-D.T01.a", "Fixtures состояний", "S", "L1", [], "host off/proxy/tunnel, separate/shared apps, blocked/degraded/unknown.", "tests/fixtures/ui-states/", "Покрыты все оси NET/REGION/STATE/APP.")
T("I17-D.T02.a", "Навигация: экран VPN vs страница apps/tunnels", "M", "L0", ["^"], "Клавиши, отмена, подтверждение последствий stop.", "I17D-NAV.md", "Выбор варианта.")
T("I17-D.T03.a", "Отображение осей и возраста evidence", "M", "L1", ["^"], "Не сводить к одной зелёной лампе.", "src/tui/mock_tunnels.rs (за флагом)", "Рендер на fixtures.")
T("I17-D.T04.a", "SVG апплета для новых состояний", "M", "L1", ["^"], "host off + app active, partial failure, blocked; темы.", "cosmic/res/*.svg", "Сборка cosmic с новыми иконками.")
T("I17-D.T05.a", "Проверка размеров и локалей", "S", "L1", ["^"], "40×12, 60×18, 80×24, 120×32; 6 языков.", "snapshot-тесты", "Snapshot на каждый размер/язык.")

M("I17", "TUI, апплет и наблюдаемость", "P1", "ui", "Реальные статусы вместо макетов.", questions="Q01, Q22")
T("I17.T01.a", "Сверка макетов с контрактами", "S", "L0", ["I15.T05.a", "I16.T05.a", "I17-D.T05.a"], "Назначение own/group tunnel.", "I17-UI-CONTRACT.md", "Нет provisional полей.")
T("I17.T02.a", "Неблокирующие действия и статусы", "L", "L1", ["^"], "Host/app статусы, свежесть, ядро/узел, причина block/failover.", "src/tui/*", "Фоновая модель как в 0.2.7.")
T("I17.T03.a", "Диагностический экспорт", "M", "L1", ["^"], "instance/generation/time; редакция секретов, argv, путей.", "src/diag/export.rs", "Сканер секретов по экспорту = 0.")
T("I17.T04.a", "Апплет и монитор ресурсов", "M", "L3", ["^"], "Сессии, ошибки, CPU/RAM workers; трафик не суммируется.", "cosmic/src/*", "Ручная проверка на COSMIC.")
T("I17.T05.a", "Проверка реальных состояний", "M", "L4", ["^"], "Long actions, локали, темы, размеры; сверка со capture.", "i17-evidence", "Snapshot ↔ evidence совпадают.")

# ───────────────────────────── X: runtime на установке ─────────────────────────────
M("X", "Проверки на установке (отложенный runtime)", "P0", "runtime",
  "Всё, что нельзя доказать без реальной системы. Только по отдельному поручению пользователя.",
  gate="Установка/изменение сети хоста — по явному разрешению")
T("X.01", "Разрешение и среда установки", "S", "D", ["I03.T05.d"],
  "Хост сейчас в состоянии K1 (следы UPD). Выбор: VM-клон или реальный хост; резервная копия; окно работ.",
  "INSTALL-PLAN.md", "Письменное поручение пользователя.", decision="D10")
T("X.02", "Runtime-регрессии I01", "M", "L4", ["X.01"], "W0-1 (K0–K6), W0-2, миграция UPD→CM на живой системе, recover после прерывания.", "i01-evidence/install", "Миграция без потери конфига и подписок.")
T("X.03", "Installation driver I02", "S", "L4", ["X.01"], "Store на реальной ФС: права, fsync, restart.", "i02-evidence/install", "Все проверки driver pass.")
T("X.04", "Installation driver I03", "M", "L4", ["X.01", "I03.T05.e"], "Реальные подписки, negotiation, refresh, отказы; секреты локально.", "i03-evidence/install", "Driver pass; actual UA зафиксирован.")
T("X.05", "Parity CM vs FlClash (I01.T03)", "L", "L4", ["X.04"], "A/B/A на одинаковом полном узле; классификация отказа по стадиям.", "i01-evidence/parity-2", "Причина отказа найдена или UNKNOWN честно.")
T("X.06", "Пакеты, systemd, гонки", "M", "L4", ["X.02"], "pacman/apt хуки, units, реальные процессные гонки, сохранение настоящего конфига.", "x-evidence", "Без регресса относительно 0.2.8.")
T("X.08", "Parity-протокол I01 на адаптере mihomo", "M", "L4", ["I04.T05.a", "X.01"],
  "A/B/A на одинаковом узле: legacy vs CoreAdapter. Revision FlClash core не считать upstream.",
  "i04-evidence/parity", "Результат или явный UNKNOWN.")
T("X.07", "Сводный runtime-отчёт", "S", "L0", ["X.02", "X.03", "X.05", "X.06", "V.05", "X.08"],
  "Собрать результаты установки: I01–I03, срез V, parity адаптера; статусы PASS/FAIL/UNKNOWN без повышения.",
  "X-RUNTIME-REPORT.md", "Каждая отложенная проверка имеет итоговый статус и ссылку на evidence.")

# ───────────────────────────── I18 ─────────────────────────────
M("I18", "Сквозная приёмка и готовность выпуска", "P0", "release", "A01–A20 на реальных приложениях.", gate="G6")
T("I18.T01.a", "Сверка W01–W11 с evidence", "M", "L0", ["I17.T05.a", "X.07"], "Открытые P0/conditional/no-go.", "I18-TRACE.md", "Трассировка полная.")
T("I18.T02.a", "A01–A20 на реальных apps", "XL", "L4", ["^"], "Captures, fault injection, actual executor.", "i18-evidence", "Все критерии pass или явный blocked.")
T("I18.T03.a", "Повтор регрессий", "L", "L4", ["^"], "CM, UPD→CM, core update/rollback, прерванный апгрейд, лимиты ресурсов, долгие сессии.", "i18-evidence/regress", "Без регресса.")
T("I18.T04.a", "Scope выпуска и runbooks", "M", "L0", ["^"], "Ядра/версии/упаковки, ограничения, rollback.", "RELEASE-SCOPE.md", "Нет неподтверждённых гарантий.")
T("I18.T05.a", "Acceptance report и решение", "S", "D", ["^"], "ready/blocked; сборка/публикация — отдельным поручением.", "I18-ACCEPTANCE.md", "Решение пользователя.")

DECISIONS = [
    ("D1", "Группы/правила/провайдеры в native-подписках", "I03.T04.p", "**Принято 2026-10-06:** вариант 2 сейчас, вариант 3 в I04.T04.f — [I03-DECISIONS](I03-DECISIONS-2026-10-06.md)."),
    ("D2", "Политика `skip-cert-verify`", "I03.T04.y", "**Принято 2026-10-06:** явный insecure-признак `tls_verification: Disabled` с подтверждением; с pin — `Pinned`."),
    ("D3", "Смешанный список с неподдержанными строками", "I03.T04.s", "**Принято 2026-10-06:** частичный импорт с перечнем по классам; отказ при порче или 0 узлов."),
    ("D4", "HTTP-клиент с общим deadline", "H.04", "**Принято 2026-10-06:** вариант B (ureq 2 + резолв в потоке с deadline)."),
    ("D5", "YAML-стек", "H.05", "**Принято:** saphyr на пути импорта; `cargo fetch` для H.05.b разрешён 2026-10-06."),
    ("D6", "Среда для сетевого harness (rootless netns или VM)", "H.11", "**Принято 2026-10-06:** rootless netns; smoke в tests/netharness."),
    ("D7", "Принять вертикальный срез V до I04+", "V.01", "**Принято 2026-10-06:** вариант 2 ADR V.01."),
    ("D8", "Evidence-логи вне git", "H.08", "**Принято 2026-10-06:** в git только summary; .gitignore."),
    ("D9", "CI на GitHub", "H.10", "**Принято 2026-10-06:** .github/workflows/check.yml; активен после push."),
    ("D10", "Разрешение на установку и среду", "X.01", "**Принято 2026-10-06:** реальный хост + snapper, INSTALL-PLAN.md; sudo — пользователь."),
]

LANES = {
    "infra": "Инфраструктура", "import": "Импорт", "slice": "Срез", "core": "Ядра", "root": "Root-контроллер",
    "net": "Сеть", "apps": "Приложения", "env": "Окружение", "quality": "Качество", "ui": "Интерфейс",
    "runtime": "Установка", "release": "Выпуск",
}


# ───────────────────────────── проверка и вычисления ─────────────────────────────
def build():
    ids = {t["id"] for t in TASKS}
    assert len(ids) == len(TASKS), "дубль ID"
    errors = []
    for t in TASKS:
        for d in t["depends_on"]:
            if d not in ids and d not in DONE:
                errors.append(f"{t['id']}: неизвестная зависимость {d}")
        assert t["size"] in SIZE and t["level"] in LEVEL, t["id"]
    if errors:
        sys.exit("\n".join(errors))
    by = {t["id"]: t for t in TASKS}
    # топологическая сортировка + волны (ранний старт по числу шагов)
    order, state = [], {}

    def visit(n, stack):
        if state.get(n) == 2:
            return
        if state.get(n) == 1:
            sys.exit("цикл: " + " → ".join(stack + [n]))
        state[n] = 1
        for d in by[n]["depends_on"]:
            if d in by:
                visit(d, stack + [n])
        state[n] = 2
        order.append(n)

    for t in TASKS:
        visit(t["id"], [])
    wave, finish, prev = {}, {}, {}
    for n in order:
        deps = [d for d in by[n]["depends_on"] if d in by]
        wave[n] = 1 + max((wave[d] for d in deps), default=0)
        best = max(deps, key=lambda d: finish[d], default=None)
        finish[n] = SIZE[by[n]["size"]] + (finish[best] if best else 0)
        prev[n] = best
    end = max(finish, key=finish.get)
    path = []
    while end:
        path.append(end)
        end = prev[end]
    path.reverse()
    ready = [t["id"] for t in TASKS if t["id"] not in DONE and all(d in DONE for d in t["depends_on"])]
    early = check_closure(by)
    return by, order, wave, finish, path, ready, early


# Рубеж H — пул независимых работ без закрытия; H.10 опционален, I18.T05.a — конец графа.
POOL_MILESTONES = {"H"}
ALLOWED_SINKS = {"H.10", "I18.T05.a"}


def check_closure(by):
    """Инварианты разбивки относительно DAG-PLAN.json.

    1. Закрывающая (последняя) подзадача рубежа зависит от всех его подзадач.
    2. Она же зависит от закрытия каждого рубежа-предшественника из DAG-PLAN.json.
    3. Нет подзадач, результат которых никто не использует (кроме ALLOWED_SINKS).
    Возвращает намеренные ранние старты: подзадачи, которые начинаются раньше закрытия
    предшественника исходной задачи-родителя.
    """
    old = json.loads((HERE.parent / "DAG-PLAN.json").read_text())
    old_tasks = {t["id"]: t for n in old["nodes"] for t in n["tasks"]}
    anc = {}

    def ancestors(n):
        if n not in anc:
            acc = set()
            for d in by[n]["depends_on"]:
                acc.add(d)
                if d in by:
                    acc |= ancestors(d)
            anc[n] = acc
        return anc[n]

    close = {}
    for t in TASKS:
        close[t["milestone"]] = t["id"]
    errors = []
    for m, c in close.items():
        if m in POOL_MILESTONES:
            continue
        missed = [i for i, t in by.items() if t["milestone"] == m and i != c and i not in ancestors(c)]
        if missed:
            errors.append(f"{m}: закрытие {c} не охватывает {missed}")
    for n in old["nodes"]:
        for d in n["depends_on"]:
            if n["id"] in close and d in close and close[d] not in ancestors(close[n["id"]]):
                errors.append(f"{n['id']}: закрытие {close[n['id']]} не зависит от закрытия {d}")
    used = {d for t in TASKS for d in t["depends_on"]}
    sinks = [i for i in by if i not in used and i not in ALLOWED_SINKS]
    if sinks:
        errors.append(f"подзадачи без потребителя: {sinks}")
    if errors:
        sys.exit("\n".join(errors))
    early = []
    for i in by:
        parent = i.rsplit(".", 1)[0]
        for d in old_tasks.get(parent, {}).get("depends_on", []):
            if d in DONE or d.split(".")[0] == parent.split(".")[0]:
                continue
            target = close.get(d.split(".")[0])
            if target and target not in ancestors(i):
                early.append((i, d))
    return early


def render(by, order, wave, finish, path, ready, early):
    total = sum(SIZE[t["size"]] for t in TASKS)
    crit = sum(SIZE[by[n]["size"]] for n in path)
    L = []
    w = L.append
    w("# CM: детальный DAG оставшейся работы")
    w("")
    w(f"Дата: {DATE}. Сгенерировано `tools/remaining_dag.py` — правки вносить в скрипт, затем перезапускать. "
      "Дополняет [DETAILED-DAG-PLAN](DETAILED-DAG-PLAN.md) (рубежи и gates G0–G6 остаются в силе) и "
      "[точку остановки](WORK-STOP-2026-10-05.md). Машинный граф: [REMAINING-DAG-PLAN.json](REMAINING-DAG-PLAN.json). "
      "Очередь Composer на H.01, H.02 и I03.T04: [COMPOSER-QUEUE-2026-10-06](COMPOSER-QUEUE-2026-10-06.md); "
      "независимое ревью и оракул pinned-источников — Grok: [GROK-QUEUE-2026-10-06](GROK-QUEUE-2026-10-06.md).")
    w("")
    w("## 1. Как читать")
    w("")
    w("- Пять крупных задач каждого рубежа разбиты на подзадачи `Ixx.Tyy.z`. Зависимости заданы на уровне подзадач: "
      "работа, не требующая принятого результата предшественника, может идти раньше (§4.1).")
    w("- Инварианты проверяет генератор: последняя подзадача рубежа зависит от всех его подзадач и от "
      "закрытия каждого рубежа-предшественника из DAG-PLAN.json; подзадач без потребителя нет. "
      "Поэтому закрытие рубежа не может опередить исходный DAG, даже если его отдельные части начинаются раньше. "
      "H — пул без закрытия. Runtime-проверки (L4) кодовых рубежей собираются в X.07 и не блокируют "
      "следующие кодовые рубежи; I18 ждёт X.07.")
    w("- Новые рубежи: **H** (гигиена и инфраструктура проверок), **V** (вертикальный срез, предложение — решение D7), "
      "**X** (runtime-проверки на установке). Они не меняют N = 18 направлений.")
    w("- Размер — относительный вес, не срок: " + ", ".join(f"`{k}`={v}" for k, v in SIZE.items()) +
      " (S ≤ ½ дня, M 1–2 дня, L 3–5 дней, XL — разбить при старте).")
    w("- Уровень доказательства: " + "; ".join(f"`{k}` — {v}" for k, v in LEVEL.items()) + ".")
    w("- Кодово завершены (runtime-приёмка в X): " + ", ".join(f"`{d}`" for d in DONE) + ".")
    w("")
    w("## 2. Сводка")
    w("")
    w(f"- Подзадач: **{len(TASKS)}** в {len(MILESTONES)} рубежах; суммарный вес **{total}**.")
    w(f"- Критический путь: **{len(path)}** подзадач, вес **{crit}** (≈ {crit / total:.0%} от суммы) — "
      "это нижняя граница при неограниченном параллелизме.")
    w(f"- Готовы к старту сейчас: **{len(ready)}** — см. §4.")
    w(f"- Решений, без которых часть графа не двинется: **{len(DECISIONS)}** — см. §5.")
    w("")
    w("| Рубеж | Название | Приоритет | Подзадач | Вес | Вопросы | Gate |")
    w("|---|---|---|---|---|---|---|")
    for m in MILESTONES:
        ts = [t for t in TASKS if t["milestone"] == m["id"]]
        w(f"| {m['id']} | {m['title']} | {m['priority']} | {len(ts)} | {sum(SIZE[t['size']] for t in ts)} | "
          f"{m['questions'] or '—'} | {m['gate'] or '—'} |")
    w("")
    w("## 3. Граф рубежей")
    w("")
    edges = set()
    for t in TASKS:
        for d in t["depends_on"]:
            if d in by and by[d]["milestone"] != t["milestone"]:
                edges.add((by[d]["milestone"], t["milestone"]))
    w("```mermaid")
    w("flowchart LR")
    for m in MILESTONES:
        w(f'  {m["id"].replace("-", "_")}["{m["id"]}: {m["title"]}"]')
    for a, b in sorted(edges):
        w(f"  {a.replace('-', '_')} --> {b.replace('-', '_')}")
    w("```")
    w("")
    w("### Ближайшая работа: I03.T04 и H")
    w("")
    w("```mermaid")
    w("flowchart TD")
    near = [t for t in TASKS if t["id"].startswith(("I03.", "H.", "V."))]
    for t in near:
        nid = t["id"].replace(".", "_").replace("-", "_")
        w(f'  {nid}["{t["id"]}<br/>{t["title"]} · {t["size"]}"]')
    for t in near:
        for d in t["depends_on"]:
            if d in by and by[d] in near:
                w(f"  {d.replace('.', '_').replace('-', '_')} --> {t['id'].replace('.', '_').replace('-', '_')}")
    w("```")
    w("")
    w("## 4. Что можно начинать сейчас")
    w("")
    w("Все зависимости этих подзадач закрыты. Решение-подзадачи (`D`) — вопросы пользователю, их стоит задать первыми.")
    w("")
    for n in ready:
        t = by[n]
        w(f"- `{n}` · {t['title']} · `{t['size']}` · {t['level']}" + (f" · решение {t['decision']}" if t["decision"] else ""))
    w("")
    w("### 4.1. Намеренно ранние старты")
    w("")
    w("Эти подзадачи начинаются до закрытия предшественника своей исходной задачи. Это проектирование, спецификации "
      "и подготовка, которые не опираются на принятый результат; закрытие их рубежа всё равно ждёт предшественника.")
    w("")
    for a, b in early:
        w(f"- `{a}` · {by[a]['title']} — до закрытия `{b}`")
    w("")
    w("### Критический путь")
    w("")
    w(" → ".join(f"`{n}`" for n in path))
    w("")
    w("Ускорить можно только сокращением этой цепочки: раньше получить среду harness (D6), не блокировать I06 на "
      "полном I04 (проектирование I06.T01–T02 уже вынесено раньше), держать I10.T01 и I17-D параллельно.")
    w("")
    w("## 5. Решения")
    w("")
    w("| ID | Решение | Где блокирует | Комментарий |")
    w("|---|---|---|---|")
    for d in DECISIONS:
        w(f"| {d[0]} | {d[1]} | `{d[2]}` | {d[3]} |")
    w("")
    w("Вопросы Q01–Q27 из [QUESTIONS](QUESTIONS.md) указаны у рубежей и решаются перед своими gates.")
    w("")
    w("## 6. Подзадачи")
    for m in MILESTONES:
        w("")
        w(f"### {m['id']}. {m['title']}")
        w("")
        w(f"**Приоритет:** {m['priority']}. **Поток:** {LANES[m['lane']]}. "
          + (f"**Вопросы:** {m['questions']}. " if m["questions"] else "")
          + (f"**Gate:** {m['gate']}." if m["gate"] else ""))
        w("")
        w(m["goal"])
        for t in TASKS:
            if t["milestone"] != m["id"]:
                continue
            w("")
            deps = ", ".join(f"`{d}`" for d in t["depends_on"]) or "—"
            mark = " ★" if t["id"] in path else ""
            w(f"#### `{t['id']}` {t['title']}{mark}")
            w("")
            w(f"`{t['size']}` · {t['level']} · волна {wave[t['id']]} · зависит от: {deps}"
              + (f" · решение **{t['decision']}**" if t["decision"] else ""))
            w("")
            w(f"- **Что:** {t['what']}")
            w(f"- **Выход:** {t['outputs']}")
            w(f"- **Готово, когда:** {t['done_when']}")
    w("")
    w("★ — подзадача на критическом пути.")
    w("")
    w("## 7. Правила, которые не меняются")
    w("")
    w("- Кодовое завершение и runtime-приёмка учитываются раздельно; L1 не закрывает NET/APP-гарантии.")
    w("- Неподдержанное — явный Unsupported, не молчаливое удаление; секреты не попадают в evidence и ошибки.")
    w("- Установка, сборка пакета, изменение сети хоста и внешние публикации — только по отдельному поручению (D9, D10).")
    w("- При изменении schema/core/route/DNS/adapter повторяются существенные проверки зависимых подзадач (reopen = новая revision evidence, не обратная стрелка).")
    w("")
    return "\n".join(L)


def main():
    by, order, wave, finish, path, ready, early = build()
    md = render(by, order, wave, finish, path, ready, early)
    data = {
        "schema_version": 2, "date": DATE, "supersedes_granularity_of": "DAG-PLAN.json",
        "size_weights": SIZE, "evidence_levels": LEVEL, "done": DONE,
        "milestones": MILESTONES,
        "tasks": [dict(t, wave=wave[t["id"]], critical=t["id"] in path) for t in TASKS],
        "decisions": [dict(id=a, title=b, blocks=c, note=d) for a, b, c, d in DECISIONS],
        "critical_path": path, "ready_now": ready,
        "early_starts": [dict(task=a, before_closure_of=b) for a, b in early],
    }
    js = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    if "--check" in sys.argv:
        stale = OUT_MD.read_text() != md or OUT_JSON.read_text() != js
        sys.exit("план устарел: перезапустите генератор" if stale else 0)
    OUT_MD.write_text(md)
    OUT_JSON.write_text(js)
    print(f"{len(TASKS)} подзадач, критический путь {len(path)}, готовы сейчас {len(ready)}")


if __name__ == "__main__":
    main()
