#!/usr/bin/env python3
"""Проверка тестов роли 2 на покрытие значений рёбер и на пустые проверки.

Каждое значение ребра имеет номер (с единицы, в порядке списка `values` в <CODE>-DAG.json).
Проверка значения помечается меткой `[РЕБРО.N]`:
- Rust: строка-комментарий `// [H05.2]`, не дальше 12 строк за ней — `assert`;
- стенд (Python): первый аргумент `ok("H12.3", ...)`.

Отказ, если:
- у значения нет ни одной метки и его нет в списке заблокированных;
- метка указывает на несуществующее ребро или номер;
- проверка заведомо истинна или принимает несколько исходов;
- тест помечен `#[ignore]` без записи о дефекте (с `--strict` — любой `#[ignore]` и любой `defect(` в стенде);
- заблокировано значение ребра уровня L1 либо заблокированных больше лимита.

Запуск: python3 docs/design/cm-network-manager/tools/test_lint.py [--dags AB,FIN] [--report] [--strict]
"""

import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DOCS = HERE.parent
ROOT = DOCS.parents[2]
TESTS = ROOT / "tests"
BLOCKED_LIMIT = 12
MARK = re.compile(r"\[([A-Z]{1,2}\d{2})\.(\d+)\]")
PY_OK = re.compile(r"\b(?:ok|defect)\(\s*[\"']([A-Z]{1,2}\d{2})(?:\.(\d+))?[\"']")

RUST_BAD = [
    (re.compile(r"assert!\(\s*true\b"), "assert!(true)"),
    (re.compile(r"\|\|\s*true\b"), "|| true"),
    (re.compile(r"assert!\([^;]*>=\s*0\s*[,)]"), "сравнение >= 0"),
]
PY_BAD = [
    (re.compile(r"\bor\s+True\b"), "or True"),
    (re.compile(r",\s*True\s*(,|\))"), "условие True"),
    (re.compile(r">=\s*0\s*(,|\))"), "сравнение >= 0"),
    (re.compile(r"\bor\b"), "несколько исходов через or"),
    (re.compile(r"\bin\s*\("), "несколько исходов через in (...)"),
]


def load_edges(codes):
    edges = {}
    for code in codes:
        path = DOCS / f"{code}-DAG.json"
        for edge in json.loads(path.read_text())["edges"]:
            edges[edge["id"]] = (len(edge["values"]), edge["level"], code)
    return edges


def py_calls(text):
    """Вызовы ok(...) целиком, с учётом переноса строк."""
    for match in re.finditer(r"\b(?:ok|defect)\(", text):
        depth, index = 0, match.end() - 1
        while index < len(text):
            char = text[index]
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0:
                    break
            index += 1
        yield text.count("\n", 0, match.start()) + 1, text[match.start():index + 1]


def main():
    codes = ["AB", "FIN"]
    if "--dags" in sys.argv:
        codes = sys.argv[sys.argv.index("--dags") + 1].split(",")
    edges = load_edges(codes)
    strict = "--strict" in sys.argv
    errors, seen = [], set()

    for path in sorted(p for code in codes for p in TESTS.glob(f"audit_{code.lower()}_*.rs")):
        lines = path.read_text().splitlines()
        for number, line in enumerate(lines, 1):
            for bad, name in RUST_BAD:
                if bad.search(line):
                    errors.append(f"{path.name}:{number}: пустая проверка ({name})")
            if "#[ignore" in line and not re.search(r"дефект [A-Z]{1,2}\d{2}", line):
                errors.append(f"{path.name}:{number}: #[ignore] без записи о дефекте")
            elif "#[ignore" in line and strict:
                errors.append(f"{path.name}:{number}: открытый дефект")
            if not line.lstrip().startswith("//"):
                continue
            for edge, value in MARK.findall(line):
                if edge not in edges:
                    continue  # метки прошлых пакетов без DAG в списке
                if not 1 <= int(value) <= edges[edge][0]:
                    errors.append(f"{path.name}:{number}: у ребра {edge} нет значения {value}")
                    continue
                if not any("assert" in follow for follow in lines[number:number + 12]):
                    errors.append(f"{path.name}:{number}: за меткой [{edge}.{value}] нет assert")
                    continue
                seen.add((edge, int(value)))

    for path in sorted(TESTS.glob("*_stand.py")):
        text = path.read_text()
        for number, call in py_calls(text):
            match = PY_OK.match(call)
            if not match or match.group(1) not in edges:
                continue
            edge, value = match.group(1), match.group(2)
            if value is None:
                errors.append(f"{path.name}:{number}: ok(\"{edge}\") без номера значения")
                continue
            if not 1 <= int(value) <= edges[edge][0]:
                errors.append(f"{path.name}:{number}: у ребра {edge} нет значения {value}")
                continue
            if call.startswith("defect("):
                if strict:
                    errors.append(f"{path.name}:{number}: [{edge}.{value}] открытый дефект")
                seen.add((edge, int(value)))
                continue
            condition = call.split(",", 2)[2] if call.count(",") >= 2 else ""
            bad_name = next((name for bad, name in PY_BAD if bad.search(condition)), None)
            if bad_name:
                errors.append(f"{path.name}:{number}: [{edge}.{value}] пустая проверка ({bad_name})")
                continue
            seen.add((edge, int(value)))

    blocked = set()
    for code in codes:
        path = TESTS / "fixtures" / code.lower() / "blocked.json"
        if not path.exists():
            continue
        for item in json.loads(path.read_text()):
            edge, value = item["edge"], int(item["n"])
            if edge not in edges or not 1 <= value <= edges[edge][0]:
                errors.append(f"{path.name}: нет значения {edge}.{value}")
            elif edges[edge][1] == "L1":
                errors.append(f"{path.name}: {edge}.{value} — значение уровня L1 блокировать нельзя")
            elif not item.get("reason") or not item.get("driver"):
                errors.append(f"{path.name}: {edge}.{value} — нужны reason и driver")
            else:
                blocked.add((edge, value))
    if len(blocked) > BLOCKED_LIMIT:
        errors.append(f"заблокировано {len(blocked)} значений, лимит {BLOCKED_LIMIT}")

    missing = [(edge, value) for edge, (count, _, _) in sorted(edges.items())
               for value in range(1, count + 1) if (edge, value) not in seen and (edge, value) not in blocked]
    total = sum(count for count, _, _ in edges.values())
    if "--report" in sys.argv:
        by_edge = {}
        for edge, value in missing:
            by_edge.setdefault(edge, []).append(value)
        for edge, values in by_edge.items():
            print(f"нет проверки: {edge} значения {values}")
    print(f"значений {total}, проверено {len(seen)}, заблокировано {len(blocked)}, без проверки {len(missing)}, "
          f"замечаний {len(errors)}")
    for error in errors[:200]:
        print(error)
    sys.exit(1 if errors or missing else 0)


if __name__ == "__main__":
    main()
