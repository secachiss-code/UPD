# I02: модель источников, профилей и сессий

Дата исходного черновика: 2026-10-03, актуальное уточнение 2026-10-04. После завершения кодовой части I01.T05 пользователь разрешил продолжать кодовую очередь. Код I02.T01–T03 реализован Luna xhigh, Sol завершил ревью и исправления. Строгие types/validators в src/profiles/model.rs; 13 независимых regression tests подготовлены, compile-only проверка прошла. [Evidence](i02-evidence/model-types-2026-10-04/summary.json). Код I02.T04 storage реализован Luna xhigh и reviewed Sol: 12 store integration и6 fault unit regressions prepared; compile-only PASS. [Storage evidence](i02-evidence/store-review-2026-10-04/summary.json). T05 завершён:3 schema examples/error contract,35 checks prepared, CLI build/COSMIC compile PASS, runtime NOT_RUN; [кодовое завершение](I02-CODER-COMPLETION-2026-10-04.md). Семь открытых вопросов WAVE0 §2 уточнены в [implementation contract](I02-MODEL-IMPLEMENTATION-CONTRACT-2026-10-04.md), который имеет приоритет над историческим черновиком ниже. Хранение задано [storage contract](I02-STORAGE-IMPLEMENTATION-CONTRACT-2026-10-04.md). Runtime acceptance остаётся открытой: [S01–S13](I02-TEST-CONTRACT.md) — NOT_RUN до установки.

Принятые условия здесь не переоткрываются: A+C, произвольные приложения, независимые источники и свой сервер, для приложений только туннель, для хоста off/proxy/tunnel, личность браузера принадлежит профилю данных. Текущий код CM 0.2.8 остаётся singleton: список подписок с одним полем `active`, одна служба и один TUN. Новый src/profiles Store хранит эти записи через отдельный API; production CLI/UI пока использует legacy singleton до последующих этапов интеграции.

`schema_version` этого контракта — **1**. Typed records и snapshot несут этот номер; неизвестные версии и поля отклоняются. Schema1 ранее не публиковалась в production, автоматического импорта legacy singleton нет.

## T01. Сущности

Идентификатор любой сущности — непрозрачная строка, выданная один раз. Отображаемое имя, URL и имя узла идентификатором не являются. Повторное использование идентификатора запрещено.

| Сущность | Владелец записи | Обязательные ссылки | Что записи не принадлежит |
|---|---|---|---|
| Source | хранилище профилей | `kind`: subscription или manual_server; `generation`; `credential_ref` | cache DNS, сессия, личность браузера |
| Node | поколение Source | `source_id`, `source_generation`, digest полного определения | выбор «этот узел теперь другой» без нового поколения |
| ConnectionProfile | хранилище профилей | `source_id`, привязка узла или именованная политика выбора, вид ядра, DnsPolicy | runtime cache, маршруты хоста |
| TunnelInstance | runtime туннеля | `connection_profile_id`, собственный `generation` | HostPolicy, данные приложений, BrowserIdentityProfile |
| ApplicationDefinition | хранилище приложений | executable, argv как массив, cwd, `data_profile_id`, environment как список пар, назначение own-tunnel или group | имя продукта в коде, режим proxy |
| ApplicationGroup | хранилище приложений | список `application_id`, один `tunnel_instance_id`, `mutual_network_access` | общий HOME и общий browser identity |
| EnvironmentProfile | сессия | timezone, locale, languages | системные часы и timezone хоста |
| BrowserIdentityProfile | профиль данных приложения | `data_profile_id` | TunnelInstance и страна выхода |
| Session | runtime запуска | application, data profile, tunnel instance, `tunnel_generation`, `environment_profile_id` | право менять generation задним числом |
| Verification | ось наблюдения | ось NET, REGION, STATE или APP; значение; время evidence; generation | вывод о другой оси |
| HostPolicy | отдельная запись хоста | `mode`: off, proxy или tunnel | список сессий приложений |

`credential_ref` — непрозрачная ссылка. Материал секрета в публичный статус Source, Node, Session и Verification не входит.

DnsPolicy — поле ConnectionProfile. Её значения на этом рубеже: `ipv6` = pass или block, `fake_ip` = allowed или forbidden. Выбор конкретного resolver этот документ не делает: это вопрос I10, который не начат. DnsRuntime, cache и пул fake-IP принадлежат поколению TunnelInstance и не переживают смену этого поколения как валидные данные.

`mutual_network_access` по умолчанию false. Общий туннель группы сам по себе не открывает доступ между приложениями.

Verification хранит четыре оси независимо. Начальное значение каждой — `unknown`. Допустимые значения: `unknown`, `partial`, `verified`, `blocked`, `error`. `verified` у NET не заполняет REGION, STATE или APP.

## T02. Политики хоста и приложений

HostPolicy и политики приложений — разные записи.

Хост может быть off, proxy или tunnel. Proxy хоста означает только настроенный хостовый proxy и не означает, что вся сеть хоста им охвачена. Для tunnel и proxy у хоста есть собственный `connection_profile_id`. У режима off этой ссылки нет.

Приложение назначается только в туннель. Режима proxy и режима off у приложения нет. Назначение — либо собственный TunnelInstance, либо ApplicationGroup с одним общим TunnelInstance. Источник приложения задаётся его ConnectionProfile и может отличаться от источника хоста и от других групп.

