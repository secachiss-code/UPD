# Повторная симуляция изменённых сценариев I01

Статус: **SIMULATED**. Это предсказание по исходному тексту, прочитанному 2026-10-04. Бинарник, тесты, systemd, package manager и сеть не запускались. Лента не является runtime acceptance, не принимает I01, не закрывает I01.T04 и не открывает I02. Установка на хост этой лентой не разрешена.

Старая лента [I01-GROKBUILD-SIMULATION-2026-10-04.md](I01-GROKBUILD-SIMULATION-2026-10-04.md) не изменялась. Задание: [I01-GROKBUILD-SIMULATION-FOLLOWUP-2026-10-04.md](I01-GROKBUILD-SIMULATION-FOLLOWUP-2026-10-04.md). Основание правок: [ревью Sol](I01-GROKBUILD-SIMULATION-SOL-REVIEW-2026-10-04.md).

`cargo check --offline --locked --tests` в [сводке](i01-evidence/simulation-fixes-2026-10-04/summary.md) — **COMPILE_PASS**: компиляция тестовых целей, без их исполнения. Десять новых регрессий ниже остаются **NOT_RUN**. Эта сессия их не запускала и проверку компиляции не повторяла.

## Фиксация исходников

SHA-256 прочитанных файлов совпал со сводкой Sol:

| Файл | SHA-256 |
| --- | --- |
| `src/main.rs` | `9a63c421e4963d1bfe0940eda76bf743bdc2e07db0fdb6fb64ad81c7ae9f5183` |
| `src/migration/manual.rs` | `fb90270a519b0c261ebd988358b0f27c9b4aadc27cc0f75fe6f5bf9815c6fd56` |
| `src/migration/transaction.rs` | `ae4a94bfb28b858f82499618b9d6b1322b2a867652c652a2b50894843e0570f7` |
| `tests/audit_i01_migration.rs` | `5e9a1e9e39b0f504f2553b4f33cd758779ba204a86ee9286b207b96e8f386a6b` |

`target/x86_64-unknown-linux-musl/debug/cm` по-прежнему `b46c5d9381ddadc93b8a1f2b428ca71f9772ceef814795df2c44e4d179498b3b`. Метка времени бинарника раньше меток этих четырёх файлов. Состояние бинарника: **OLD_NOT_REBUILT**. Ветки этой ленты ему не приписываются.

Текст отказа печатается через `t!("ошибка: {0}", e)` в `cmd_migration` → `err_code`. Тело `{0}` — английская строка `Err` из миграции. Префикс берётся из активной локали (`ошибка: {0}` / `error: {0}` / …). Ниже указан код 1 и тело `Err`. Ответы systemd и pacman в этой ленте — условия модели, не снятый вывод команд.

## Модель P

Одна и та же картина для сценариев, где нужен успешный `manual::plan`. Числа 48 и 22 сюда не переносятся.

Наследство, всё uid 0, каталоги без групповой и чужой записи:

- `/usr/local/bin/upd` — обычный файл `0755`, байты с SHA-256 `legacy::CLI_SHA256`.
- `/etc/upd.conf` — обычный файл `0600`, синтетическое содержимое.
- `/var/lib/upd/vpn/config.yaml` — обычный файл `0600`, синтетический sentinel `password: synthetic-raw-vpn\n`. Каталоги `0700`.
- `/var/lib/upd/vpn/bin/mihomo` — обычный файл `0755`, синтетические байты. Процесс его не исполняет, пока сценарий прямо не вводит PID.
- `/etc/systemd/system/upd-vpn.service` — обычный файл `0644`, байты совпадают с `legacy::system_units` для `upd-vpn.service` и префиксом `# Managed by upd\n`.
- `/etc/systemd/system/multi-user.target.wants/upd-vpn.service` — symlink на `/etc/systemd/system/upd-vpn.service`.
- Других путей из `LEGACY_PATHS`, других unit, drop-in, `.requires`, GUI, cron, `/run/upd`, `/run/upd/helper.sock` и любых путей CM нет.
- `/` — каталог uid 0 без групповой и чужой записи. `/run/systemd/system` — каталог. `/etc/systemd/user` и `/etc/pacman.d/hooks` — каталоги uid 0, `0755`, без файлов CM.

Служба `upd-vpn.service`: `is-enabled` = `enabled`; `LoadState=loaded`; `ActiveState=inactive`; `FragmentPath=/etc/systemd/system/upd-vpn.service`; `DropInPaths` пуст; `NeedDaemonReload=no`. Пользовательских сессий нет. Pacman есть в `PATH` и на каждый инвентарный путь отвечает кодом 1 с текстом `No package owns`. Apt, NetworkManager, polkit и `/etc/cron.d/cm` отсутствуют. Рядом с текущим исполняемым файлом CM нет `cm-cosmic`. Зеркала пакетом не ведутся: `watch` = `None`. Журнала `/var/lib/cm-migration` нет. Блокировки свободны.

При таком `watch` генератор CM кладёт в replacements 11 файлов: `/usr/local/bin/cm`, семь system unit (`cm-auto.service`, `cm-auto.timer`, `cm-net.service`, `cm-net.timer`, `cm-vpn.service`, `cm-helper.socket`, `cm-helper.service`), два user unit (`cm-notify.service`, `cm-notify.timer`) и `/etc/pacman.d/hooks/zz-cm.hook`. Unit-файлы CM в план попадают даже без парного наследства: пара служб создаётся только для найденного unit.

`manual::plan` собирает `changes` в таком порядке:

