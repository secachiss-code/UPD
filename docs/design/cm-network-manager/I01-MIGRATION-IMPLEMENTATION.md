# I01.T04: реализация транзакционной миграции

Дата: 2026-10-04. Исполнитель кода — Sol; независимый тестировщик и рецензент — Luna. Astra не использовалась. Это код и disposable-проверки, без установки пакета, изменения служб/сети хоста или публикации.

Обновление после ленты Grokbuild: исправлены пробелы process quiescence, ранней валидации plan и отказа при наблюдаемой поздней активности старой службы. [Ревью Sol](I01-GROKBUILD-SIMULATION-SOL-REVIEW-2026-10-04.md). Результаты 15/15 ниже относятся к предыдущей версии исходников и бинарника. Новые регрессии подготовлены, но не запускались; `cargo check --offline --locked --tests` прошёл, бинарник ещё не пересобран. Реальные проверки отложены до установки собранного бинарника по поручению пользователя.

Дополнительное ревью повторной ленты выполнил Sol без делегирования: [итог](I01-GROKBUILD-SIMULATION-FOLLOWUP-SOL-REVIEW-2026-10-04.md). Неизвестные удалённые/анонимные executable теперь отклоняются по nlink=0: иначе нельзя исключить legacy core через удалённый hardlink. Обычный посторонний executable с доступным inode не отклоняется по basename. Compile-only check новой версии прошёл; runtime tests остаются NOT_RUN.

## Поведение

Добавлены production-команды:

```text
cm migration plan
cm migration apply
cm migration recover
cm install --migrate-upd
```

`plan` проверяет поддержанный layout, ownership и состояния служб без записи и без остановки процессов. `apply` и явный флаг install используют один исполнитель транзакции. `recover` загружает сохранённый журнал новым процессом и компенсирует незавершённую операцию. Повтор завершённого перехода сверяет результат и не копирует credentials повторно. Повтор восстановления завершённой компенсации ничего не меняет. После компенсации новый переход требует отдельного архивирования private-журнала; автоматического удаления резервной копии нет.

Обычный install не начинает перенос молча. Guard старых путей сохраняется для обычных команд, а незавершённый журнал и эксклюзивная transaction lock блокируют новые root-команды CM даже после удаления старых путей. Миграционные команды проверяют реальный euid; переменные тестового окружения не заменяют права root и не меняют production filesystem root `/`.

## Поддержанный layout и ownership

Первый production policy поддерживает ручную UPD 0.2.7 в systemd-окружении:

- CLI и при наличии GUI сверяются с закреплёнными SHA-256 артефактов 0.2.7; установленный бинарник для проверки версии не исполняется.
- Units, hooks, cron, desktop, icons и polkit сверяются побайтно с замороженными генераторами и ресурсами commit `07ef2381fb962fe74958c504a896c584f6a4e4be`.
- Владельцы, права, типы и предки проверяются. CM targets должны отсутствовать; конфликт старого/нового, symlink target/ancestor, неизвестные overrides/drop-ins и чужие файлы отклоняются до мутации.
- Для старых units требуется `NeedDaemonReload=no`: устаревшее состояние manager cache отклоняется.
- Package ownership запрашивается read-only через локальный pacman/dpkg-query/rpm для всех известных persistent legacy paths. Package-owned и неоднозначный результат дают отказ.
- `plan` теперь проходит общий read-only filesystem validator, включая содержимое деревьев; unsafe nested entries отклоняются до успешной сводки.
- Данные конфигурации/подписок/назначений копируются целиком, с исходными bytes, uid/gid и mode. Строковые замены внутри credentials, URLs и пользовательских профилей не выполняются.

Активные VPN/helper/операции должны быть остановлены заранее; активные helper socket и оставшиеся специальные файлы не поддерживаются. Переход глобальных user units допускается после выхода пользовательских сессий: частные user overrides и home/panel references этот policy не переписывает. Неизвестные домашние ярлыки сохраняются; CM applet нужно заново выбрать в настройках панели. Если установлен legacy GUI, рядом с новым CLI нужен текущий `cm-cosmic`, чтобы не оставить неподдержанный GUI переход.

