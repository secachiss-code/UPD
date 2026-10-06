# Черновик ADR: YAML-стек (H.05 / D5)

Статус: **D5 записан пользователем 2026-10-06.** Для пути импорта выбран `saphyr` 0.1.0 со своим guard на `Parser::next_event`. Миграцию в этой сессии не делал: продуктовый код пишет Cursor. `src/vpn.rs` остаётся на `serde_yaml` до H.06.

Дата: 2026-10-06. Исполнитель: Grok. Продуктовый код не менялся. Сборка musl не запускалась. Fixtures на чужих крейтах не прогонялись: в `Cargo.toml` их нет.

Границы, которые задал пользователь на итерации 1:

- Последствия считаются для пути импорта (`yaml_guard` + `parse_yaml`). `src/vpn.rs` остаётся на `serde_yaml` до H.06.
- В сравнении пять строк: оставить пин, `serde_yaml_ng`, `serde_norway`, `yaml_serde`, `saphyr` со своим guard.

Проверенный locked-артефакт: `serde_yaml` `0.9.34+deprecated`, checksum `6a8b1a1a2ebf674015cc02edccce75287f1a0130d394307b36743c2f5d504b47`; прямой пин `unsafe-libyaml` `0.2.11`, checksum `673aac59facbab8a9007c7f6108d11f63b603f7cabff99fabf650fea5c32b861`. Оба checksum совпадают с `Cargo.lock`. Чужие крейты читались как опубликованные `.crate` с crates.io, не как git HEAD.

Локальный `rustc` в момент сверки: 1.98.1. Это выше заявленных MSRV кандидатов. Целевой musl этим фактом не доказан.

## Записанное решение D5

Пользователь выбрал строку `saphyr`. Это решение, не мнение рецензента.

Для Cursor, когда миграция начнётся:

- Путь импорта читает события `saphyr_parser::Parser::next_event`. Высокоуровневый `Yaml::load_from_str` не использовать: он клонирует якоря и не превращает дубль ключа в ошибку.
- Guard на этих событиях сохраняет нынешние отказы: alias, ненулевой anchor, tag, второй документ, ключ `<<`, глубина 64, 1_000_000 значений, тело 8 МиБ и не-UTF-8.
- Дубль ключа по-прежнему ловит `StrictValueSeed`, без имени ключа и без значения в тексте ошибки.
- Четыре теста guard переписываются на события `saphyr-parser`. Три YAML-теста в `tests/audit_i03_native.rs` остаются на тех же `ParserError`. Шесть JSON-тестов не трогать.
- Прямой `unsafe-libyaml` с пути импорта уходит. Транзитивный остаётся, пока `vpn.rs` на `serde_yaml`.
- Feature `encoding` у `saphyr` для этого пути не включать.

## Вопрос пользователю

Закрыт выбором выше. Текст вопроса, на который дан ответ: какой стек остаётся на пути импорта. `src/vpn.rs` при этом выборе продолжает тянуть `serde_yaml` до H.06.

## Что путь импорта гарантирует сейчас

Свойства ниже живут в нашем коде.

`parse_native` для `MihomoYaml` сначала вызывает `yaml_guard::validate`, и только потом `serde_yaml::Deserializer::from_str` (`src/sources/parser/native.rs`).

Guard (`src/sources/parser/yaml_guard.rs`) читает события `unsafe_libyaml` по одному:

- тело длиннее `MAX_RAW_SOURCE_BYTES` (8 МиБ) и не-UTF-8 — `Malformed`, до парсера;
- один документ, без `%YAML` и без tag directives;
- alias, anchor, tag и ключ `<<` — `UnsupportedYaml`;
- глубина больше 64 — `TooDeep`;
- больше 1_000_000 значений — `TooManyValues`.

Дубли ключей guard не смотрит. Их ловит `StrictValueSeed::visit_map`: второй ключ даёт `ParserError::DuplicateKey` (`native.rs`). Тот же потолок глубины и числа значений повторен в seed (`MAX_NATIVE_DEPTH`, `MAX_NATIVE_ENTRIES`).

В `yaml_guard.rs` 11 строк со словом `unsafe`. Отдельной C-библиотеки в зависимостях нет: crates.io описывает `unsafe-libyaml` как «libyaml transpiled to rust by c2rust». В самом крейте 0.2.11 таких строк 240. В обёртке `serde_yaml` 0.9.34 — 59.

