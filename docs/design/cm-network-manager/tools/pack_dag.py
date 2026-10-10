#!/usr/bin/env python3
"""Единый пакет для Grok: всё, что можно закодировать сейчас, одним DAG.

Состав: рубеж I06 (i06_dag.py) + доставка ядер, Xray, сеть приложения (pack_core.py)
+ приложения, регион, качество, интерфейс (pack_apps.py) + рёбра между направлениями
и сквозная приёмка.

Пишет PACK-DAG.md/.json, GROK-PACK-CODE.md, TESTS-PACK.md.
Запуск: python3 docs/design/cm-network-manager/tools/pack_dag.py [--check]
"""

import copy
import sys
from pathlib import Path

import direction_kit as kit
import i06_dag
import pack_apps
import pack_core

HERE = Path(__file__).resolve().parent
OUT = HERE.parent
FINAL = "PACK.Z"

DIRECTIONS = ([(d, f"Контроллер · {t}", g) for d, t, g in i06_dag.DIRECTIONS if d != "Z"]
              + pack_core.DIRECTIONS + pack_apps.DIRECTIONS
              + [("Z", "Приёмка", "Отрицательные проверки контроллера и сквозной сценарий: приложение в своей сети выходит только через свой туннель.")])

V = copy.deepcopy(i06_dag.V) + copy.deepcopy(pack_core.V) + copy.deepcopy(pack_apps.V)
E = copy.deepcopy(i06_dag.E) + copy.deepcopy(pack_core.E) + copy.deepcopy(pack_apps.E)

# Зависимости между направлениями пакета становятся внутренними.
ids = {v["id"] for v in V}
for v in V:
    v.setdefault("covers", [])
    inner = [d for d in v.get("external", []) if d in ids]
    v["deps"] = list(v["deps"]) + inner
    v["external"] = [d for d in v.get("external", []) if d not in ids]

V.append(dict(
    id=FINAL, direction="Z", title="Принять пакет сквозным сценарием.", size="L", level="L2",
    deps=["I06.T05.a", "I05.D4", "I05.X2", "I08.N6", "I09.N7", "I09.N8", "I11.A3", "I11.A5", "I12.A7",
          "I14.R2", "I14.R3", "I16.Q2", "I17.U2"],
    files=["docs/design/cm-network-manager/pack-evidence/<дата>/summary.json"],
    spec="Вершина роли 2: кода нет. Закрывается, когда все рёбра проверены, обходов нет и сквозной сценарий Z02 проходит.",
    external=[], covers=[]))


def edge(eid, u, v, contract, check, values, level="L1"):
    E.append(dict(id=eid, source=u, target=v, contract=contract, check=check, values=values, level=level))


edge("X01", "I06.I3", "I05.X2",
     "Xray запускается под uid вызывающего тем же механизмом, что mihomo.",
     "`tests/audit_pack_xray.rs`: внутри `unshare --user --map-users` с `CM_TEST_XRAY`.",
     ["XrayWorker с set_run_as(uid 1): процесс ядра имеет Uid 1 и CapEff 0 в /proc/<pid>/status",
      "без set_run_as поведение прежнее (uid запускающего)"], level="L2")
edge("X02", "I06.W2", "I08.N6",
     "Операции сети проходят тот же порядок проверок, что операции worker: кадр → личность → права → владелец.",
     "`tests/audit_pack_net_ops.rs`.",
     ["net_apply от peer, завершившегося перед кадром → peer_changed, RecordingExec пуст",
      "net_apply с лишним полем `\"index\":5` в op → bad_frame: номер туннеля нельзя задать из запроса",
      "net_apply и net_revert больше не отвечают unsupported"])
edge("X03", "I06.W2", "I11.A3",
     "Запуск приложения — операция класса App со своим действием polkit.",
     "`tests/audit_pack_app_ops.rs`.",
     ["Authorizer получает \"io.github.cm.app\"; при отказе → denied, процесс не создан",
      "app_launch больше не отвечает unsupported"])
