# I03.T04: строгий import и ручной сервер

I03.T03 reviewed/COMPILE_PASS, runtime NOT_RUN. Следующий этап выполняется последовательно:
native JSON/YAML validation, затем URI/base64 conversion, затем public manual/import pipeline.
Sol ведёт review, Luna xhigh — реализацию. Приложение, тесты, службы, ядро и provider URLs
не запускать; разрешены чтение pinned upstream source/docs и compile-only проверки.

## Граница и результат

Pure parser принимает bounded bytes, explicit format и exact core pin; возвращает private
typed definitions и constrained defaults. Никаких URL-запросов, чтения host paths, converter,
legacy singleton или записи Store внутри parser. Разбор должен завершиться до публикации.
Невалидный ответ не меняет Store. Public entrypoints связывают распарсованный ответ с тем
же `Negotiated` body, который породил actual UA; local manual definition создаёт отдельный
Source и один Node. Профили ссылаются на независимые Source ID существующей модели.

Сохранять все принятые поля и влияющие defaults без silent dropping. Неизвестные поля,
unsupported features/transports и host controls отвергать typed safe error. Ошибки и Debug
не содержат исходные JSON/YAML, URL, UA, UUID, password или ключи. Номер узла безопасен,
произвольное имя поля и текст serde/core ошибки — нет.

## Native validation (первая часть T04)

JSON/YAML: до allocation-heavy обработки ограничить body 8 MiB; reject duplicate keys,
multiple documents, YAML tags/merge/aliases (без expansion), excess nesting >64 и entries
>1M. Максимум 4096 nodes. UTF-8, object top-level, nonempty proxies обязательны.
YAML не превращать через `Value` с потерей duplicate keys. Общий strict visitor либо
preflight token reader должен сохранять границу ресурсов; serde recursion cap не отключать.

Принятый native subset: `proxies` и пять constrained global defaults T03. Остальные known
native sections нельзя молча выбросить: host controls restricted; groups/rules/providers
unsupported до отдельной реализации с сохранением семантики. Remote providers отвергать
без сетевых запросов. File providers также отвергать, пока нет explicit confined resolver;
capability approval сама по себе не разрешает читать произвольный файл. Inline providers
можно добавить только как полностью валидируемые определения с ограничениями и origin;
иначе typed unsupported. Документировать фактический conservative subset.

Полные protocol-specific структуры валидировать по mihomo1.19.32/commit
88dcbf7f1614a67c3b36b848ee3592dfa92ada36. Только первичные pinned sources/docs.
Общие обязательные поля: nonempty bounded name, server host/IP (без URL/control/space),
integer port 1..65535, exact protocol type. Дубликаты имён отвергать. TLS, REALITY,
transport options, auth и все вложенные структуры проверять по схемам. UUID/ключи и другие
секреты сохраняются в private definition. Cert/key file paths и uncontrolled host reads
не разрешать. Unsupported protocol-native options не притворяются поддержанными.
Header maps проверять как header maps, не как node-option keys; имена `dns`/`plugin`
в headers сами по себе не являются host controls. Согласовать transport с матрицей T01.

Если полная ветка протокола пока не реализована, возвращать unsupported, явно указать
разницу static capability и implemented parser subset; не объявлять T04 завершённым
до review оставшихся веток и manual/URI entrypoints. Не реализовывать real HTTP adapter
как скрытый side effect parser. UA negotiation остаётся injected и bounded; concrete
fetch adapter и persistence fetch settings обсуждаются Sol в рамках этого же этапа.

## Review/evidence

Сначала native parser + synthetic fixtures; Sol review/compile. Затем URI/base64 и pipeline
в той же T04. Только после завершения T04 перейти T05 installation regression driver,
fresh CLI build/COSMIC compile и source-hash manifest. Prepared tests — NOT_RUN, никакого
PASS по fixtures до установки. Старые T03 manifest относятся к снимку до T04.
