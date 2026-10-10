#!/usr/bin/env python3
"""Общий рендер направления в формате «вершины и рёбра-контракты».

Направление описывается данными (см. i06_dag.py):
  * DIRECTIONS — [(код, название, цель)];
  * V — вершины: id, direction, title (одно предложение), size, level, deps, files, spec;
  * E — рёбра: id, source, target, contract, check, values, level;
  * CFG — имена файлов, тексты вступления и правила двух ролей.

Модуль проверяет граф и пишет <CODE>-DAG.md/.json, GROK-<CODE>-CODE.md, TESTS-<CODE>.md.
"""

import json
import sys


def check(V, E, final):
    ids = [v["id"] for v in V]
    errors = []
    if len(ids) != len(set(ids)):
        errors.append("повтор id вершины")
    known = set(ids)
    for v in V:
        for d in v["deps"]:
            if d not in known:
                errors.append(f"{v['id']}: неизвестная зависимость {d}")
    edge_ids = [e["id"] for e in E]
    if len(edge_ids) != len(set(edge_ids)):
        errors.append("повтор id ребра")
    for e in E:
        if e["source"] not in known or e["target"] not in known:
            errors.append(f"{e['id']}: неизвестная вершина")
        if not e["values"]:
            errors.append(f"{e['id']}: нет значений проверки")
    out = {v: [e["target"] for e in E if e["source"] == v] for v in ids}
    for v in ids:
        if v != final and not out[v]:
            errors.append(f"{v}: нет исходящего ребра (нечем проверить)")
    pairs = {(e["source"], e["target"]) for e in E}
    for v in V:
        for d in v["deps"]:
            if (d, v["id"]) not in pairs:
                errors.append(f"зависимость {d} → {v['id']} без ребра-контракта")
    graph = {v: set(out[v]) | {x["id"] for x in V if v in x["deps"]} for v in ids}
    state = {}

    def visit(n):
        if state.get(n) == 1:
            errors.append(f"цикл через {n}")
            return
        if state.get(n) == 2:
            return
        state[n] = 1
        for m in graph[n]:
            visit(m)
        state[n] = 2

    for n in ids:
        visit(n)
    reach = {final}
    changed = True
    while changed:
        changed = False
        for v in ids:
            if v not in reach and graph[v] & reach:
                reach.add(v)
                changed = True
    for v in ids:
        if v not in reach:
            errors.append(f"{v} не доходит до {final}")
    if errors:
        sys.exit("\n".join(errors))


def topo(V):
    order, seen = [], set()
    by = {v["id"]: v for v in V}

    def go(n):
        if n in seen:
            return
        seen.add(n)
        for d in by[n]["deps"]:
            go(d)
        order.append(n)

    for v in V:
        go(v["id"])
    return order


def node(vid):
    return vid.replace(".", "_").replace("-", "_")


def md_dag(DIRECTIONS, V, E, CFG):
    by = {v["id"]: v for v in V}
    code = CFG["code"]
    lines = [f"# {CFG['title']} — DAG", "",
             f"Дата: {CFG['date']}. Источник: [tools/{CFG['source']}](tools/{CFG['source']}) (правится только он). {CFG['links']}", "",
             "Схема:",
             "- **направление** — группа вершин одной темы;",
             "- **вершина** — одна задача (одно предложение);",
             "- **ребро** u → v — контракт: что u гарантирует v, как это проверить и с какими значениями.", "",
             "Роли:",
             f"- **роль 1** (Grok) пишет код всех вершин за один проход — [GROK-{code}-CODE.md](GROK-{code}-CODE.md);",
             f"- **роль 2** пишет тесты и проверки по рёбрам следующей итерацией — [TESTS-{code}.md](TESTS-{code}.md).", ""]
    lines += CFG["dag_intro"] + ["", "## Направления", "", "| | Направление | Цель | Вершины |", "|---|---|---|---|"]
    for d, title, goal in DIRECTIONS:
        lines.append(f"| {d} | {title} | {goal} | {', '.join(v['id'] for v in V if v['direction'] == d)} |")
    lines += ["", "## Граф", "", "Стрелка — ребро с контрактом, подпись — id ребра.", "", "```mermaid", "flowchart LR"]
    for d, title, _ in DIRECTIONS:
        lines.append(f"  subgraph {d}[\"{d}: {title}\"]")
        for v in V:
            if v["direction"] == d:
                lines.append(f"    {node(v['id'])}[\"{v['id']}<br/>{v['title']}\"]")
        lines.append("  end")
    for e in E:
        lines.append(f"  {node(e['source'])} -->|{e['id']}| {node(e['target'])}")
    lines += ["```", "", "## Вершины", "", "| Вершина | Задача | Размер | Уровень | Зависит от | Файлы |", "|---|---|---|---|---|---|"]
    for vid in topo(V):
        v = by[vid]
        lines.append(f"| `{v['id']}` | {v['title']} | {v['size']} | {v['level']} | {', '.join(v['deps']) or '—'} | {', '.join('`' + f + '`' for f in v['files'])} |")
    lines += ["", "## Рёбра: контракт, проверка, значения", ""]
    for e in E:
        lines += [f"### {e['id']} · {e['source']} → {e['target']} ({e['level']})", "",
                  f"- **Контракт:** {e['contract']}", f"- **Как проверить:** {e['check']}", "- **Значения:**"]
        lines += [f"  - {x}" for x in e["values"]]
        lines.append("")
    return "\n".join(lines)