## Как устроен загрузчик serde_yaml

Это общее для пина и для трёх форков: их `loader.rs` / `de.rs` — тот же алгоритм. У `serde_yaml_ng` 0.10.0 `loader.rs` совпадает с locked 0.9.34 побайтно (`diff -q`). У `serde_norway` 0.9.42 и `yaml_serde` 0.10.7 отличия в этом файле — стиль и `alloc::` вместо `std::`, не другая обработка alias.

Опубликованный `serde_yaml` 0.9.34:

- `Loader::next_document` складывает все события документа в `Vec`, и только потом serde видит первое значение (`src/loader.rs`, цикл до `DocumentEnd`). Потокового потолка размера в крейте нет.
- Alias записывается и потом раскрывается: `DeserializerFromEvents::jump` повторяет события якоря. Счётчик прыжков обрывается, когда превышает `events.len() * 100` (`src/de.rs`, `jump`). Раскрытие alias в крейте есть. Его нет на нашем пути импорта только потому, что guard отвергает alias раньше вызова `Deserializer`.
- `remaining_depth` стартует со 128 (`src/de.rs`, оба места сборки десериализатора). Это выше нашего 64. Guard срабатывает раньше.
- `MapAccess::next_key_seed` отдаёт каждый ключ, включая повтор (`src/de.rs`). Схлопывания дубля нет. Поэтому текущий `StrictValueSeed` видит второй ключ.
- Отказ с текстом `duplicate entry` и самим ключом стоит в `Deserialize` для `Mapping` (`src/mapping.rs`). Наш seed этим путём не идёт и ключ в ошибку не кладёт.

Итог для любого форка с тем же загрузчиком: guard остаётся нужен. Форк сам по себе не даёт потоковый лимит, не запрещает alias и не заменяет наш отказ на дубле.

## Кандидаты

Даты и лицензии — crates.io. «Строк `unsafe`» — число строк с этим словом в опубликованном `.crate`, не число блоков.

| Вариант | Версия и дата | Лицензия | Движок | Строк `unsafe` | Поток до дерева | Alias | Дубль ключа в `MapAccess` |
|---|---|---|---|---|---|---|---|
| Пин | `serde_yaml` 0.9.34+deprecated, 2024-03-25; `unsafe-libyaml` 0.2.11, 2024-03-17 | MIT OR Apache-2.0 и MIT | тот же `unsafe-libyaml`, что у guard | 59 + 240 + наши 11 | guard уже режет события по одному | guard отвергает до `Deserializer`; крейт раскрыл бы | отдаёт оба ключа |
| `serde_yaml_ng` | 0.10.0, 2024-05-26 | MIT (у 0.9.36 было MIT OR Apache-2.0) | `unsafe-libyaml` `^0.2.11` | 59 в обёртке, движок тот же | нет, тот же `Vec` событий | тот же `jump` | тот же `next_key_seed` |
| `serde_norway` | 0.9.42, 2024-12-21 | MIT OR Apache-2.0 | `unsafe-libyaml-norway` `^0.2.13` (новейший 0.2.15, 2024-12-21, 240 строк `unsafe`, MIT, снова c2rust) | 59 + 240 | нет | тот же `jump` | тот же |
| `yaml_serde` | 0.10.7, 2026-08-18, публикатор `ingydotnet`, репозиторий `github.com/yaml/yaml-serde` | MIT OR Apache-2.0 | `libyaml-rs` 0.3.0, 2026-03-11, одна версия, 238 строк `unsafe`, MIT, снова c2rust, `github.com/yaml/libyaml-rs` | 58 + 238 | нет; `loader.rs` отличается `alloc::` | тот же `jump` | тот же; отказ `Mapping` на строке 870 |
| `saphyr` | 0.1.0, 2026-09-19, MSRV 1.85, edition 2024 | MIT OR Apache-2.0 | `saphyr-parser` 0.1.0, без `unsafe-libyaml` | 0 в `saphyr` и 0 в `saphyr-parser` | `Parser::next_event` отдаёт одно событие (`saphyr-parser` `src/parser.rs`) | событие `Event::Alias` есть; `YamlLoader` клонирует якорь (`saphyr` `src/loader.rs`) | у `saphyr` нет `serde::Deserializer` |

