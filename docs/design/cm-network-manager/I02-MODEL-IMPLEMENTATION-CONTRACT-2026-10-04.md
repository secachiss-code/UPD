# I02.T01–T03: уточнённый контракт перед кодом хранения

Sol уточнил семь вопросов WAVE0 §2 после завершения кодовой части I01.T05. Это продолжение разрешённой последовательной кодовой работы; runtime acceptance I01 не подменяется. Схема 1 ранее не записывалась в production, поэтому уточнение черновика не является миграцией существующей schema.

## Идентичность и поколения

ID непрозрачен, непустой, максимум128 ASCII alnum/underscore/hyphen, не путь и не URL. Один общий issued_ids registry сохраняется навсегда: удалённые ID нельзя выдавать снова, включая смену типа сущности. Примеры ID в тестах синтетические; runtime ID получают entropy, не display name. Каждый entity record и graph snapshot несёт schema_version=1; неизвестная версия/поля дают отказ.

Node immutable: ID принадлежит одной паре source_id/source_generation и digest полного определения. В новом Source generation создаются новые Node IDs, даже для неизменного digest. Старые Nodes сохраняются как archive при наличии pins; тот же ID нельзя заменить новой definition или переписать generation. Source.current_node_ids перечисляет только его текущее поколение. ConnectionProfile может явно pin старый retained Node, выбор policy — именованный, без случайной подмены. Session pin хранит node_id, source_generation, tunnel_instance_id и tunnel_generation; активные pins неизменяемы.

Verification.generation — поколение TunnelInstance, вместе с tunnel_instance_id. Четыре оси NET/REGION/STATE/APP независимы, default unknown. При исчезновении использованного определения Source update сохраняет Session pins и ставит только NET blocked с evidence time; остальные оси не становятся verified.

## Host, apps, группы и окружение

HostPolicy mode off требует пустые connection_profile_id и tunnel_instance_id. Для proxy/tunnel обе ссылки обязательны; TunnelInstance owner=Host. App tunnel owner — Application(app_id) или Group(group_id), с owner references. stop_host очищает только HostPolicy и помечает его прежний instance stopped в модели; app Sessions/pins не меняет. Это запись модели, не управление настоящим worker.

ApplicationDefinition.assignment — единственный авторитетный источник членства: OwnTunnel(tunnel_id) либо Group(group_id). ApplicationGroup хранит tunnel_instance_id и mutual_network_access=false по умолчанию; application_ids выводятся из assignments, не хранятся второй несогласованной копией. Group worker owner соответствует группе. Приложение не имеет proxy/off. Autostart независим у HostPolicy, application и group.

До запуска ApplicationDefinition ссылается на EnvironmentPreset (timezone/locale/languages). EnvironmentProfile — отдельный snapshot для конкретной Session с preset_id и session_id; active Session ссылается на свой snapshot, последующая правка preset не меняет текущую Session. BrowserIdentityProfile принадлежит data_profile_id и не меняется из-за host/tunnel/country. Никаких browser fingerprint controls или значений Q27 этот этап не реализует.

## Ссылки и удаление

Удаление Source запрещено при ссылке ConnectionProfile, HostPolicy через profile, group через tunnel/profile или живой Session. Историческая ended Session хранит прежние IDs/generations, но не блокирует удаление; её pins не используются как live references. Tombstone registry не удаляется.

Global immutable CredentialMetadata registry содержит digest/size и owner_source_id происхождения. При выдаче нового credential ref его owner Source существует; после удаления origin-owner общая ссылка может оставаться у другого live Source/Node, а owner ID сохраняется как tombstone. План удаления не включает такие используемые ссылки. Метаданные устаревших поколений того же Source сохраняют ownership, чтобы последующее staged removal не оставило их без плана.

Credential material живёт только в private blobs, никогда в сериализуемых публичных Source/Node/Session/Verification. Graph хранит opaque credential_refs и metadata digest/size. Старые blobs сохраняются пока используются retained Node pins. Публичные views выбирают metadata явно, а не Serialize private graph и последующее regex удаление secrets.

Staged removal: durable removal record с plan_id и source_id сначала снимает разрешённые source/node links, затем удаляет больше не используемые credential blobs, затем фиксирует completion. Pending stage читается явно, повтор того же plan_id не создаёт второй blob/Source, complete plan сохраняет tombstone. Ошибка удаления оставляет pending stage, не ложный completed.

## Конкурентность и publication

CAS по graph revision и ожидаемой Source generation: при двух writers от одного snapshot один целый commit с revision+1, второй Conflict(expected,current); смесь поколений запрещена. Busy/IO/failure имеют отдельные коды. Source generation увеличивается на1 при смене содержимого; no-op не поднимает generation. Node и credential_refs публикуются одним graph snapshot. Новые private blobs durable до atomic snapshot publication; fail/crash до publication сохраняет старый graph и не меняет использованные blobs. Неопубликованные blobs остаются private orphan, не читаются через public status и не объявляются успешной generation. Recover/orphan cleanup не удаляет неизвестный пользовательский файл по маске.

Хранилище schema1 создаётся отдельно от legacy singleton config; автоматического import/конвертации working VPN и CLI default перехода нет. Module API на I02, live controller integration позже. Отложенные tests S01–S13 и installation/runtime checks не получают PASS от compile.
