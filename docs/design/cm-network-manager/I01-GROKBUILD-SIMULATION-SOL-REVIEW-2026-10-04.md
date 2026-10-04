# Sol: ревью ленты Grokbuild I01

Источник: [лента Grokbuild](I01-GROKBUILD-SIMULATION-2026-10-04.md). Это анализ исходников, не запуск бинарника, systemd или package manager. Реальные проверки по поручению пользователя отложены до установки собранного бинарника. Оригинальная лента сохраняется.

## Подтверждённые исправления

1. `manual::no_legacy_process` проверял device/inode CLI и GUI, но не `/var/lib/upd/vpn/bin/mihomo`. Живое legacy-ядро при неактивном VPN unit могло пройти audit. Удалённый или заменённый executable также нельзя надёжно исключить только сравнением с текущим inode установленного файла. Исправление выполнила Luna xhigh, Sol проверил код. Добавлен owned core, точный managed/deleted pathname, device/inode для aliases и отказ при неизвестном executable state. При missing exe допускаются исчезнувший PID, подтверждённый zombie или kernel thread. Пять регрессий подготовлены; сборка и runtime validation пока NOT_RUN. [Отчёт изменения](I01-CORE-QUIESCENCE-CODE-CHANGE-2026-10-04.md).
2. `manual::plan` не выполнял полную проверку содержимого деревьев, уже существовавшую перед записью журнала в `Executor::execute`. Поэтому symlink, special file или foreign owner внутри данных выявлялись только в apply. Общий read-only `Executor::validate_plan` извлечён из прежнего executor preflight и вызывается в manual plan и execute. Добавлен ранний production process audit до вывода сводки и записи backups. Sol проверил extraction и подготовленные две регрессии по коду; runtime validation NOT_RUN. [Отчёт изменения](I01-PLAN-PREFLIGHT-CODE-CHANGE-2026-10-04.md).

3. `Systemd::set` мог остановить старую service/helper socket, ставшую активной после предыдущего audit. Luna xhigh добавила pure guard непосредственно после inspect и до service mutators: активные старые `.service` и `upd-helper.socket` дают отказ. Timer/path admissions можно остановить; новые CM services можно остановить при rollback. Sol проверил этот вызов и три подготовленных unit tests по коду; runtime validation NOT_RUN. Исправление не заменяет атомарную блокировку systemd admissions: активация после самого inspect остаётся непроверенной гонкой.

## Подтверждённое поведение и ограничения

- Активность timer/path — состояние admissions, а не доказательство активной операции. Эти admissions останавливаются перед файлами и их состояние переносится. Автоматический запуск связанной операции после включения admissions остаётся предметом реальной проверки.
- `ServiceState` хранит enabled/active, не точное failed состояние systemd. Это граница модели, не обещание восстановить failure diagnostics.
- `plan` не закрепляет поколение для будущего apply; apply получает новый снимок и повторно сверяет поколения под locks. Это не применение ранее подписанного плана.
- `recover` компенсирует незавершённый переход назад. После terminal journal он идемпотентно не меняет дерево; проверка committed результата реализована в повторном apply. Recover не является проверкой целостности завершённой установки.
- Обрыв backup preparation до первого durable journal требует ручного разбора retained backup; переноса source/service к этому моменту ещё нет.
- Foreign edit и corruption намеренно блокируют compensation; формулировка «recover всегда возвращает» применима только при целых blobs, допустимом текущем дереве и успешных adapters.
- Секреты имеются в исходных/перенесённых данных и private backup, а не только в backup. JSON journal и CLI не содержат secret bytes.
- Числа 48/22 — предсказание выбранной картины. Без полного inventory/manifest модели это не универсальный вывод о числе шагов.
- Гонки после последнего process/service audit, effective global user unit behavior и реальные package manager ответы остаются непроверенными. Симуляция не доказывает их безопасность.

## Статусы

Лента: SIMULATED, рассмотрена Sol. Все три исправления последовательно выполнила Luna xhigh, Sol проверил исходники и подготовленные регрессии. `cargo check --offline --locked --tests` — COMPILE_PASS; это не исполнение тестов. Текущие source SHA сверены Sol со сводкой, все совпали. [Compile-only evidence](i01-evidence/simulation-fixes-2026-10-04/summary.md). Новые исправления не покрыты старым fixture PASS и прежним binary digest; binary OLD_NOT_REBUILT. Runtime tests и real CLI/systemd/package ownership: NOT_RUN до установки. Очередь остаётся на I01; исходные сетевые unknown не переписываются как PASS. [Повторная симуляция изменённых сценариев](I01-GROKBUILD-SIMULATION-FOLLOWUP-2026-10-04.md).

Проверку `PF_KTHREAD` и формата proc stat сверили с первичными исходниками Linux: [sched.h](https://raw.githubusercontent.com/torvalds/linux/master/include/linux/sched.h), [proc/array.c](https://raw.githubusercontent.com/torvalds/linux/master/fs/proc/array.c). Это проверка основания кода, не прогон process audit на хосте.