1. `/etc/cm.conf` ← дерево `/etc/upd.conf`
2. `/var/lib/cm` ← дерево `/var/lib/upd`
3. `/etc/pacman.d/hooks/zz-cm.hook`
4. `/etc/systemd/system/cm-auto.service`
5. `/etc/systemd/system/cm-auto.timer`
6. `/etc/systemd/system/cm-helper.service`
7. `/etc/systemd/system/cm-helper.socket`
8. `/etc/systemd/system/cm-net.service`
9. `/etc/systemd/system/cm-net.timer`
10. `/etc/systemd/system/cm-vpn.service`
11. `/etc/systemd/user/cm-notify.service`
12. `/etc/systemd/user/cm-notify.timer`
13. `/usr/local/bin/cm`
14. `/etc/systemd/system/multi-user.target.wants/cm-vpn.service` ← symlink на unit
15. снять wants-ссылку `upd-vpn.service`
16. снять `/etc/systemd/system/upd-vpn.service`
17. снять `/etc/upd.conf`
18. снять `/usr/local/bin/upd`
19. снять `/var/lib/upd`

Службы, две:

1. `upd-vpn.service` → выключена и неактивна, до файлов.
2. `cm-vpn.service` → включена и неактивна, после файлов.

Итог модели P, если `plan` доходит до `Ok`: **19 файловых шагов и 2 шага служб**. Четыре lock path (`/var/lib/upd/.lock`, `/var/lib/upd/vpn/.vpn-config.lock`, `/etc/.upd.conf.lock`, `/var/lib/upd/.vpn-subs.lock`) в это число не входят. Сводка печатается только после `validate_plan` и после раннего `no_legacy_process`.

Отдельный inventory регрессии `manual_plan_prevalidates_nested_data_without_writing_journal`: прямой вызов `manual::plan` с одним replacement `/usr/local/bin/cm` и без unit. Шаги: `/etc/cm.conf`, `/var/lib/cm`, `/usr/local/bin/cm`, затем снятие `/etc/upd.conf`, `/usr/local/bin/upd`, `/var/lib/upd`. Это **6 файловых шагов и 0 шагов служб**. Командная сводка модели P этим числом не подменяется.

Ниже каждая строка: состояние → событие → предсказанный ответ → diff → инвариант → ветка → допущения.

## 1. Неактивный VPN unit и живой mihomo

Решающий PID один. Остальные записи `/proc` в этой строке — чужие исполняемые файлы с другим inode и без managed path.

### 1.1 Прямой путь ядра

Состояние: модель P. `upd-vpn.service` неактивен. Процесс держит исполняемый файл `/var/lib/upd/vpn/bin/mihomo`: `readlink` `/proc/PID/exe` равен этому пути, inode тот же.

Событие: `cm migration plan`.

Ответ: код 1, тело `legacy executable still running; migration refused`. Строка сводки с числом шагов не печатается.

Diff: `/var/lib/cm-migration` не создаётся. Blob, `journal.json` и lock нет. Sentinel и unit на месте. `systemctl stop/disable/enable/start` не вызывается. Служба остаётся неактивной.

Инвариант: отказ раньше backup и журнала. `plan` уже вернул план 19/2 в память; `execute` не вызывается, план отбрасывается.

Код: `cmd_migration` вызывает `manual::plan`, затем `no_legacy_process("/")`. Внутри `plan` проверка `state.active` для `.service` и `upd-helper.socket` этот unit пропускает. Отказ даёт `audit_processes`: `is_managed_proc_target` видит точное равенство с `/var/lib/upd/vpn/bin/mihomo`.

Допущения: в дереве нет socket, fifo и symlink, поэтому `validate_plan` проходит. `/run/upd/helper.sock` отсутствует, иначе `plan` отказал бы раньше текстом про helper socket. `metadata` managed-файла читается. Pacman и `loginctl` отвечают как в модели P.

### 1.2 Тот же inode по другому пути

Состояние: модель P. Файл `/elsewhere/mihomo` — hardlink на inode `/var/lib/upd/vpn/bin/mihomo`. Процесс запущен по пути `/elsewhere/mihomo`. Unit VPN неактивен.

Событие: `cm migration plan`, затем отдельно то же для `cm migration apply`.

Ответ обеих команд: код 1, то же тело `legacy executable still running; migration refused`. Сводка не печатается. `apply` не входит в `execute`.

Diff: как в 1.1. Для `apply` дополнительно не создаются backup и `journal.json`.

Инвариант: имя `mihomo` само по себе не используется. Совпадение device/inode с живым managed-файлом достаточно для отказа.

Код: `is_managed_proc_target` для `/elsewhere/mihomo` даёт ложь. Следующая ветка `fs::metadata("/proc/PID/exe")` находит пару `(dev, ino)` в списке, собранном `fs::metadata` по трём managed-путям, включая `/var/lib/upd/vpn/bin/mihomo`.

Допущения: managed-файл всё ещё существует, поэтому его inode попал в список. `metadata` на `/proc/PID/exe` возвращает inode отображённого файла. Это поведение ядра здесь не снималось.

### 1.3 Apply при том же прямом пути

Состояние: как 1.1, журнала нет.

Событие: `cm migration apply`.

Ответ: код 1, то же тело про running executable.

Diff: каталога миграции нет.

Инвариант: ранний audit стоит в `cmd_migration` до `execute`. Повторная `validate_plan` внутри `execute` до этого отказа не доходит.

Код: тот же вызов `no_legacy_process` после `plan` и до построения `ProductionObserver`.

Допущения: `already_committed` ложен, `/run/systemd/system` есть, `current_exe` читается.

## 2. Точный путь с суффиксом ` (deleted)`

Суффикс сравнивается целиком: `format!("{} (deleted)", managed.display())`. Иное окончание, пробел или перевод этого суффикса ветку не выбирают.

### 2.1 GUI снят, процесс показывает точный deleted path

Состояние: модель P без файла `/usr/local/bin/upd-cosmic`. Процесс: `readlink` = `/usr/local/bin/upd-cosmic (deleted)`.

Событие: `cm migration plan`.

Ответ: код 1, `legacy executable still running; migration refused`.

Diff: журнала нет, sentinel на месте.

Инвариант: отсутствующий GUI не мешает `plan` дойти до audit. Identity для NotFound пропускается, отказ даёт само имя в `/proc`.

