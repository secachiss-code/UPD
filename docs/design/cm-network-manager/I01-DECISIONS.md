# I01: решения и оставшиеся рубежи

Дата: 2026-10-03. Это рабочая запись I01, не разрешение на I02–I18. Исполнение тестов отделено от координатора.

## D01: фактический baseline

Установленный UPD и исходники CM 0.2.8 — разные исполняемые состояния. Тесты текущих исходников не исправляют установленную систему автоматически. Установка в этой работе не выполняется; бинарник тестовой сборки используется только в disposable fixtures.

Различающиеся full node definitions и effective DNS/rules исключают сравнение «тот же список = одинаковый конфиг». Исторический отказ остаётся unknown без contemporaneous snapshot; исправление UA/DNS не назначается по одной строке ошибки.

## D02: limited parity вместо вмешательства в живую сеть

Для FlClash используется доказанный matching-version desktop IPC в собственном worker. Wrapper протокол не заменяет транспортное ядро; несработавший standalone CLI flag не считается отказом VPN. [Контрольный протокол](I01-PARITY-PROTOCOL.md) сначала подтверждает config preparation без внешней сети, затем допускает ограниченный proxy pilot.

Выполняющееся окружение FlClash на хосте сохраняется. Результаты в его внешнем сетевом окружении имеют отдельный scope и не принимаются как доказательство host TUN independence или причина прежнего отказа.

## D03: безопасный отказ legacy install

Вместо непроверенного переносa добавлен read-only guard. Известный legacy/ambiguous footprint блокирует установку до config save и install operations. Это ограниченное исправление предотвращает прежние stop/remove/rename при таком переходе; поддержка автоматической миграции временно отсутствует.

Варианты полноценного переноса: memory-only rollback; durable journal/recovery; ручной runbook. Memory-only не закрывает прерывание процесса. Предпочтительный будущий вариант — production-used MigrationPlan с durable backup/journal и injectable executor; ручной runbook может быть fallback после проверки ownership, не командой удалить старые данные для обхода guard.

## D04: следующий объём I01.T04, без расширения N

1. Определить реально поддержанные legacy versions/layouts и ownership бинарников/units/hooks/desktop/cron. Unknown/foreign/package-managed варианты отклонять до изменений.
2. Спроектировать persisted plan: source/target generations, private backup, permissions, original service enabled/active state, operation locks и entrypoint переход.
3. Реализовать executor, shared production/fixtures. Защита legacy helper и новых операций требует больше, чем однократный Status GET.
4. Покрыть всё install transaction boundary, включая ошибки после переноса: binary/GUI/units/hooks/daemon-reload/enable/core start. Fresh CM targets ownership также остаётся обязательным вопросом.
5. Провести fault injection до/после journal boundaries, прекращение subprocess и recovery новым процессом; сохранить данные и состояния служб при собственном recovery failure.
6. Независимо принять C07–C11/C15–C16. Только затем рассматривать снятие C17 blocker для конкретного проверенного layout. Реальный host install и package build этим документом не разрешаются.

## D05: качество приёмки

Количество прошедших unit tests не компенсирует missing network/migration scenarios. Scoped PASS C17 и local regressions не закрывают I01 целиком. При отсутствии privileges/prerequisites явно фиксируются BLOCKED/NOT_RUN, при недостаточной причинности — INCONCLUSIVE. Первые неудачные попытки harness сохраняются с причиной, а исправления harness отделены от product fixes.

## D06: результат pilots и диагностический недостаток

WS C06: A1 4/10, B 2/10, A2 0/10. TCP/REALITY C19: 0/10 во всех трёх фазах, live controls 0/4. В C19 все 37 запросов, включая warm-up, завершились curl35. Это общая наблюдаемая TLS-ошибка клиента; CONNECT status и точная причина в исходном драйвере не сохранялись. Преимущество ядра и причина прежнего отказа остаются INCONCLUSIVE. Различающиеся сохранённые UPD/Fl profiles — доказанный факт, их влияние на прежний отказ пока гипотеза.

Следующий bounded контракт C20 сохраняет недостающий этап ошибки на четырёх запросах, вместо нового latency pilot. Сравниваются own native/own Fl/live proxy/direct host path; последний всё ещё может проходить через живой TUN. Настройка живого FlClash, остановка VPN, отключение проверки сертификатов и обновление подписок не разрешаются. Отчёт исполнителя и независимое заключение имеют приоритет для финальных статусов; незавершённая проверка сохранности не получает PASS.

## D07: синтез 2026-10-03, C20 уже исполнен

C20 исполнен до этой записи и в D06 ещё назван следующим контрактом. Итог чтения evidence, без нового прогона: [baseline](I01-BASELINE-REPORT.md). Собственные worker-ы остановились на TLS после CONNECT 200, стадия ядра UNKNOWN. Живой proxy и запрос без явного proxy дали HTTP 204 на одном URL. C06 и C19 сохраняют свои результаты. Историческая причина журнала остаётся unknown. Отдельный C21 не создаётся.

Q20: молчаливый перенос по-прежнему запрещён. Проект транзакции — [I01-MIGRATION-DESIGN.md](I01-MIGRATION-DESIGN.md). Guard остаётся. Дополнительно назван открытый путь: не-install команда и fallback `conf_path` / `state_dir`. Он не исполнялся и держит применение миграции в NO-GO.

Q21 не закрыт. Один HTTP 204 и валидная подготовка config не подтверждают исправление подписки.

G0 разрешает бумажную модель I02.T01–T03 и не разрешает реализацию I02, старт I03–I18 и заявление об исправленном VPN. I02 остаётся PLANNED, пока T04 и T05 не исполнены по [контракту хранения](I02-TEST-CONTRACT.md).

