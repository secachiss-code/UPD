#!/usr/bin/env python3
"""Полный план оставшихся работ для Grok 4.7 из REMAINING-DAG-PLAN.json.

Исполнитель по умолчанию — Grok; ревью — координатор. Решения пользователя и работы на
реальном хосте помечены отдельно. Запуск после remaining_dag.py:
  python3 docs/design/cm-network-manager/tools/grok_plan.py
"""

import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PLAN = json.loads((HERE.parent / "REMAINING-DAG-PLAN.json").read_text())
OUT = HERE.parent / "GROK-PLAN-FULL-2026-10-06.md"

DONE = set(PLAN["done"])
TASKS = [t for t in PLAN["tasks"] if t["id"] not in DONE]
BY = {t["id"]: t for t in PLAN["tasks"]}
MILESTONES = {m["id"]: m for m in PLAN["milestones"]}

# Рубежи, где ошибка — утечка трафика, обход прав или потеря данных: ревью полное,
# финальное P0-ревью в свежем контексте (MODEL-WORKFLOW §3).
P0_REVIEW = {"I06", "I07", "I08", "I09", "I10"}
P0_TASKS = {"H.05.b", "H.09", "H.12", "V.02", "V.04", "I04.T04.c", "I05.T04.a", "I05.T04.b",
            "I11.T02.a", "I15.T02.a", "I17.T03.a"}

# Задачи, которые решает или выполняет не Grok.
USER_DECIDES = {
    "H.10": "workflow готов (D9); зелёный прогон — после push пользователем",
    "I08.T01.a": "Grok готовит варианты Q09; пользователь решает",
    "I15-R.T04.a": "Grok исследует; пользователь решает Q26/Q27",
    "I15-R.T05.a": "Grok пишет ADR; пользователь утверждает go/no-go",
    "I18.T05.a": "пользователь: ready/blocked",
}
ON_HOST = {"X.02", "X.03", "X.04", "X.05", "X.06", "X.08", "V.05"}
NEEDS_HARNESS_DECISION = "H.11"  # D6 принято: rootless netns; осталось — сценарий с ядром


def executor(task):
    tid = task["id"]
    if tid in USER_DECIDES:
        return USER_DECIDES[tid]
    if tid in ON_HOST:
        return "пользователь на хосте выполняет; Grok готовит driver и разбирает evidence"
    if tid == NEEDS_HARNESS_DECISION:
        return "Grok: сценарий с тестовым сервером ядра на основе tests/netharness (D6 принято)"
    return "Grok"


def review(task):
    if task["id"] in USER_DECIDES or task["id"] in ON_HOST:
        return "координатор"
    if task["milestone"] in P0_REVIEW or task["id"] in P0_TASKS:
        return "координатор, полное + свежий контекст"
    return "координатор"


def stage(task):
    """Крупный этап по рубежу: порядок работы Grok."""
    m = task["milestone"]
    order = {
        "H": 1, "I03": 1, "V": 2, "I04": 2, "I17-D": 2, "I15-R": 2, "I05": 3, "I06": 3,
        "I07": 4, "I08": 4, "I09": 5, "I10": 5, "I11": 6, "I12": 6, "I13": 6, "I14": 6,
        "I15": 7, "I16": 7, "I17": 8, "X": 8, "I18": 9,
    }
    return order[m]


STAGES = {
    1: ("Хвосты I03 и гигиена", "Закрыть I03 (T04.x, T05), saphyr, PTY, harness, рефакторинг vpn.rs/helper.rs, аудит unsafe."),
    2: ("Срез и контракт ядер", "Новая модель доходит до пользователя (V), CoreAdapter и mihomo-адаптер (I04); параллельно ранние I15-R и I17-D."),
    3: ("Xray и привилегированный контроллер", "Второе ядро и доставка ядер (I05); typed root-протокол, peer-идентификация, транзакции (I06)."),
    4: ("Маршруты хоста и сеть приложений", "A+C: CM владеет routes/firewall, outer egress, host off/proxy/tunnel (I07); netns на приложение (I08)."),
    5: ("Fail-closed и DNS", "Нет прямого выхода при сбоях, reconcile (I09); DNS/IPv6/UDP на обоих ядрах (I10)."),
    6: ("Приложения и регион", "Универсальный запуск (I11), группы (I12), brokers/упаковки (I13), timezone/locale (I14)."),
    7: ("Приватное окружение и качество", "Адаптеры приложений по ADR I15-R (I15), автоматический выбор узла (I16)."),
    8: ("Интерфейс и установка", "TUI/апплет/диагностика (I17), отложенные runtime-проверки на хосте (X)."),
    9: ("Приёмка", "A01–A20, регрессии, scope выпуска (I18)."),
}