Код: `audit_processes` на NotFound не кладёт inode. `is_managed_proc_target` сравнивает цель с `/usr/local/bin/upd-cosmic (deleted)`.

Допущения: остальной layout модели P проходит `plan`.

### 2.2 Ядро удалено или заменено, процесс показывает точный deleted path

Состояние: модель P. Файла `/var/lib/upd/vpn/bin/mihomo` нет, либо на его пути лежит другой inode. Процесс: `/var/lib/upd/vpn/bin/mihomo (deleted)`. Дерево без symlink и special file. Unit VPN неактивен.

Событие: `cm migration plan`.

Ответ: код 1, `legacy executable still running; migration refused`.

Diff: журнала нет. Новый inode на старом пути, если он есть, не копируется: `execute` не начат.

Инвариант: данные ядра не сверяются с pinned digest. Отказ именно у process audit, уже после `validate_plan`.

Код: `is_managed_proc_target` для третьего managed-пути. Текущий inode пути в список либо не входит, либо это уже другой inode; до сравнения inode выполнение не доходит, потому что имя совпало раньше.

Допущения: каталог `/var/lib/upd` остаётся обычным деревом, чтобы `plan` вернул `Ok`.

### 2.3 CLI заново создан с теми же pinned-байтами

Состояние: на `/usr/local/bin/upd` снова обычный файл `0755` uid 0 с тем же SHA-256. Старый процесс: `/usr/local/bin/upd (deleted)`.

Событие: `cm migration plan`.

Ответ: код 1, `legacy executable still running; migration refused`.

Diff: журнала нет.

Инвариант: совпадение digest текущего файла не отменяет deleted path старого процесса.

Код: `digest` в `plan` проходит. Отказ снова в `is_managed_proc_target` для `/usr/local/bin/upd (deleted)`.

Допущения: текущий файл именно pinned 0.2.7. Иначе срабатывает 2.4 или 2.5, и эта ветка не достигается.

### 2.4 CLI отсутствует на диске

Состояние: `/usr/local/bin/upd` нет. Процесс может показывать `/usr/local/bin/upd (deleted)`.

Событие: `cm migration plan`.

Ответ: код 1. Тело — текст I/O-ошибки `symlink_metadata` из `check_file`, а не фраза про running executable.

Diff: журнала нет.

Инвариант: отказ по-прежнему раньше журнала. Ветка deleted path для CLI в этом состоянии не исполняется.

Код: `plan` читает `/usr/local/bin/upd` через `check_file` до `validate_plan` и до `no_legacy_process`.

Допущения: родитель пути доступен `Executor::path`.

### 2.5 CLI заменён другими байтами

Состояние: `/usr/local/bin/upd` существует, digest не равен `CLI_SHA256`. Процесс может показывать deleted path.

Событие: `cm migration plan`.

Ответ: код 1, `unsupported or foreign UPD binary; expected pinned manual UPD 0.2.7`.

Diff: журнала нет.

Инвариант: process audit снова не достигается. Для GUI с чужим digest та же развилка даёт `foreign/unsupported UPD GUI binary`, если файл GUI присутствует и не проходит `digest`.

Код: сравнение `digest(&cli)` с `legacy::CLI_SHA256` в `plan`.

Допущения: файл проходит `check_file` по типу, владельцу и mode.

### 2.6 Чужой исполняемый файл с именем mihomo

Состояние: модель P, свой mihomo не запущен. Другой процесс: путь `/usr/bin/mihomo` или `/tmp/mihomo`, другой inode.

Событие: `cm migration plan`.

Ответ: код 0. Сводка: `CM: supported manual UPD 0.2.7; 19 filesystem steps, 2 service steps. No files or services changed.`

Diff: пустой. Журнала нет, службы не меняются.

Инвариант: basename `mihomo` без managed path и без того же inode не считается legacy.

Код: оба сравнения в `audit_processes` этого PID не выбирают. Функция возвращает `Ok`, затем `cmd_migration` печатает длины `plan.changes` и `plan.services`.

Допущения: других legacy PID нет. Модель P целиком проходит `plan` и `validate_plan`.

## 3. Пропавший exe

Каждая строка — отдельный прогон, где указанный PID решает результат, а прочие PID чужие и читаемые. `no_legacy_process` смотрит настоящие `/` и `/proc`. Синтетический корень принимает только приватный `audit_processes`, его production-команда не вызывает.

Общая развилка: `readlink` `/proc/PID/exe` вернул `ENOENT` или `ESRCH`. Иная ошибка `readlink`, включая `EACCES`, сразу даёт `safely_missing_exe` = false и текст `cannot establish legacy process quiescence: …` без разбора `stat`.

### 3.1 PID уже исчез

Состояние: `read_dir` успел увидеть цифровой каталог, `readlink` даёт `ENOENT` или `ESRCH`, `metadata` каталога PID тоже `ENOENT` или `ESRCH`.

Событие: `cm migration plan` на модели P.

Ответ: этот PID пропускается. Если он единственное отклонение, код 0 и сводка 19/2.

Diff: пустой.

Инвариант: исчезновение PID — допустимое отсутствие. Quiescence из-за него не считается неустановленной.

Код: `safely_missing_exe` → `Ok(true)` по второй проверке PID. Цикл `audit_processes` делает `continue`.

Допущения: код ошибки именно `ENOENT` или `ESRCH`.

### 3.2 PID исчез, пока читался stat

Состояние: каталог PID существовал на проверке в `safely_missing_exe`, файл `stat` к чтению уже `ENOENT` или `ESRCH`, повторный `metadata` PID тоже `ENOENT` или `ESRCH`.

Событие: тот же `plan`.

Ответ: PID пропускается. При чистом остатке — код 0 и сводка 19/2.

Diff: пустой.

Инвариант: гонка исчезновения во время `stat` тоже допустимое отсутствие.

Код: `proc_stat_is_unexecutable` возвращает `Ok(true)` в ветке парного `ENOENT`/`ESRCH`.

Допущения: оба кода из этой пары.

