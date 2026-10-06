# Fixtures `I03.T04.b`

Дата: 2026-10-06. Входов: 27. Это данные для теста, который пишется до `native/common.rs`. Composer переносит каталог без изменений и подключает файлы через `include_str!`. До валидатора тест не проходит.

Точка входа — типизированный builder вложенной map со схемой `fixture-opts` ниже, не весь документ подписки. Исключение: `duplicate-key-literal.json` и `depth-over-limit.json` идут в строгий разбор JSON, который кормит builder. Сравнивать `definition_digest` канонического результата. `source_body_sha256` у `order-a.json` и `order-b.json` разный и здесь не критерий.

Текст `Display` и `Debug` любого отказа из этой таблицы не содержит имени ключа и значения. Маркеры `CMFIXB-*` в отказе не встречаются.

Код ошибки — предложение, если такого варианта в `ParserError` ещё нет. Сведение к уже существующему варианту записать в отчёт подзадачи. Файл при этом не менять.

## Схема `fixture-opts`

Ключи строго в нижнем регистре. Иного ключа нет.

| Ключ | Тип | Правило |
|---|---|---|
| `path` | строка | 1…256 байт UTF-8. Пустая строка — отказ |
| `count` | число | целое JSON, не строка |
| `flag` | bool | JSON `true`/`false`. Число не подходит |
| `items` | список строк | 0…128 элементов. Пустой список принимается. Элемент — непустая строка без управляющих и bidi, не длиннее 32 байт |
| `mode` | enum | `plain` или `marked` |
| `left`, `right` | строка, необязательны | вместе — отказ. Одна сторона или ни одной — принять |
| `alpha`, `beta` | bool, необязательны | вместе — отказ. Одна сторона или ни одной — принять |
| `token`, `token-name` | строка, необязательны | только парой. Один без другого — отказ |

Лимит строки 256 совпадает с уже существующим потолком имени HTTP-заголовка в `src/sources/parser/native/transport.rs`. Лимит списка 128 совпадает с потолком числа заголовков там же. Вложенность — `MAX_NATIVE_DEPTH` = 64: значение на глубине 65 — отказ `TooDeep`. Builder не поднимает этот потолок.

Управляющие символы — `char::is_control`, включая NUL, CR, LF. Этого мало для bidi: `U+202E` имеет категорию Cf и `is_control` для него ложен. Дополнительно отвергать `U+200E`, `U+200F`, `U+202A`–`U+202E`, `U+2066`–`U+2069`.

Регистр ключа. `path` и `Path` в одном объекте — два ключа для JSON. Оба доходят до builder. Ожидание: `DuplicateKey`, не выбор одного из них и не `UnsupportedField` на `Path` при живом `path`.

## Случаи

| Файл | Результат | Класс | Риск |
|---|---|---|---|
| `unknown-nested-key.json` | отказ | `UnsupportedField` → `UnsupportedNodeField` | неизвестный ключ `not-a-key` удалён молча. В отказе нет `not-a-key` и `CMFIXB-unknown-value` |
| `duplicate-key-literal.json` | отказ | `DuplicateKey` | второе значение затирает первое. В отказе нет `CMFIXB-secret-one` и `CMFIXB-secret-two` |
| `duplicate-key-case.json` | отказ | `DuplicateKey` | `Path` принят рядом с `path`. В отказе нет `CMFIXB-case-lower` и `CMFIXB-case-upper` |
| `control-nul.json` | отказ | `InvalidNode` | NUL в строке принят. Это не синтаксическая ошибка JSON |
| `control-cr.json` | отказ | `InvalidNode` | CR в строке принят |
| `control-lf.json` | отказ | `InvalidNode` | LF в строке принят |
| `control-bel.json` | отказ | `InvalidNode` | прочий управляющий (`U+0007`) принят |
| `control-bidi.json` | отказ | `InvalidNode` | `U+202E` принят, потому что `is_control` ложен |
| `string-at-limit.json` | принять | — | строка ровно из 256 байт `a` отвергнута |
| `string-over-limit.json` | отказ | `InvalidNode` | 257 байт приняты или обрезаны |
| `empty-list.json` | принять | — | пустой `items` отвергнут или выкинут |
| `list-at-limit.json` | принять | — | ровно 128 элементов отвергнуты |
| `list-over-limit.json` | отказ | `InvalidNode` | 129 элемент принят. Это не документный `TooManyEntries` |
| `depth-over-limit.json` | отказ | `TooDeep` | 64 вложенных объектов, строка на глубине 65 принята |
| `type-string-for-int.json` | отказ | `InvalidNode` | строка `"1"` принята как `count` |
| `type-map-for-list.json` | отказ | `InvalidNode` | map принят как `items` |
| `type-number-for-bool.json` | отказ | `InvalidNode` | число `1` принято как `flag` |
| `exclusive-left-right.json` | отказ | `InvalidNode` | заданы и `left`, и `right` |
| `exclusive-alpha-beta.json` | отказ | `InvalidNode` | заданы и `alpha`, и `beta` |
| `exclusive-left-only.json` | принять | — | одна сторона пары отвергнута |
| `exclusive-alpha-only.json` | принять | — | одна сторона второй пары отвергнута |
| `pair-token-only.json` | отказ | `InvalidNode` | `token` без `token-name`. В отказе нет `CMFIXB-token` |
| `pair-name-only.json` | отказ | `InvalidNode` | `token-name` без `token`. В отказе нет `CMFIXB-token-name` |
| `pair-both.json` | принять | — | полная пара отвергнута |
| `order-a.json`, `order-b.json` | принять | digest равен | разный порядок ключей дал разный `definition_digest`. Сырой sha256 файлов разный и не сравнивается |
| `order-value-differs.json` | принять | digest другой | тот же порядок, что у `order-b.json`, но `count` = 3. Digest совпал с `order-b.json` — канонизация выкинула значение |