def main():
    ready = [t for t in TASKS if all(d in DONE for d in t["depends_on"])]
    weight = {"S": 1, "M": 2, "L": 4, "XL": 8}
    L = []
    w = L.append
    w("# Полный план оставшихся работ для Grok")
    w("")
    w("Дата: 2026-10-06. Сгенерировано `tools/grok_plan.py` из [REMAINING-DAG-PLAN.json](REMAINING-DAG-PLAN.json) — "
      "зависимости, размеры и критерии там; правки вносить в `tools/remaining_dag.py`, затем перезапускать оба скрипта. "
      "Текущий ближайший блок с подробностями: [GROK-QUEUE §Блок 3](GROK-QUEUE-2026-10-06.md). "
      "Решения: [I03-DECISIONS](I03-DECISIONS-2026-10-06.md), [QUESTIONS](QUESTIONS.md).")
    w("")
    w("## Правила для Grok на весь план")
    w("")
    w("- Исполнитель — Grok 4.7 (high), ревью — координатор. Подзадача закрыта только после ревью; для P0-рубежей "
      "(I06–I10) и отмеченных задач — полное ревью и финальное P0-ревью в свежем контексте.")
    w("- Брать только подзадачи, у которых закрыты все зависимости. Порядок внутри этапа — по столбцу «зависит от».")
    w("- Перед сдачей: `tests/check_audit.sh` зелёный, фактический прогон записан в evidence (summary + хеши, сырые логи — по D8).")
    w("- Сборка и тесты в своём `CARGO_TARGET_DIR`. Сеть — только чтение первичных источников pinned-версий и разрешённые "
      "`cargo fetch`. Никаких реальных подписок и секретов; fixtures синтетические.")
    w("- Установка, службы, изменение сети хоста, внешние публикации, коммиты и push — только по поручению пользователя. "
      "L2-проверки — в среде harness (D6), L3/L4 на хосте — по поручению (D10).")
    w("- Не переформатировать существующие файлы: никаких `cargo fmt` и `rustfmt` по старым файлам "
      "(`rustfmt src/main.rs` форматирует и все его модули). В diff — только смысловые строки; rustfmt — "
      "только по своим новым файлам. Лишнее переформатирование — отказ при ревью.")
    w("- Неподдержанное — явный Unsupported, не молчаливое удаление. Секреты не попадают в ошибки, `Debug`, статус и evidence.")
    w("- Решения пользователя Grok не выбирает: готовит материалы и останавливает зависимые подзадачи.")
    w("")
    w("## Сводка")
    w("")
    w(f"- Осталось подзадач: **{len(TASKS)}**, суммарный вес **{sum(weight[t['size']] for t in TASKS)}** "
      "(S=1, M=2, L=4, XL=8; это не сроки).")
    w(f"- Можно начинать сейчас: **{len(ready)}**.")
    w("")
    w("| Этап | Что | Подзадач | Вес |")
    w("|---|---|---|---|")
    for number, (title, _) in STAGES.items():
        items = [t for t in TASKS if stage(t) == number]
        w(f"| {number} | {title} | {len(items)} | {sum(weight[t['size']] for t in items)} |")
    w("")
    w("## Можно начинать сейчас")
    w("")
    for t in ready:
        w(f"- `{t['id']}` {t['title']} · `{t['size']}` · {executor(t)}")
    w("")
    w("## Решения и действия пользователя по ходу плана")
    w("")
    w("| Где | Что нужно |")
    w("|---|---|")
    for tid, text in USER_DECIDES.items():
        if tid not in DONE:
            w(f"| `{tid}` | {text} |")
    w(f"| `{NEEDS_HARNESS_DECISION}` | D6 принято (rootless netns); для сценария с ядром нужен бинарник mihomo/xray |")
    w("| `X.*`, `V.05` | выполнение на реальном хосте по поручению |")
    covered = set(USER_DECIDES) | {NEEDS_HARNESS_DECISION}
    for d in PLAN["decisions"]:
        if "Принято" not in d["note"] and d["blocks"] not in covered:
            w(f"| `{d['blocks']}` | {d['id']}: {d['title']} |")
    w("")
    for number, (title, goal) in STAGES.items():
        items = [t for t in TASKS if stage(t) == number]
        if not items:
            continue
        w(f"## Этап {number}. {title}")
        w("")
        w(goal)
        w("")
        w("| ID | Подзадача | Размер | Уровень | Зависит от | Исполнитель | Ревью |")
        w("|---|---|---|---|---|---|---|")
        for t in items:
            deps = ", ".join(f"`{d}`" for d in t["depends_on"] if d not in DONE) or "—"
            mark = " ★" if t.get("critical") else ""
            w(f"| `{t['id']}`{mark} | {t['title']} | {t['size']} | {t['level']} | {deps} | {executor(t)} | {review(t)} |")
        w("")
        for t in items:
            w(f"- **`{t['id']}`** — {t['what']} **Готово, когда:** {t['done_when']}")
        w("")
    w("★ — критический путь. Уровни: D — решение/ADR, L0 — документ, L1 — unit/fixture, L2 — netns harness, "
      "L3 — реальное приложение, L4 — сквозная проверка/установка.")
    w("")
    OUT.write_text("\n".join(L))
    print(f"{len(TASKS)} подзадач, готовы сейчас {len(ready)} → {OUT.name}")


if __name__ == "__main__":
    main()