### 3.3 Kernel thread

Состояние: каталог PID есть, `exe` отсутствует с `ENOENT` или `ESRCH`. В `stat` после последней `)` первое поле не `Z`, поле flags (индекс 6) содержит бит `PF_KTHREAD` (`0x0020_0000`).

Событие: `cm migration plan` на модели P.

Ответ: PID пропускается. При чистом остатке — код 0 и сводка 19/2.

Diff: пустой.

Инвариант: подтверждённый kernel thread — допустимое отсутствие, даже если состояние процесса `R` или `S`.

Код: `proc_stat_is_unexecutable` → `Ok(flags & PF_KTHREAD != 0)`.

Допущения: разбор полей совпадает с форматом `stat`, который Sol сверял с `fs/proc/array.c`: после последней `)` индекс 0 — state, индекс 6 — flags. Сам `/proc` хоста не читался. Другие биты flags не мешают, если бит `PF_KTHREAD` установлен.

### 3.4 Zombie

Состояние: каталог PID есть, `exe` отсутствует с `ENOENT` или `ESRCH`. После последней `)` первое поле `stat` — байты `Z`.

Событие: `cm migration plan` на модели P.

Ответ: PID пропускается. При чистом остатке — код 0 и сводка 19/2.

Diff: пустой.

Инвариант: точное состояние `Z` — допустимое отсутствие. Разбор flags для него не нужен.

Код: `proc_stat_is_unexecutable` возвращает `Ok(true)` на `*state == b"Z"` до чтения flags.

Допущения: `readlink` действительно неуспешен. Если `exe` у zombie читается и указывает на managed path, раньше срабатывает отказ сценария 2. Состояние `X` этой веткой не выбирается: при отсутствии бита `PF_KTHREAD` оно идёт строкой 3.5.

### 3.5 Живой обычный PID

Состояние: каталог PID есть, `exe` отсутствует с `ENOENT` или `ESRCH`. `stat` читается, состояние `S` или `R`, flags без бита `PF_KTHREAD`, строка правильно разобрана.

Событие: `cm migration plan`.

Ответ: код 1, `cannot establish legacy process quiescence: …` с исходной ошибкой `readlink`. Сводки нет.

Diff: журнала нет.

Инвариант: это неустановленная quiescence, не допустимое отсутствие. `safely_missing_exe` возвращает `Ok(false)`, и общий цикл превращает это в отказ.

Код: `proc_stat_is_unexecutable` → `Ok(false)`. Условие `Err(error) if safely_missing_exe(...)?` ложно, следующая ветка `match` возвращает текст про quiescence.

Допущения: `stat` доступен и хорошо сформирован. Иначе отказ будет 3.6 или 3.7.

### 3.6 Нечитаемый stat

Состояние: каталог PID есть, `exe` даёт `ENOENT` или `ESRCH`, чтение `stat` падает ошибкой, отличной от пары «PID тоже исчез».

Событие: `cm migration plan`.

Ответ: код 1, `cannot establish legacy process state from {pid}/stat: …`. Сводки нет.

Diff: журнала нет.

Инвариант: quiescence не установлена. Это другой текст, чем у живого PID в 3.5.

Код: `proc_stat_is_unexecutable`, ветка `Err(error)` после неудачного исключения исчезнувшего PID. `?` в `safely_missing_exe` пробрасывает эту строку наружу.

Допущения: каталог PID остаётся читаемым для `metadata`. Если недоступен уже он, текст другой: `cannot establish legacy process quiescence for {pid}: …`.

### 3.7 Испорченный stat

Состояние: `exe` отсутствует допустимым кодом, `stat` читается, но в нём нет `)`, нет поля состояния, либо поле flags не разбирается как `u64`. Состояние при этом не `Z`.

Событие: `cm migration plan`.

Ответ: код 1, `malformed process stat: {pid}/stat`. Сводки нет.

Diff: журнала нет.

Инвариант: порча `stat` — неустановленная quiescence.

Код: `rposition` по `)`, затем `fields.first()`, затем `parse::<u64>()` в `proc_stat_is_unexecutable`.

Допущения: файл `stat` открылся. Последняя `)` считается концом `comm`, включая `)` внутри имени, как в подготовленной регрессии `401 (kernel ) thread)`.

Подготовленные и не запущенные тесты этой группы: `live_legacy_core_is_detected_by_inode_but_same_name_is_ignored`, `exact_deleted_legacy_executable_paths_are_detected`, `unreadable_or_missing_exe_for_a_present_pid_fails_closed`, `missing_stat_for_present_pid_fails_closed`, `missing_exe_is_ignored_only_for_kernel_threads_or_zombies`.

## 4. Общий read-only validator

### 4.1 Чистая модель P

Состояние: модель P, legacy-процессов нет, записей `/proc` из раздела 3, которые дают отказ, нет.

Событие: `cm migration plan`.

Ответ: код 0. Текст: `CM: supported manual UPD 0.2.7; 19 filesystem steps, 2 service steps. No files or services changed.`

Diff: пустой. Каталога `/var/lib/cm-migration` нет. Sentinel `password: synthetic-raw-vpn\n` не меняется.

Инвариант: сводка печатается только после полного прохода `validate_plan` по всем 19 изменениям и после `no_legacy_process`. Оба прохода read-only: `snapshot(..., false, ...)`.

Код: конец `manual::plan` вызывает `Executor::validate_plan`. Печать стоит в `cmd_migration` после `no_legacy_process`.

Допущения: список раздела «Модель P». Ответы pacman, loginctl и systemd — условия модели.

### 4.2 Вложенный symlink

Состояние: модель P плюс root-owned symlink `/var/lib/upd/vpn/nested-symlink`. Sentinel на месте.

Событие: `cm migration plan`.

Ответ: код 1, `symlink in data source`. Сводки нет.

Diff: пустой, включая sentinel. Журнала, blob и lock нет.

Инвариант: отказ внутри `plan`, до возврата плана и до process audit.