edge("X04", "I08.N6", "I11.A3",
     "Приложение получает сеть только того экземпляра, который принадлежит вызывающему.",
     "`tests/audit_pack_app_ops.rs`: два uid (`--map-users`).",
     ["uid B: app_launch(browser), сеть browser создана uid A → not_running; процесс не создан",
      "дескриптор netns открывается по номеру из аренды владельца, а не по имени из кадра"], level="L2")
edge("X05", "I06.S1", "I11.A5",
     "Клиент говорит с контроллером тем же кадром, что описан в протоколе.",
     "`tests/audit_pack_app.rs`.",
     ["controller::client::call шлёт одну строку JSON версии 1 с id из 16 hex-символов и читает одну строку ответа",
      "ответ длиннее 64 КиБ или не JSON → ошибка клиента, код выхода `cm app run` 4"])
edge("X06", "I06.I3", "I11.A2",
     "План запуска несёт RunAs вызывающего без изменений.",
     "`tests/audit_pack_app.rs`.",
     ["plan(..., RunAs{uid 1000, gid 1000, groups [1000, 998]}, ...).run_as == тот же RunAs"])
edge("Z01", "I06.T05.a", FINAL,
     "Контроллер принят: отрицательные проверки I06 пройдены до сквозного сценария.",
     "`i06-evidence/<дата>/summary.json`.",
     ["все рёбра C01–C16 — PASS; обходов нет"], level="L2")
edge("Z02", "I11.A3", FINAL,
     "Сквозной сценарий: приложение в своём netns выходит наружу только через TUN своего worker-а; при гибели worker-а трафик блокируется, а не идёт напрямую.",
     "`tests/audit_pack_e2e.rs`: `unshare -rnm`, `cm controller serve` с `SystemExec` и `CM_TEST_MIHOMO`; «интернет» — третий netns с HTTP-сервером, как в `tools/packet_flow_lab.sh` (лаборатория координатора, результаты ниже — ожидаемые).",
     ["net_apply → worker_start (mode direct) → app_launch `curl -q -s --noproxy '*' -m 5 http://198.51.100.2:8080/` → HTTP 200",
      "счётчик `iifname \"cmv0h\" oifname \"cmtun0\"` > 0; счётчики `\"cmv*\" counter drop` == 0; `/connections` worker-а показывает downloadTotal > 0",
      "kill -9 процесса ядра → тот же запрос: тайм-аут (curl rc 28), HTTP-кода нет; на интерфейсе «интернета» нет пакетов с адреса 10.213.0.2",
      "удаление cmtun0 (`ip link del`) → запрос по-прежнему не проходит: в таблице 100 остаётся `blackhole default metric 200`",
      "контроль: без правила iif и без таблицы cm (и с NAT наружу) тот же запрос даёт 200 — блокируют именно правила CM",
      "DNS: `getent hosts example.test` внутри netns уходит на 198.18.0.2 (пакеты на cmtun0, порт 53), на «интернет»-интерфейсе нет DNS-пакетов с адреса 10.213.0.2",
      "net_revert → нет cmv0h, cmtun0, netns cm-0, правила priority 1000 и таблицы 100; таблица cm содержит только два запрета",
      "curl вызывается с `-q`: `~/.curlrc` пользователя может задавать прокси"], level="L2")