`autostart` хранится отдельно у HostPolicy, у ApplicationDefinition и у ApplicationGroup. Включение автостарта хоста не включает приложения. Остановка хоста переводит HostPolicy в off и останавливает только туннель или proxy хоста. Сессии приложений при этом продолжаются. Остановка всех туннелей — отдельная явная операция, её нет у команды остановки хоста.

Завершение одной сессии группы не завершает остальные сессии и не останавливает хост. Точный refcount и момент остановки общего worker — рубеж I12. Для этой модели достаточно правила владения: время жизни группы не привязано к HostPolicy.

### Пример 1. Две подписки и три приложения

- Source `sub-a` и Source `sub-b`, разные `credential_ref` и разные поколения.
- Application `app-1` назначено на собственный TunnelInstance профиля, который ссылается на `sub-a`.
- Application `app-2` и `app-3` входят в одну ApplicationGroup. Группа ссылается на другой TunnelInstance профиля `sub-b`.
- У `app-2` и `app-3` разные `data_profile_id`, разные EnvironmentProfile и разные BrowserIdentityProfile.
- `mutual_network_access` группы false.
- HostPolicy = off. Все три приложения остаются запущенными: `app-1` в собственном туннеле, `app-2` и `app-3` в одном общем.

Имён продуктов в этих записях нет. `app-1` задаётся executable и массивом argv.

### Пример 2. Хост и группа на разных источниках

- HostPolicy = tunnel, профиль хоста ссылается на `sub-a`.
- Группа из примера 1 по-прежнему ссылается на `sub-b`.
- Остановка хоста оставляет группу и её сессии. Поколение туннеля группы не меняется из-за остановки хоста.
- Исчезновение узла в `sub-a` оставляет туннелю хоста прежнюю generation и ставит его Verification NET в `blocked` или `error`. Группа на `sub-b` не переназначается.

## T03. Идентификаторы, схема и поколения

Каждая сохранённая запись содержит `schema_version`. Запись другой версии будущий код не обязан читать как версию 1: это условие будущей миграции схемы, а не реализованный путь.

Generation Source — целое число. Оно увеличивается на единицу, когда принятое содержимое источника меняет digest. Node ссылается на пару (`source_id`, `source_generation`). Узел, исчезнувший в новом поколении, не заменяется другой записью с тем же Node id.

Generation TunnelInstance увеличивается, когда применённая конфигурация этого туннеля меняется, включая смену поколения DNS cache и fake-IP. Старый cache к новому поколению не относится.

Session при создании записывает `tunnel_instance_id` и `tunnel_generation`. Пока сессия жива, смена выбранного узла в ConnectionProfile не переписывает эту пару. Новое соединение после явного restart или reconnect получает уже новую generation. Бесшовный перенос открытого потока эта модель не обещает: это ограничение I12, здесь оно только не скрывается.

BrowserIdentityProfile ссылается на `data_profile_id`. Смена TunnelInstance, страны выхода или HostPolicy его не меняет. Пока хотя бы один процесс этого профиля данных жив, записанный набор остаётся тем же. Числа «28 дней» и «пять записей истории» в контракт версии 1 не входят. Они остаются гипотезой Q27 до I15-R, который этим рубежом не начат.

Живая Session, чей узел исчез в новом поколении Source, сохраняет прежнюю generation и получает Verification NET = `blocked` или `error` с временем evidence. Она не получает новый случайный узел.

Удаление Source или Node, на который ссылается ConnectionProfile, группа, хост или живая Session, будущим хранилищем отклоняется. Снятие ссылок и удаление — разные шаги. Это требование к I02.T04, проверенное контрактом, а не поведение текущего кода.

## Что не сделано

Atomic store, отдельные immutable blobs, CAS, исчезновение digest узла и staged removal реализованы. Restart/failure fixtures подготовлены, runtime не исполнен. Их ожидаемое поведение перечислено в [контракте проверок](I02-TEST-CONTRACT.md). Проверки до установки остаются NOT_RUN. После ревью и завершения кодовой части I02.T04/T05 последовательная кодовая очередь может продолжаться; runtime acceptance и эксплуатация не объявляются завершёнными.

## Проходы проверки I02

1. Список сущностей сверен с I02.T01. В первом черновике у ApplicationDefinition не было environment. Поле добавлено как список пар, без интерпретации содержимого. Формулировка примера 1 больше не называет общий туннель трёх приложений тремя туннелями.
2. Host stop не входит в жизненный цикл сессий. Session хранит generation. BrowserIdentityProfile ссылается на профиль данных и не ссылается на туннель. Числа Q27 в `schema_version` 1 не входят.
3. Примеры не содержат секретов, URL и имён продуктов. Текст не говорит, что хранилище уже записано в коде. Контракт S01–S13 помечен NOT_RUN.
4. Критерии «две подписки и три приложения», «host stop не останавливает приложения» и «живая сессия хранит generation» закрываются только будущими S01, S10 и S07. Провал подмены узла и смеси поколений в контракте блокирует приёмку. На том же проходе ссылка сессии названа `environment_profile_id`, а текущий код описан как список подписок с одним полем `active`. Пятый проход не потребовался.