## Транзакция и восстановление

`src/migration/transaction.rs` — общий production/fixture executor. `manual.rs` строит проверенный план, `legacy_v027.rs` хранит exact-match references. `cmd_migration` в main строит новый layout теми же current generators, что обычная установка: CLI/GUI, system/user units, hooks, dispatcher и polkit.

До мутаций записываются private backup blobs и JSON journal в `/var/lib/cm-migration` (0700; файлы 0600). Журнал хранит schema, phases, metadata/digests исходного и целевого поколения, исходные enabled/active состояния, lock paths, созданные lock files и шаг исполнения. Содержимое credentials в JSON и сообщения CLI не входит.

Порядок: общий transaction lock → legacy heavy lock → VPN/config/subscription locks → повторная проверка поколений и quiescence → отключение старых admissions → все новые файлы/данные → retirement старых owned paths → daemon reload → перенос enabled/active состояний → durable commit. Корневые пути не берутся из `CM_*`/`UPD_*`. Production проверяет процессы до сводки plan и записи backup, повторяет аудит после получения locks и остановки admissions. Аудит `/proc` учитывает CLI, GUI и owned legacy core по device/inode и точному managed/deleted path, а не basename процесса или старому PID. Перед service mutators проверяется наблюдаемая активность старых service/helper socket; при активности — отказ. Это не атомарный freeze: гонка после последнего inspect остаётся для реальной проверки.

До каждого service/file/reload шага журнал синхронизируется. Ошибка выполняет компенсацию в обратном порядке. Если ещё не было мутаций, отказ не перезаписывает службы и существующие файлы/lock inodes. Если процесс прерван, тот же журнал восстанавливается отдельным процессом. Частичная staging-запись признаётся своей только по точному пути, owner/mode и содержимому, являющемуся префиксом журналированного backup. Несовпадающий файл сохраняется и восстановление отказывает.

Перед компенсацией проверяются все backup digests и все изменяемые пути. Чужое изменение не удаляется ради успешного отката. Отказ восстановления оставляет журнал и backup, блокирует root-команды CM и позволяет повторить `recover` после устранения причины. Безопасное повторение завершённого recovery не создаёт вторую установку.

Mirror probes, refresh подписок, редактирование home, Garuda aliases и live VPN переключение в транзакцию не входят. При ручном переходе переносится существующий core и его данные; активный core в этом policy не мигрируется и не запускается ради проверки сети.

## Проверки и границы приёмки

Luna завершила независимые проверки в `tests/audit_i01_migration.rs`: **15 PASS, 0 FAIL**. Итоговые регрессии: 5/5 startup guard, 4/4 language tests, 12/12 реальных help-вызовов в шести локалях; offline locked build бинарника CM прошёл. Вердикт — **PASS_WITH_SCOPE_LIMITS**. [Сводка, команды и SHA-256](i01-evidence/migration-implementation/summary.json), [независимое ревью](i01-evidence/migration-implementation/review.md).

В проверках обнаружены и исправлены два дефекта: trailing slash при восстановлении корневого regular file и удаление постороннего файла под фиксированным временным именем журнала. Регрессионные сценарии прошли. Исходные неуспехи сохранены рядом с успешными прогонами; итоговые hashes относятся к проверенным исходникам.

Покрываются successful migration, bytes/owner/mode, исходные и целевые service states, ошибки на durable boundaries, завершение subprocess и recovery, retry после recovery failure, corrupted backup, постороннее изменение/стейджинг, symlink/ancestor/overlap, busy flock, сохранение inode при отказе до мутаций, pending startup gate, повтор commit и полная композиция exact 0.2.7 policy с executor.

Сетевые/root namespace ограничения прежней среды не выдаются за PASS. Реальные systemd/package-manager adapters на disposable Linux и root CLI ещё требуют отдельной проверки; fake services доказывают логику журнала и компенсации, а не поведение служб хоста. Package layouts, изменённые legacy versions и активный VPN остаются неподдержанными, а полный I01 не объявлен принятым.