Код: `walk` записывает `Kind::Link`, потому что для symlink проверка mode `0o022` не применяется. `validate_plan` для `Content::Tree` отвергает любой `Kind::Link` в снимке источника.

Допущения: владелец symlink равен uid корня. Чужой владелец выберет строку 4.4 раньше, на том же узле.

### 4.3 Special file

Состояние: модель P плюс socket или fifo `/var/lib/upd/vpn/runtime.sock`, uid 0, mode `0600`.

Событие: `cm migration plan`.

Ответ: код 1, `special file in migration data; quiesce the legacy runtime first`.

Diff: пустой. Журнала нет. Sentinel на месте.

Инвариант: special file отвергается на чтении снимка, без открытия его содержимого.

Код: `walk` после проверки владельца: не каталог, не обычный файл и не symlink.

Допущения: mode без битов `0o022`. Mode `0664` на том же inode дал бы строку 4.4 и до этой ветки не дошёл.

### 4.4 Чужой владелец или групповая запись

Состояние A: обычный файл `/var/lib/upd/vpn/group-writable`, uid 0, mode `0664`, байты `synthetic writable-entry sentinel`. Состояние B, отдельно: обычный файл mode `0600` с uid, отличным от владельца `/`.

Событие: `cm migration plan` для каждого состояния отдельно.

Ответ обоих: код 1, `foreign owner or writable migration entry`.

Diff: пустой. Оба sentinel остаются. Журнала нет.

Инвариант: групповая и чужая запись (`mode & 0o022`) и чужой uid — одна ветка. Обычная запись владельца (`0644`, `0600`, `0755`) её не выбирает.

Код: условие в начале `walk`.

Допущения: узел лежит внутри дерева `/var/lib/upd`, которое попадает в `Content::Tree`.

### 4.5 Повторная проверка в apply

Состояние: на входе в `execute` дерево уже содержит дефект 4.2, 4.3 или 4.4. Либо дефект был до `plan`, либо он появился после возврата `plan` и до `validate_plan` внутри `execute`.

Событие: `cm migration apply`.

Ответ: код 1 с текстом соответствующей строки 4.2–4.4.

Diff: `create_dir_all` журнала не вызывается. Blob и lock нет. Sentinel на месте.

Инвариант: `execute` повторяет тот же `validate_plan` до создания каталога журнала и до захвата lock. Успешный более ранний `plan` не становится разрешением на запись.

Код: первая проверка в `Executor::execute` после отказа на уже существующем журнале. В модели P журнала нет, поэтому выполняется `validate_plan`. Если дефект уже был на `plan`, `execute` даже не вызывается: отказ тот же и тоже без каталога.

Допущения: внешняя запись между двумя вызовами возможна; сама лента её не выполняет.

### 4.6 Чистый apply доходит до повторной проверки

Состояние: модель P на обоих вызовах.

Событие: `cm migration apply` до первого создания каталога журнала.

Ответ этой границы: `validate_plan` возвращает `Ok`. Следующее действие кода — создать `/var/lib/cm-migration` с mode `0700`, если каталога нет.

Diff на самой проверке: пустой. Дальше транзакция уже пишет. Продолжение до commit в эту повторную ленту не входит.

Инвариант: повторный проход стоит раньше первой записи журнала.

Код: `execute` → `validate_plan` → `journal_dir` / `create_dir_all`.

Допущения: ранний `no_legacy_process` тоже прошёл.

Прямой вызов регрессии на фикстуре 6/0 предсказывает тот же контракт: чистый `manual::plan` возвращает план и не создаёт `/var/lib/cm-migration`; symlink и файл `0664` дают `Err`, дерево и credential sentinel остаются, каталога журнала нет. Успешный прямой `validate_plan` тоже ничего не пишет. Тесты `manual_plan_prevalidates_nested_data_without_writing_journal` и `shared_plan_validator_success_is_read_only` не запускались.

## 5. Старая служба стала активной перед inspect внутри set

Модель P. Единственная служба «до файлов» — `upd-vpn.service`, поэтому её `Systemd::set` — первый mutator транзакции. `upd-helper.socket` и `upd-auto.service` попадают в ту же функцию `guard_active_legacy_admission`, когда именно их передают в `set` и `inspect` показывает `active`. Раскладка, где перед ними уже есть другие службы «до файлов», успевает выполнить `set` тех более ранних unit; это уже другой inventory, не модель P.

### 5.1 Окно после audit на границе locked

Состояние: модель P дошла до `execute`. Ранний process audit прошёл. Снимки и blobs записаны, `journal.json` в фазе `Prepared`. Legacy lock взяты, поколение файлов совпало. На границе `locked` повторный `inspect` ещё видит `upd-vpn.service` неактивным, process audit повторяется и проходит. Сразу после этого журнал сохраняется как `Applying`, `mutations_started=true`, шаг `service-before-0` сохранён. Затем VPN становится `ActiveState=active`. Файлы деревьев внешний запуск не меняет.

Событие: `services.set` для `upd-vpn.service`, цель — выключена и неактивна.

Ответ `set`: `active legacy service appeared; migration refused`. `stop`, `disable`, `enable` и `start` для этого вызова не отправляются.

Дальше `execute` вызывает `rollback`, потому что ошибка уже после `mutations_started`.

Предсказанный конец команды, если blobs целы, посторонней правки нет и `daemon-reload` успешен: код 1, тело `migration failed (active legacy service appeared; migration refused); recovery incomplete (active legacy service appeared; migration refused); backups retained, run cm migration recover`.

Diff:

- каталог `/var/lib/cm-migration` есть, mode `0700`;
- blobs подготовки остаются;
- `journal.json` остаётся в фазе `RollingBack`, шаг `rollback`, `mutations_started=true`; фазы `RolledBack` нет;
- созданные этой транзакцией пустые lock-файлы rollback снимает до `daemon-reload`; `transaction.lock` и журнал остаются;
- деревья `/etc/upd.conf`, `/var/lib/upd` и unit-файл не заменяются транзакцией;
- новых `/etc/cm.conf`, `/var/lib/cm` и unit CM нет;
- `upd-vpn.service` остаётся active: guard не вызывает `stop`;
- предсказанный вызов `/usr/bin/systemctl daemon-reload` во время незавершённой компенсации есть. Это чтение ленты, не снятый вывод systemctl.

