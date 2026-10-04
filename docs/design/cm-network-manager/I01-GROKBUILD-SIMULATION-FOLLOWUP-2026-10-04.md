# Grokbuild: повторная симуляция после ревью Sol

Результат: [лента повторной симуляции](I01-GROKBUILD-SIMULATION-FOLLOWUP-TAPE-2026-10-04.md). Статус ленты — SIMULATED.

Только SIMULATED, без выполнения бинарника, тестов, служб, package manager или сети. Реальные проверки остаются для установки собранного бинарника. Старую ленту [2026-10-04](I01-GROKBUILD-SIMULATION-2026-10-04.md) сохранить.

Входы: актуальные `src/main.rs`, `src/migration/manual.rs`, `src/migration/transaction.rs`, `tests/audit_i01_migration.rs`, [ревью Sol](I01-GROKBUILD-SIMULATION-SOL-REVIEW-2026-10-04.md), новая сводка `i01-evidence/simulation-fixes-2026-10-04/summary.md`. Сначала зафиксировать SHA исходников. Прежний собранный бинарник не пересобран и не содержит этих исправлений; нельзя приписать ему новые исходные ветки.

Повторить изменённые сценарии последовательно:

1. VPN unit inactive, но работает executable `/var/lib/upd/vpn/bin/mihomo`. Лента plan/apply: ранний process audit отказывает до backup/journal. Проверить также иной путь к тому же inode (hardlink).
2. CLI, GUI или legacy core удалён/заменён, а `/proc/PID/exe` содержит точный managed path с ` (deleted)`. Показать отказ. Посторонний executable с basename `mihomo` и иным inode не считать legacy.
3. Missing exe: исчезнувший PID, подтверждённый kernel thread, zombie, живой обычный PID, unreadable/malformed stat. Показать ветки `safely_missing_exe`/`proc_stat_is_unexecutable`, различая допустимое отсутствие и неустановленную quiescence.
4. Чистый supported layout, затем отдельно nested symlink, special file, unsafe mode/owner внутри данных. `manual::plan` теперь использует общий `validate_plan`: успешная сводка только после read-only проверки всего filesystem plan. Отказы не создают journal/blob/lock и сохраняют sentinels. Повторная проверка перед execute остаётся.
5. Old VPN/helper/operation service становится active после предыдущего audit, но до inspect внутри `Systemd::set`. Pure guard отказывает до stop/disable/enable/start. Покажи предыдущие шаги транзакции, результат компенсации и возможный pending journal; не обещай автоматический rollback поверх чужой работающей операции.
6. Active timer/path admission и новая CM service при rollback: guard не запрещает предусмотренные stop/state transitions. Сохрани различие service и timer/path.
7. Активация после inspect внутри `Systemd::set`, но до mutator: не считать гонку закрытой. Обозначить limitation и конкретную будущую runtime проверку.

Для каждой строки ленты: исходное состояние → событие/команда → предсказанный ответ → filesystem/journal/service diff → invariant → точная функция/ветка кода → допущения. Не скрывать промежуточные отказы, rollback failures и retry. Число шагов обосновать явным inventory выбранной модели, не считать старые 48/22 универсальной константой.

Вернуть ленту, подтверждение/контрпримеры исправлений и список real checks при установке. Compile PASS означает только компиляцию; новые regression tests не исполнялись. Лента не является runtime acceptance и не даёт права на host install.