Репозитории форков: [serde-yaml-ng](https://github.com/acatton/serde-yaml-ng), [serde-yaml у cafkafk](https://github.com/cafkafk/serde-yaml), [yaml-serde](https://github.com/yaml/yaml-serde), [saphyr](https://github.com/saphyr-rs/saphyr). Архив исходного крейта: [заметка релиза 0.9.34](https://github.com/dtolnay/serde-yaml/releases/tag/0.9.34) — дальнейших версий автор не планирует, замена не назначена.

`serde_saphyr` в таблицу не входил. Это другой крейт, не `saphyr` 0.1.0, и в границе сравнения его не было.

## Последствия для пути импорта

Пин. Код импорта не меняется. Четыре теста guard и девять native остаются как есть. В бинарнике один YAML-крейт на импорт и на `vpn.rs`. Сопровождение обоих крейтов остановилось в марте 2024. `unsafe-libyaml` 0.2.11 — новейшая версия, не удержанный старый пин. Прямая зависимость `=0.2.11` совпадает с требованием `serde_yaml` `^0.2.11`.

`serde_yaml_ng`. Движок тот же, cargo сведёт его с пином guard в один `unsafe-libyaml` 0.2.11. Guard можно оставить. Меняется тип в `parse_yaml`: `serde_yaml_ng::Deserializer` вместо `serde_yaml::Deserializer`. `vpn.rs` по-прежнему на `serde_yaml`, так что в бинарнике две serde-обёртки и один движок до H.06. Последняя публикация — май 2024. Лицензия 0.10.0 сужена до MIT; у проекта лицензия MIT, этого хватает. Четыре guard-теста не переписываются. Три YAML-теста native ожидаемо живы, потому что `MapAccess` и `jump` те же; прогона нет.

`serde_norway`. Алгоритм загрузчика тот же, тип движка другой: `unsafe-libyaml-norway`. Guard написан на типы `unsafe_libyaml`. Пока guard не переведён на norway, импорт тянет оба c2rust-крейта, а `vpn.rs` добавляет ещё и `serde_yaml` с `unsafe-libyaml` 0.2.11. Последняя публикация norway — декабрь 2024. Fixtures guard без переноса FFI не соберутся на новом типе парсера. Три YAML-теста native ожидают тот же `Deserializer::from_str`; сам вызов надо переименовать.

`yaml_serde`. Та же развилка, что у norway: загрузчик прежний, движок — `libyaml-rs`, не `unsafe-libyaml`. Оставить guard как есть значит держать два c2rust-дерева плюс `serde_yaml` для `vpn.rs`. Это единственный форк с публикацией в 2026 (август) и с MSRV 1.82. В крейте есть `no_std` через feature `std`. На критерий импорта это не влияет: путь импорта уже на std. Четыре guard-теста остаются привязаны к нынешнему FFI, пока guard не переведён на `libyaml-rs`.

`saphyr`. Своего `serde::Deserializer` нет. `StrictValueSeed` на него не садится. Путь импорта — обход `Parser::next_event` и свой guard на `Event`: `Alias`, ненулевой anchor id, `Some(tag)`, второй `DocumentStart`, скаляр `<<` в позиции ключа, глубина, число значений. `Yaml::load_from_str` для этого не подходит: `YamlLoader` клонирует якорь в дерево и вызывает `hash.insert`, не глядя на прежнее значение (`src/loader.rs`). Дубль ключа там не становится ошибкой. В сканере явный потолок — `flow_level: u8` (`saphyr-parser` `src/scanner.rs`); на переполнении текст `recursion limit exceeded`. Отдельного предела 64 на блочную вложенность там нет, свой guard всё равно нужен. Четыре теста guard переписываются на события `saphyr-parser`. Три YAML-теста native можно сохранить, если новые ошибки мапятся в те же `ParserError`. Шесть JSON-тестов движок не трогает. `unsafe` на пути импорта (11 строк guard и прямой `unsafe-libyaml`) уходит. Транзитивный `unsafe-libyaml` остаётся, пока `vpn.rs` сидит на `serde_yaml`. Feature `encoding` по умолчанию тянет `encoding_rs`; для этого пути его можно не включать. MSRV 1.85 локальный 1.98.1 покрывает.

Сборка musl для всех пяти строк в этой итерации не запускалась. В манифестах проверенных крейтов нет `cc` и системной libyaml: движки либо c2rust-Rust, либо чистый Rust `saphyr`. Это не заменяет прогон.

## Корпус

Четыре теста guard в `src/sources/parser/yaml_guard.rs`: `syntax_annotations_refused_but_secret_text_is_data`, `streaming_depth_limit_before_tree_loading`, `streaming_entry_limit_before_tree_loading`, `malformed_and_complex_keys_fail_without_parser_text`.

Девять тестов `tests/audit_i03_native.rs`. YAML через `ImportFormat::MihomoYaml` идёт в трёх: дубль ключа, кавычки с маркерами `&` / `*` / `!` / `<<`, отказ alias, tag, merge и двух документов. Остальные шесть кормят JSON и общий разбор узла.

## Что означает каждая строка как работа

Общее для всех строк, кроме пина: `src/vpn.rs` до H.06 остаётся на `serde_yaml`. Закрытие `I03.T04.x` ждёт записанный D5, а не зелёную миграцию.

Пин. Работы в коде нет. Guard, `parse_yaml` и все 13 тестов корпуса остаются. В бинарнике один YAML-крейт. Риск, который остаётся: оба крейта без релизов с марта 2024, и 11 строк `unsafe` в guard плюс c2rust-движок никуда не деваются. Новых свойств безопасности эта строка не добавляет и не отнимает: alias, глубина и размер уже режет guard.

`serde_yaml_ng`. Работа — заменить тип десериализатора в `parse_yaml`. Guard и четыре его теста не трогать: движок тот же, cargo сведёт его в один `unsafe-libyaml` 0.2.11. До H.06 в бинарнике две serde-обёртки. Свойства alias, лимита и дубля ключа не становятся свойствами крейта, они остаются нашими. Публикаций с мая 2024 нет. Лицензия этой версии — только MIT.

`serde_norway`. Работа больше, чем смена имени. Загрузчик тот же, а типы парсера другие (`unsafe-libyaml-norway`). Либо guard переписывается на эти типы, либо рядом с ним остаётся старый `unsafe-libyaml` ради нынешнего FFI. Второе даёт два c2rust-движка плюс `serde_yaml` для `vpn.rs`. Четыре guard-теста без переноса FFI на новом типе не соберутся. Последняя публикация — декабрь 2024. Поведение дубля и alias для seed то же, что у пина, если guard по-прежнему стоит впереди.

`yaml_serde`. Та же развилка, что у norway: алгоритм загрузчика прежний, движок другой (`libyaml-rs`). Работа — либо перевести guard на `libyaml-rs`, либо жить с двумя c2rust-деревьями. Это единственная serde-обёртка с публикацией в 2026 году. Свойства безопасности пути импорта опять остаются в нашем guard, не в крейте.

`saphyr`. Работа — переписать YAML-половину `parse_yaml` и четыре guard-теста. `serde::Deserializer` у крейта нет, `StrictValueSeed` на него не сажается. Guard смотрит события `Parser::next_event` и сам отвергает alias, anchor, tag, второй документ, ключ `<<`, глубину и число значений. Высокоуровневый `Yaml::load_from_str` для этого пути не годится: он клонирует якоря и не превращает дубль ключа в ошибку. Шесть JSON-тестов не трогать. С пути импорта уходят наши 11 строк `unsafe` и прямая зависимость `unsafe-libyaml`. Сам движок c2rust из бинарника не исчезает, пока `vpn.rs` на `serde_yaml`.

## Мнение рецензента, не решение

Свойства, ради которых выбирают стек (нет alias, лимит до дерева, дубль ключа без эха значения), уже записаны в guard и в `StrictValueSeed`. Три serde-форка этот каркас не заменяют: они по-прежнему копят события документа и раскрывают alias через `jump`. `serde_yaml_ng` не добавляет второго движка, и с мая 2024 новых версий нет. `serde_norway` и `yaml_serde` при живом нынешнем guard добавляют второй c2rust-крейт; у `yaml_serde` при этом есть публикация 2026 года. `saphyr` — единственная строка, где парсер без `unsafe` и события приходят по одному, и это переписывание `parse_yaml` и четырёх guard-тестов, а не смена имени крейта. Пока `vpn.rs` на `serde_yaml`, c2rust-движок из бинарника целиком не исчезает ни в одной строке, кроме пина, где он и так один.