Инвариант: автоматического отката поверх чужой работающей операции нет. Ветка «не восстанавливать состояние служб», которая действует при `mutations_started=false`, здесь уже не выбирается.

Код: `Systemd::set` делает `inspect`, затем `guard_active_legacy_admission`. Предикат истинен для unit с префиксом `upd-` и суффиксом `.service`. `Self::run` стоит ниже guard. Откат при `mutations_started` доходит до финального цикла `set(service, before)` и там снова упирается в тот же guard для активного `upd-vpn.service`. `cm-vpn.service` в первом цикле отката получает цель default; unit ещё не установлен, `inspect` даёт default, mutator для него не нужен.

Допущения: активация попадает строго после `inspect` на границе `locked` и до `inspect` внутри `set`. Содержимое инвентарных путей не меняется, иначе срабатывает 5.4. `daemon-reload` возвращает успех; иначе 5.2.

### 5.2 Та же гонка, daemon-reload в откате неуспешен

Состояние: как 5.1 до начала компенсации.

Событие: `rollback` доходит до `services.reload()`, и команда `daemon-reload` возвращает неуспех.

Ответ: код 1, обёртка `migration failed (active legacy service appeared; migration refused); recovery incomplete (daemon-reload failed); backups retained, run cm migration recover`.

Diff: журнал уже сохранён как `RollingBack` до reload. Финальный `set` старой службы не выполняется. VPN остаётся active. Blobs остаются.

Инвариант: и при успехе, и при неуспехе reload компенсация не завершается и службу не останавливает.

Код: `Systemd::reload`, затем в `execute` ветка `recovery incomplete`.

Допущения: проверки blobs и посторонней правки прошли, поэтому выполнение дошло до reload.

### 5.3 Повтор, пока служба ещё active

Состояние: журнал `RollingBack` после 5.1 или 5.2. `upd-vpn.service` всё ещё active. Деревья совпадают со снимком.

Событие: `cm migration recover`.

Ответ: код 1, тело уже без внешней обёртки: `active legacy service appeared; migration refused`.

Diff: фаза остаётся `RollingBack`. Служба остаётся active. Новый commit не начинается. Обычная root-команда, которая вызывает `startup_allowed`, получает `unfinished migration; run cm migration recover`.

Инвариант: повтор не останавливает чужую операцию и не помечает журнал `RolledBack`.

Код: `recover` для незавершённой фазы снова вызывает `rollback`. Финальный `set(upd-vpn, before)` снова входит в guard.

Допущения: `inspect` по-прежнему видит active. Lock журнала свободен.

### 5.4 Активная служба успела изменить инвентарное дерево

Состояние: окно то же, что в 5.1, но к моменту `rollback` снимок `/var/lib/upd` или другого пути плана уже не совпадает с `before` и не объясняется staging-файлом транзакции.

Событие: компенсация внутри упавшего `apply`.

Ответ: код 1, `migration failed (active legacy service appeared; migration refused); recovery incomplete (concurrent foreign change; recovery refused without deleting it); backups retained, run cm migration recover`.

Diff: фаза остаётся `Applying`, потому что отказ посторонней правки стоит до присвоения `RollingBack`. Инвентарная чужая запись не удаляется. `stop` VPN не вызывается. `daemon-reload` этой веткой не достигается.

Инвариант: компенсация останавливается до первой компенсирующей записи по дереву. Работающая операция не гасится.

Код: цикл сравнения снимков в `rollback` до `j.phase = RollingBack`.

Допущения: отличие снимка именно постороннее, не собственный staging prefix `.cm-migration.tmp` от blob этой транзакции.

### 5.5 Более раннее окно, до границы locked

Состояние: модель P. Служба становится active после раннего process audit и до `inspect` внутри границы `locked`. `mutations_started` ещё false. Blobs и `Prepared` уже записаны.

Событие: `cm migration apply`.

Ответ: код 1, `migration failed (legacy operation/helper became active while preparing migration); original installation restored`.

Diff: журнал доводится до `RolledBack`, шаг `compensated-before-mutations`. Созданные lock снимаются. Состояния служб из снимка поверх уже активной службы не записываются. `stop` не вызывается. Деревья не меняются. Blobs остаются. Следующий `apply` просит архивировать журнал: `previous migration rolled back; archive its private journal before a new attempt`.

Инвариант: это другое окно, чем 5.1. Здесь компенсация как раз отказывается накладывать сохранённое состояние службы на операцию, начавшуюся во время подготовки.

Код: `ProductionObserver::boundary` при имени `locked`. Затем `rollback` при `!mutations_started`.

Допущения: active виден именно systemctl-`inspect` на этой границе.

### 5.6 Helper socket и operation service

Состояние: в `set` передаётся `upd-helper.socket` или `upd-auto.service`, и `inspect` этого вызова уже показывает active. Для socket цель транзакции — выключен и неактивен. Для operation service то же.

Событие: этот вызов `set`.

Ответ: `active legacy service appeared; migration refused` до `stop`/`disable`/`enable`/`start` данного вызова.

Diff вызова: пустой со стороны mutator. Если это первый `set` после уже выставленного `mutations_started`, форма журнала совпадает с 5.1. Если более ранний `set` другой неактивной службы уже выполнил `disable` или `stop`, этот эффект сохраняется; данная лента такой раскладки не разворачивает, потому что в модели P более ранней службы нет.

Инвариант: оба имени входят в предикат guard. Неактивные `upd-vpn.service`, `upd-helper.socket` и `upd-auto.service` тот же предикат пропускает.