CFG = dict(
    code="PACK",
    title="Единый пакет: контроллер, ядра, сеть приложения, запуск, регион, качество, интерфейс",
    date="2026-10-10",
    source="pack_dag.py",
    final=FINAL,
    links="Основания: [ADR-CONTROLLER.md](ADR-CONTROLLER.md), [ADR-WORKER-TOPOLOGY.md](ADR-WORKER-TOPOLOGY.md), [ADR-TUN-OWNERSHIP.md](ADR-TUN-OWNERSHIP.md), [I05-GAPS.md](I05-GAPS.md), [ADR-BROWSER-IDENTITY.md](ADR-BROWSER-IDENTITY.md), [ADR-PACKET-FLOW.md](ADR-PACKET-FLOW.md).",
    dag_intro=[
        "Пакет собирает всё, что можно закодировать сейчас без новых решений пользователя и без проверок на установленной системе.",
        "Данные пакета — `tools/i06_dag.py`, `tools/pack_core.py`, `tools/pack_apps.py`; этот документ — их объединение.",
        "",
        "**Путь данных приложения проверен лабораторией** (`tools/packet_flow_lab.sh`, 2026-10-10, mihomo 1.19.32):",
        "- приложение живёт в своём netns с одним veth; на хосте правило `iif veth → таблица → TUN worker-а`; worker остаётся в сети хоста;",
        "- запрос из netns проходит через TUN (HTTP 200, ядро насчитало трафик);",
        "- после `kill -9` worker-а и после удаления TUN запрос не проходит (тайм-аут): маршрут `blackhole` удерживает блокировку;",
        "- без правил CM прямой путь существует, то есть блокируют именно правила.",
        "",
        "**Что в пакет не вошло и почему:**",
        "- режимы хоста и outer egress (I07): CM должен забрать маршруты у нынешнего VPN хоста; нужен отдельный packet-flow для хоста и проверка на установленной системе;",
        "- путь данных Xray для приложений (I05.T02): у Xray нет TUN-режима, проверенного лабораторией; в пакете Xray работает только как прокси на порту;",
        "- локальные сети и сервисы приложений (I08.T01.a): решение Q09 за пользователем; сейчас всё, кроме своего туннеля, запрещено;",
        "- упаковки Flatpak и AppImage (I13), границы HOME и keyring (I15.T02): нужны замеры на настоящих приложениях;",
        "- каталог устройств (I15-R.T04–T05): решения Q26/Q27 за пользователем;",
        "- cgroup-область на сессию и запись `Session` в Store при запуске: нужен делегированный cgroup или `systemd-run`;",
        "- проверки на установке (X), приёмка (I18).",
        "",
        "Столбец «Закрывает» в брифе связывает вершину с подзадачей общего плана.",
    ],
    code_task=[
        "Реализовать все вершины ниже в порядке зависимостей. Это пять новых модулей (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`), два подмодуля ядер (`src/core/delivery/`, `src/core/xray/`), представление статуса и небольшие правки существующих файлов.",
        "",
        "Порядок работы и сдачи — волнами, каждая волна собирается и проходит gate отдельно:",
        "1. **Контроллер:** I06.P1 … I06.S1.",
        "2. **Ядра:** I05.D1 … I05.D4, I05.X1, I05.X2.",
        "3. **Сеть приложения:** I08.N1 … I08.N6, I09.N7, I09.N8.",
        "4. **Приложения:** I11.A1 … I11.A5, I12.A6, I12.A7.",
        "5. **Чистые модули:** I14.R1 … R3, I16.Q1, Q2, I17.U1, U2 — не зависят от волн 1–4 и могут идти первыми.",
        "",
        "Если волна упирается в противоречие спецификации, остановись на ней, сдай сделанное и опиши противоречие. Не придумывай обходной путь в P0-коде (контроллер, сеть).",
    ],
    rules_code=i06_dag.CFG["rules_code"]
    .replace("разрешён только для новых файлов `src/controller/*.rs`; добавить `src/controller` в `find` строки `NEW_MODULE_FMT` в `tests/check_audit.sh`.",
             "разрешён только для новых файлов (`src/controller/`, `src/net/`, `src/app/`, `src/env/`, `src/quality/`, `src/core/delivery/`, `src/core/xray/`, `src/core/process.rs`, `src/status/tunnels.rs`); добавить эти каталоги и файлы в проверку формата в `tests/check_audit.sh`.")
    .replace("(`src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`, `src/helper/auth.rs`, `src/core/unit.rs`, `src/core/mihomo/lifecycle.rs`, `tests/check_audit.sh`)",
             "(`src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`, `src/helper/auth.rs`, `src/core/mod.rs`, `src/core/unit.rs`, `src/core/leases.rs`, `src/core/mihomo/lifecycle.rs`, `src/core/mihomo/config.rs`, `src/identity/plan.rs`, `src/status.rs`, `src/tui/mock_tunnels.rs`, `cosmic/src/tunnel_icons.rs`, `tests/check_audit.sh`)")
    .replace("4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога `--base` теста. Настоящий `pkcheck` в тестах не вызывается.",
             "4. Без установки, `systemctl`, изменения сети хоста и файлов вне каталога `--base` теста. Настоящий `pkcheck` не вызывается. Команды `ip` и `nft` выполняются только внутри `unshare -rn`; на хосте — никогда. Загрузка ядер из сети в этой итерации не выполняется (только код и подменный `Download`).")
    + "\n10. **Сеть — fail-closed по построению.** Любое изменение порядка команд из I08.N2 или текста таблицы из I08.N3 — отказ при ревью: порядок проверен лабораторией.",
    tests_task=[
        "Написать за один проход проверки всех рёбер пакета:",
        "- рёбра C01–C16 — контроллер (те же, что в [TESTS-I06.md](TESTS-I06.md));",
        "- K01–K16 — ядра и сеть; M01–M16 — приложения, регион, качество, интерфейс; X01–X06 — связи между направлениями;",
        "- Z02 — сквозной сценарий в `unshare -rnm` с настоящим ядром.",
        "Значения в рёбрах — ожидаемые результаты.",
    ],
    rules_tests=i06_dag.CFG["rules_tests"]
    .replace("Тесты лежат в `tests/audit_i06_*.rs`.", "Тесты лежат в `tests/audit_i06_*.rs` и `tests/audit_pack_*.rs`, фикстуры — в `tests/fixtures/pack/`.")
    .replace("`docs/design/cm-network-manager/i06-evidence/<дата>/summary.json`", "`docs/design/cm-network-manager/i06-evidence/<дата>/summary.json` и `pack-evidence/<дата>/summary.json`")
    + "\n10. Сценарии с `ip netns` требуют записи в `/run/netns`: запускать в `unshare -rnm` и монтировать tmpfs на `/run` внутри. `curl` — только с `-q` и `--noproxy '*'`.\n11. Проверки с Xray — при `CM_TEST_XRAY=$HOME/.cache/cm-cores/xray/xray`; без переменной печатают SKIPPED и не засчитываются.",
    tests_report=[
        "- Отчёт: таблица «ребро → PASS/FAIL → тест», дефекты в формате `<ребро>: ожидалось …, получено …`; обходы контроллера и утечки трафика мимо туннеля — P0, первыми.",
        "- `i06-evidence/<дата>/summary.json` и `pack-evidence/<дата>/summary.json`.",
    ],
    final_done="Все рёбра пакета проверены, обходов и утечек нет, evidence записан.",
)


def covers_table():
    lines = ["", "## Что закрывает пакет в общем плане", "", "| Вершина | Закрывает подзадачи |", "|---|---|"]
    for v in V:
        if v.get("covers"):
            lines.append(f"| `{v['id']}` | {', '.join(v['covers'])} |")
    lines.append("")
    lines.append("Вершины `I06.*` заменяют I06.T01.b–T04.a; `I06.T05.a` — приёмка контроллера.")
    return "\n".join(lines)


def main():
    kit.check(V, E, FINAL)
    if "--check" in sys.argv:
        print(f"PACK: {len(V)} вершин, {len(E)} рёбер — граф корректен")
        return
    kit.write(OUT, DIRECTIONS, V, E, CFG)
    for name in ("PACK-DAG.md", "GROK-PACK-CODE.md"):
        path = OUT / name
        path.write_text(path.read_text() + covers_table() + "\n")


if __name__ == "__main__":
    main()