def md_code(DIRECTIONS, V, E, CFG):
    by = {v["id"]: v for v in V}
    code = CFG["code"]
    lines = [f"# Grok: {CFG['title']} — весь код за один проход", "",
             f"Дата: {CFG['date']}. Координатор: Claude. Граф и контракты: [{code}-DAG.md]({code}-DAG.md). {CFG['links']}", "",
             "## Задача", ""] + CFG["code_task"] + ["",
             "Тесты пишет роль 2 по рёбрам. Поэтому точные значения из рёбер — часть спецификации, а не пожелание.", "",
             "## Правила", "", CFG["rules_code"], "", "## Вершины по порядку", ""]
    for vid in topo(V):
        v = by[vid]
        if v["id"] == CFG["final"]:
            continue
        lines += [f"### {v['id']} — {v['title']}", "",
                  f"Файлы: {', '.join('`' + f + '`' for f in v['files'])}. Зависит от: {', '.join(v['deps']) or '—'}.", "",
                  v["spec"], ""]
        outs = [e for e in E if e["source"] == v["id"]]
        if outs:
            lines.append("Контракты, которые проверит роль 2:")
            for e in outs:
                lines.append(f"- **{e['id']} → {e['target']}:** {e['contract']}")
                lines += [f"  - {x}" for x in e["values"]]
            lines.append("")
    lines += ["## Сдача", "",
              "Отчёт:",
              "- по каждой вершине: сделано или открыто с причиной;",
              "- `git diff --stat` по существующим файлам;",
              "- EXIT и счётчики gate;",
              "- список мест, где спецификация допускала толкование, и выбранное толкование.", ""]
    return "\n".join(lines)


def md_tests(DIRECTIONS, V, E, CFG):
    code = CFG["code"]
    lines = [f"# Роль 2: тесты и проверки — {CFG['title']} (следующей итерацией)", "",
             f"Дата: {CFG['date']}. Начинать после сдачи кода ролью 1 ([GROK-{code}-CODE.md](GROK-{code}-CODE.md)). Граф: [{code}-DAG.md]({code}-DAG.md).", "",
             "## Задача", ""] + CFG["tests_task"] + ["", "## Правила", "", CFG["rules_tests"], "",
             "## Файлы", "", "| Файл | Рёбра |", "|---|---|"]
    files = {}
    for e in E:
        f = e["check"].split("`")[1] if "`" in e["check"] else e["check"]
        files.setdefault(f, []).append(e["id"])
    for f, ids in files.items():
        lines.append(f"| `{f}` | {', '.join(ids)} |")
    lines += ["", "## Рёбра", ""]
    for e in E:
        lines += [f"### {e['id']} · {e['source']} → {e['target']} ({e['level']})", "",
                  f"- **Контракт:** {e['contract']}", f"- **Как проверить:** {e['check']}", "- **Ожидаемые значения:**"]
        lines += [f"  - [ ] {x}" for x in e["values"]]
        lines.append("")
    lines += ["## Сдача", ""] + CFG["tests_report"] + [""]
    return "\n".join(lines)


def tasks_for_remaining(V, E, CFG):
    out = []
    by = {v["id"]: v for v in V}
    for vid in topo(V):
        v = by[vid]
        outs = [e for e in E if e["source"] == vid]
        done = "; ".join(f"{e['id']}: {e['contract']}" for e in outs) or CFG["final_done"]
        deps = list(v["deps"]) + list(v.get("external", []))
        out.append(dict(id=vid, title=v["title"].rstrip("."), size=v["size"], level=v["level"], deps=deps,
                        what=f"{CFG['code']}-DAG.md, вершина {vid}.", out=", ".join(v["files"]), done=done))
    return out


def write(out_dir, DIRECTIONS, V, E, CFG):
    code = CFG["code"]
    (out_dir / f"{code}-DAG.md").write_text(md_dag(DIRECTIONS, V, E, CFG))
    (out_dir / f"GROK-{code}-CODE.md").write_text(md_code(DIRECTIONS, V, E, CFG))
    (out_dir / f"TESTS-{code}.md").write_text(md_tests(DIRECTIONS, V, E, CFG))
    (out_dir / f"{code}-DAG.json").write_text(json.dumps(
        {"schema_version": 1, "date": CFG["date"], "directions": [dict(id=d, title=t, goal=g) for d, t, g in DIRECTIONS],
         "vertices": V, "edges": E}, ensure_ascii=False, indent=2) + "\n")
    print(f"{code}: {len(V)} вершин, {len(E)} рёбер → {code}-DAG.md, GROK-{code}-CODE.md, TESTS-{code}.md, {code}-DAG.json")