Код: `guard_active_legacy_admission`. Покрывающие и не запущенные тесты: `active_legacy_vpn_helper_and_operation_admissions_are_refused`, `inactive_legacy_services_are_allowed`.

Допущения: `ActiveState=active`. Значение `failed` в модели хранится как `active=false` и этот отказ не выбирает.

## 6. Timer, path и новая служба CM

Предикат guard покрывает только `upd-*.service` и точное имя `upd-helper.socket`. Timer, path и имена `cm-*` он пропускает. Дальше обычные условия `set` сами вызывают `stop`, `disable`, `enable` или `start`.

### 6.1 Активный timer

Состояние: к модели P добавлен только разбор одного вызова. `upd-auto.timer` есть, байты совпадают с генератором, `enabled=true`, `ActiveState=active`. Цель `set` — выключен и неактивен. Это служба «до файлов».

Событие: `Systemd::set` для `upd-auto.timer`.

Ответ при успешных systemctl и совпавшем повторном `inspect`: `Ok`. Предсказанные операции: `systemctl stop upd-auto.timer`, затем `systemctl disable upd-auto.timer`.

Diff при том же допущении: timer становится неактивным и выключенным. Журнал этого одним вызовом не завершается.

Инвариант: активный timer — предусмотренный переход, guard его не перехватывает.

Код: `guard_active_legacy_admission` возвращает `Ok`, потому что имя кончается на `.timer`. Затем `current.active && !state.active` вызывает `run(..., "stop")`, отличие `enabled` вызывает `disable`.

Допущения: ответы systemctl успешны, завершающий `inspect` равен цели. Иначе `set` возвращает `service state did not converge`, и транзакция уходит в rollback. Реальный systemctl не запускался. Чтобы такой unit вообще прошёл `plan`, его байты должны быть в `references`; при `watch=None` генератор timer `upd-auto.timer` туда входит.

### 6.2 Активный path

Состояние: `upd-mirrors.path`, `ActiveState=active`, цель — неактивен и выключен.

Событие: `Systemd::set` для этого unit.

Ответ при тех же допущениях успеха: `Ok`, предсказанные операции `stop`, затем `disable`.

Diff: path-unit переведён в неактивное выключенное состояние.

Инвариант: path обрабатывается так же, как timer, и иначе, чем `upd-*.service`.

Код: суффикс `.path` не делает предикат guard истинным. Дальше та же пара условий `stop`/`disable`.

Допущения: unit узнан `plan`. Для этого `watch` должен быть `/etc/pacman.d/mirrorlist`: только тогда `legacy::system_units` кладёт `upd-mirrors.path` в reference. В модели P зеркала не ведутся, поэтому path туда не включён; строка описывает вызов `set`, а не сводку модели P. Systemctl не запускался.

### 6.3 Новая служба CM во время rollback

Состояние: rollback уже прошёл проверку blobs и посторонней правки, фаза сохранена как `RollingBack`. `inspect` для `cm-vpn.service` показывает active и enabled. Цель этого цикла — default, то есть выключена и неактивна.

Событие: первый цикл `rollback` по службам с `before_files=false`.

Ответ при успешных systemctl и совпавшем повторном `inspect`: `Ok` для этого `set`. Предсказанные операции: `stop cm-vpn.service`, затем `disable cm-vpn.service`.

Diff: активная новая служба CM остановлена и выключена этим вызовом. Это предусмотренный откат новой службы.

Инвариант: имя `cm-vpn.service` guard пропускает и при `active=true`. Останов legacy `upd-vpn.service` в том же rollback по-прежнему описывается строкой 5.1.

Код: фильтр `!before_files` в `rollback`, затем `guard_active_legacy_admission` ложен для префикса `cm-`, затем `run("stop")` и `run("disable")`.

Допущения: unit CM уже установлен и реально active к этому `inspect`. В чистой модели P цель новой службы — «включена и неактивна», поэтому прямой успешный `set` её не стартует. Строка 6.3 берёт активность, которую rollback видит в `inspect`, откуда бы она ни появилась. Systemctl не запускался. Если повторный `inspect` не сойдётся с default, `set` вернёт `service state did not converge`, журнал останется `RollingBack`.

Подготовленный и не запущенный тест предиката: `active_timers_paths_and_new_cm_services_remain_allowed`. Он проверяет чистую функцию guard, без systemctl.

## 7. Активация после inspect и до mutator

Состояние: `Systemd::set` для `upd-vpn.service` уже получил от `inspect` неактивное включённое состояние. Guard на этом снимке прошёл. До первого `systemctl stop/disable/enable/start` этого вызова служба стала active.

Событие: продолжение того же `set`.

Предсказание кода: `stop` не вызывается, потому что запомненное `current.active` ложно. Если запомненное `enabled` отличается от цели, `disable` или `enable` вызывается по старому снимку. Повторного `inspect` между guard и mutator нет.

Ответ всей команды этой лентой не назначается. Завершающий `inspect` стоит уже после mutator. Совпадёт ли он с целью, зависит от того, что systemctl сделает с уже активной службой. Это не снято. Совпадение не объявляется доказанным закрытием гонки: mutator к тому моменту уже выбран по старому снимку. Активация после завершающего `inspect` этим вызовом `set` вообще не наблюдается.

Diff: не фиксируется. Возможные продолжения, если завершающий `inspect` увидит active и цель была неактивной: `service state did not converge`, затем rollback формы раздела 5. Это развилка, не результат прогона.

Инвариант: гонка между `inspect` и mutator остаётся открытой. Guard сужает окно до одного снимка и не держит состояние systemd атомарно.

Код: `Systemd::set` сохраняет `current` и дальше использует его в условиях `run` без нового `inspect`.

Допущения: активатор — другой субъект в этом промежутке. Лента такой гонки не исполняет.

Та же открытая форма есть у новой службы: если `inspect` увидел `cm-vpn.service` неактивным, а к моменту `start` она уже active, условие `!current.active && state.active` всё равно выбирает `start` по старому снимку.

