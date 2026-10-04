# Sol: итог проверки повторной ленты I01

[Лента Grokbuild](I01-GROKBUILD-SIMULATION-FOLLOWUP-TAPE-2026-10-04.md) рассмотрена по исходникам. Статус — SIMULATED, без runtime acceptance. По последнему поручению пользователя этот же вопрос дальше решает Sol самостоятельно: новых делегирований и заданий внешней симуляции нет. Реальные проверки остаются на этап установки собранного бинарника.

## Проверенные выводы

Ранние process audit и shared filesystem validator соответствуют указанным в ленте веткам: отказ до backup/journal при обнаружении legacy core, exact deleted path и unsafe data tree. Missing exe у подтверждённого kernel thread/zombie или исчезнувшего PID допускается; неизвестное состояние даёт отказ. Guard в `Systemd::set` вызывается после inspect и до mutators, запрещает наблюдаемую активность старой service/helper socket, но пропускает timer/path и CM service rollback.

Модель P даёт 19 файловых изменений: два data transfers, 11 replacements, одна новая wants link, пять retired paths. Найден один legacy unit, поэтому service steps два. Минимальная регрессионная fixture даёт 6/0. Это условные значения при явно заданном inventory и watch=None, не универсальные числа.

Ветка late activation 5.1 действительно может оставить RollingBack и вернуть recovery incomplete, сохранив активную старую службу; при чужой записи возможен отказ раньше сохранения RollingBack. Эти результаты не означают успешный откат. Терминальные repeat/recovery и старые failure evidence лента не превращает в runtime PASS.

## Контрпример устранён Sol

Лента нашла обход: core исполняется через alias hardlink, затем alias и установленный путь удалены. Известный inode отсутствует, а `/proc/PID/exe` показывает alias с deleted suffix. Старые exact-path и installed-inode comparisons пропускали такую картину.

В `manual::audit_processes` metadata теперь проходит `check_executable_identity`: known inode даёт прежний running-legacy refusal; неизвестный executable с `nlink=0` даёт `unidentified unlinked executable; cannot establish legacy quiescence`. Этот отказ применяется в раннем audit до journal и в повторных audits. Имена процессов и сигналы не используются.

Ограничение принято явно: неизвестные удалённые и анонимные executable также отклоняются, даже если фактически не являются UPD. При потерянном происхождении нельзя доказать, что это не удалённый legacy alias. Обычный посторонний executable с nlink>0 и другим inode проходит как раньше. Это сужение supported process layout, а не обещание определить происхождение всех произвольных копий core.

Подготовлена дополнительная регрессия: удержанный file descriptor синтетического core, удаление обеих hardlinks, nlink=0, отказ при неизвестной identity; known inode сохраняет исходный отказ. Реальный core и /proc не запускаются/не подменяются этим тестом. Тест НЕ исполнялся.

## Уточнение вывода о гонке

Последняя фраза раздела 7 ленты слишком сильна: повторный inspect непосредственно перед mutator не делает переключение атомарным. Он только сдвигает окно. Даже единичный успешный runtime прогон не доказывает отсутствие гонки. Поэтому race after inspect остаётся ограничением текущего quiescent policy; никакого нового обещания атомарного freeze нет.

## Проверки и итог

Sol выполнил только форматирование, `cargo check --offline --locked --tests` и проверку whitespace; compile check завершился exit 0. Все текущие source SHA повторно сверены со [сводкой](i01-evidence/simulation-followup-review-2026-10-04/summary.json). Runtime tests NOT_RUN. Теперь подготовлено 11 новых регрессий после первого fixture PASS: 6 process audit, 2 plan validation, 3 service guard. Прежний binary OLD_NOT_REBUILT и прежние 15/15 относятся к более раннему коду.

Повторная лента принята как анализ модели с описанной поправкой. Кодовый контрпример устранён и проверен компилятором. Новая внешняя передача не нужна. Real systemd/loginctl/package ownership/process races и сохранение реальных данных остаются NOT_RUN до установки; весь I01 не объявлен принятым.