## D08: проверка координатора 2026-10-04, волна 0

[Проверка и контракт волны 0](WAVE0-REVIEW-AND-TEST-CONTRACT.md) ничего не исполняли и код не меняли.

Пробел C17a уточнён чтением кода (R02). При `/etc/upd.conf` без `/etc/cm.conf` команды с `Config::save` пишут прямо в конфиг UPD: `cm lang КОД`, `cm mirrors add/del`, переключение режимов VPN. Любой `save()` также создаёт `.vpn-config.lock` в `state_dir()/vpn`, то есть в `/var/lib/upd/vpn` при legacy-следе. Хост сейчас в таком состоянии. Отладочный `cm` под root хоста запрещён. C17a исполняется как W0-1 только в user/mount namespace с overlay. Guard в этой волне не меняется. Исправление будет отдельным заданием на код после W0-1.

G0 условный (R06), пока W0-2 не объяснит смену digest дерева данных FlClash. Бумажная модель I02.T01–T03 — черновик, не принятый результат (R08).

## D09: поручение кодеру 2026-10-04, исправление C17a

Пользователь поручил все возможные задачи кодера последовательно, самостоятельно или с Luna, без Astra. До исправления отдельный исполнитель Luna проверил startup по исходникам и собрал текущий бинарник offline. W0-1 остановился до запуска CM: среда запретила NETLINK_ROUTE socket внутри namespace. Это BLOCKED, а не исполненный characterization K0–K6.

После нового поручения кодеру добавлен startup guard для всех root-команд при legacy-следе, до Config::load/save. Guard install сохранён независимо от uid; fresh и CM-only разрешены. Исправление проверяет отдельный исполнитель на временных файловых фикстурах, с явным ограничением: они не заменяют namespace CLI evidence. Исторические формулировки D08 и W0 о неизменном guard относятся к исходной волне. Актуальный результат и остановка очереди: [отчёт кодера](CODER-QUEUE-2026-10-04.md).

I01 не принят: транзакция, ownership, legacy operation locking и recovery ещё не реализованы и не проверены; G0 остаётся условным. Зависимая реализация I02–I18 не начинается. Снятие guard, установка и package workflow не выполняются.

## D10: реализован явный переход ручного UPD 0.2.7

Пользователь уточнил, что реализация транзакции входит в часть Sol, и поручил продолжить. Отсутствие кода из D09 устранено: добавлены production-used journal/backup executor, ownership policy с закреплёнными references, legacy locks, компенсация, recovery отдельным процессом и explicit CLI. Обычные команды сохраняют guard; unattended перенос не включён. Поддержан quiescent manual UPD 0.2.7, прочие layouts и активные процессы отклоняются.

Независимое ревью Luna: PASS_WITH_SCOPE_LIMITS; 15 migration tests, 5 startup guard, 4 language regressions, 12 help invocations и offline binary build прошли. Найденные дефекты root file path и foreign journal staging исправлены и повторно проверены. [Реализация и границы](I01-MIGRATION-IMPLEMENTATION.md), [сводка с hashes](i01-evidence/migration-implementation/summary.json), [ревью](i01-evidence/migration-implementation/review.md).

Это закрывает отсутствие реализации, но не реальную Linux/systemd приёмку: adapters package ownership и systemd, root CLI и реальные helper races ещё не исполнены на disposable Linux. Предыдущие сетевые unknown/BLOCKED сохраняются. I01.T04/T05 не объявлены принятыми, зависимые I02–I18 не начаты; пакет, host install и live network не менялись. Astra не использовалась.

## D11: возвращённая симуляция и исправления по ревью Sol

Пользователь выбрал Grokbuild simulation без VM и отложил реальные проверки до установки собранного бинарника. [Лента](I01-GROKBUILD-SIMULATION-2026-10-04.md) остаётся SIMULATED. По [ревью Sol](I01-GROKBUILD-SIMULATION-SOL-REVIEW-2026-10-04.md) Luna xhigh последовательно исправила owned legacy core/deleted process audit, shared read-only plan validation и late-observed old service activation guard. Sol проверил исходники и подготовленные 10 новых регрессионных тестов. Compile-only `cargo check --offline --locked --tests` — PASS; новые runtime tests NOT_RUN, существующий binary не пересобран. [Сводка source hashes](i01-evidence/simulation-fixes-2026-10-04/summary.md).

Изменённые ветки требуют [повторной симуляции](I01-GROKBUILD-SIMULATION-FOLLOWUP-2026-10-04.md). Активные timer/path admissions, snapshot в начале apply и rollback вместо продолжения forward подтверждены как текущая модель. Атомарная защита systemd races, real adapters и точное failed состояние не доказаны. Прежние fixture PASS не переносятся на изменённый код; I01 остаётся IN_PROGRESS.

## D12: итог повторной ленты без дальнейшей передачи

Пользователь запретил дальнейшую передачу этого же вопроса другим исполнителям. Sol сам проверил [повторную ленту](I01-GROKBUILD-SIMULATION-FOLLOWUP-TAPE-2026-10-04.md), исправил новый deleted-hardlink bypass с явным отказом при unknown unlinked executable и подготовил дополнительную регрессию. Compile-only check прошёл; runtime tests NOT_RUN, binary не пересобран. Неизвестные anonymous/deleted executables также unsupported. Дополнительный inspect не считается атомарным freeze. [Итоговое ревью](I01-GROKBUILD-SIMULATION-FOLLOWUP-SOL-REVIEW-2026-10-04.md). Новый внешний прогон не запрашивается; реальные проверки отложены до установки.