Конкретная будущая проверка после установки пересобранного бинарника, не сейчас:

1. Собрать бинарник из исходников с SHA этого файла. Текущий `target/x86_64-unknown-linux-musl/debug/cm` для этой проверки не годится.
2. На disposable-макете поднять `upd-vpn.service` так, чтобы `systemctl show` внутри `Systemd::set` увидел `ActiveState=inactive`.
3. Отдельным субъектом перевести unit в `active` после этого `show` и до следующей команды `systemctl stop`, `disable`, `enable` или `start` того же вызова.
4. Записать, была ли команда `stop`, была ли команда `disable`/`enable`, код выхода, текст и фазу `journal.json`.
5. Повторить наблюдение для `cm-vpn.service` в промежутке между `inspect` и `start`.

Ожидание этой ленты — только собрать наблюдение. Закрытым окном оно станет лишь если будущий код повторно читает состояние непосредственно перед mutator и этот прогон это покажет.

## Подтверждение правок

- Живой `/var/lib/upd/vpn/bin/mihomo` при неактивном VPN unit отклоняется ранним `no_legacy_process` до `execute`. Hardlink с другим путём и тем же inode отклоняется сравнением device/inode.
- Точный managed path с суффиксом ` (deleted)` отклоняется для GUI без файла, для удалённого или заменённого ядра и для CLI, заново записанного теми же pinned-байтами.
- Чужой путь с basename `mihomo` и другим inode сводку модели P не запрещает.
- Пропавший PID, пропавший PID во время чтения `stat`, kernel thread с `PF_KTHREAD` и состояние `Z` пропускаются. Живой обычный PID, нечитаемый `stat` и порченый `stat` дают отказ и разные тексты.
- `manual::plan` и `Executor::execute` вызывают один `validate_plan` с `store=false` у плана. Symlink, special file и чужой владелец или mode `0o022` отвергаются до каталога журнала. На чистой модели P сводка — 19 и 2. На фикстуре регрессии — 6 и 0.
- Активные `upd-*.service` и `upd-helper.socket` в `set` получают отказ до mutator этого вызова.
- Активные timer и path, а также активная `cm-*` служба в rollback, guard пропускает; при цели «неактивна» код вызывает `stop`.

## Контрпримеры и оставшиеся границы

- Полностью снятый CLI и CLI с чужим digest отвергаются в `plan` раньше process audit. Фраза про running executable для них не печатается. Журнала при этом тоже нет.
- Если процесс стартовал по hardlink, затем сняты и hardlink, и `/var/lib/upd/vpn/bin/mihomo`, а `readlink` показывает путь alias с суффиксом ` (deleted)`, ни точное managed-имя, ни inode не совпадают. Текущий audit такой процесс пропускает.
- Socket внутри `/var/lib/upd` отвергается `validate_plan` раньше process audit. Живое ядро, оставившее special file в дереве, до строки 1.1 не доходит.
- В окне 5.1 журнал остаётся незавершённым. Компенсация может вызвать `daemon-reload` и всё равно не останавливает активную legacy-службу. Это не завершённый rollback.
- `ActiveState=failed` записывается как `active=false`. Такой unit guard не отвергает и `stop` по нему не выбирается.
- Глобальный user unit: `inspect` сразу возвращает `active=false`, не читая `ActiveState`. Guard активность `upd-notify.service` поэтому не видит. В модели P этой службы в наследстве нет.
- Гонка раздела 7 открыта.
- Прежний бинарник этих ветвей не содержит. Прежние 15/15 и числа 48/22 на них не переносятся.

## Проверки при установке пересобранного бинарника

До этих проверок: собрать бинарник из зафиксированных исходников и убедиться, что его SHA отличается от `b46c5d93…`. Затем, уже отдельным разрешённым прогоном:

1. Неактивный `upd-vpn.service` и процесс с inode `/var/lib/upd/vpn/bin/mihomo`, включая hardlink с другим путём. Ожидание по коду: код 1, текст про running executable, каталога `/var/lib/cm-migration` нет.
2. Три точных deleted path: CLI при файле с тем же pinned digest, отсутствующий GUI, удалённое или заменённое ядро. Ожидание по коду: тот же отказ до журнала.
3. Отдельно CLI, которого нет, и CLI с чужим digest. Ожидание по коду: отказ `check_file` или foreign binary, без текста process audit и без журнала.
4. Процесс basename `mihomo` с другим inode и чужим путём на чистой модели. Ожидание по коду: сам по себе сводку не запрещает.
5. Настоящие kernel thread и zombie не запрещают `plan`. Живой PID с нечитаемым `exe` запрещает. Точные строки malformed `stat` и пропавший `stat` остаются на `audit_processes`: production CLI подменяет `/proc` нельзя. Их пять регрессий нужно исполнить, сейчас они NOT_RUN.
6. Чистая модель: сводка `19 filesystem steps, 2 service steps` и отсутствие каталога журнала. Отдельно фикстура регрессии: 6 и 0. Затем по одному дефекту: вложенный symlink, socket `0600`, файл `0664`, чужой uid. Ожидание по коду: код 1, sentinel тот же, каталога журнала нет. Повторить дефект на `apply`: каталог по-прежнему не создаётся.
7. Запустить `upd-vpn.service` после inspect на границе `locked` и до inspect внутри `set`. Записать фазу журнала, был ли `daemon-reload`, был ли `stop`. Ожидание по коду: `stop` этой службы нет, журнал не `Committed`.
8. Активные `upd-auto.timer` и `upd-mirrors.path`: в журнале команд есть их `stop`. Активная `cm-vpn.service` на rollback останавливается. Активная `upd-vpn.service` guard не останавливает.
9. Прогон окна из раздела 7.
10. Исполнить десять подготовленных регрессий: пять process audit, две plan validation, три guard. COMPILE_PASS их не заменяет.

Реальные ответы systemctl, loginctl, pacman и поведение `/proc` до этого прогона остаются **NOT_RUN**.
