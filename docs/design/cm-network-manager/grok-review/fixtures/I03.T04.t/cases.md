# Fixtures `I03.T04.t`

Дата: 2026-10-06. Входов: 9. Классификатор тела для `negotiate_source`. Composer переносит каталог без изменений. Тест пишется до `src/sources/parser/detect.rs`.

Мок транспорта считает вызовы fetch. Первый ответ — байты fixture. «Retry» значит `BodyRejection::RetryableUnusableBody` и второй вызов со следующим User-Agent. «Terminal» значит второго вызова нет.

Пустое тело по [I03-DECISIONS](../../I03-DECISIONS-2026-10-06.md) D3 — отказ источника. Повтор со следующим UA в том решении назван для HTML и заглушки, не для пустого тела.

Двойной base64 D3 не выбирал. Эти два файла фиксируют границу цикла: два слоя списка URI принимаются, третий слой — terminal отказ без retry. Один слой, который отвергает двойную подписку, или цикл глубже двух — расхождение, его пишут в отчёт, файлы не меняют.

Маркеры `CMFIXT-html-stub`, `CMFIXT-bom-pass`, `CMFIXT-restricted`, `list-secret` не попадают в ошибку, сводку и `Debug`.

| Файл | Решение | Риск |
|---|---|---|
| `html-stub.html` | retry следующим UA | HTML-заглушка стала terminal отказом или принята как узлы. В тексте ошибки нет `CMFIXT-html-stub` |
| `empty.txt` | terminal, ноль дополнительных fetch. Класс `InvalidSyntax` | пустое тело ушло в retry и крутится по всем UA |
| `base64-html.txt` | один decode, внутри HTML → retry, как `html-stub.html` | base64 от HTML принят как подписка или стал terminal без retry |
| `base64-of-base64.txt` | два decode, один узел `edge-b64`, без retry | второй слой не развернули и ушли в бесконечный retry либо отвергли рабочую двойную подписку |
| `base64-triple.txt` | terminal `Malformed`, без retry | третий слой тоже разворачивается |
| `yaml-looking-uri-list.txt` | список URI, два узла (`edge-list`, `edge-ss-list`), без retry | текст похож на не-YAML, классификатор делает retry вместо импорта строк |
| `json-with-bom.json` | один ведущий UTF-8 BOM снимается, JSON принимается, один узел, без retry. Пароль `CMFIXT-bom-pass` только в blob | BOM даёт `InvalidSyntax` и retry той же заглушки по кругу |
| `body-over-limit.recipe` | terminal `BodyTooLarge`, без retry. Тело — 8388609 байт `0x61`, в git его нет | 8388608 + 1 байт принят или ушёл в retry |
| `unsupported-semantics.json` | terminal `RestrictedNodeOption`, ровно один fetch | годный JSON с `interface-name` классифицирован как непригодное тело и повторён со следующим UA. В ошибке нет `CMFIXT-restricted` и `eth0` |
