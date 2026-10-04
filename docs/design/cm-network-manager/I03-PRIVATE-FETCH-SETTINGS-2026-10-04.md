# I03.T04: закрытые настройки загрузки

`Negotiated` фиксирует исходный явно заданный endpoint, упорядоченный deduplicated набор UA
и проверенные лимиты **до** перестановки кандидатов по cache. Captured record нельзя
подменить аргументом после получения ответа. `SourceImportInput::from_negotiated` переносит
эту запись вместе с actual accepted UA в тот же immutable private blob Source. Метаданные
графа и status DTO не получают URL или список UA. `Debug` закрытой записи полностью redacted.

`SourceArtifact::fetch_settings` позволяет восстановить endpoint, список кандидатов, policy
и cache hint после открытия Store новым процессом. Actual UA берётся из этого же артефакта,
время — из текущей provenance Source. Cache по-прежнему проверяет endpoint, порядок UA,
pin ядра, TTL и время; это подсказка порядка запросов, а не доказательство нового ответа.
Числа duration сохраняются точно в наносекундах в пределах проверенных hard limits.

Изменение endpoint, списка UA или лимитов при том же теле/actual UA создаёт новый закрытый
Source blob. Generation и immutable Node ID/refs остаются прежними; Node digest зависит
от protocol/transport/definition/defaults, а не от fetch settings. Плохой CAS и нарушение
валидности по-прежнему отказывают до публикации. Local source не имеет fetch settings.
Для прежнего unpublished artifact без этого поля доступен `None`, а refresh может заполнить
его новым blob. Автоматическое угадывание URL, provider и converter отсутствует.

Подготовлены две регрессии: endpoint-only refresh/reopen/cache без изменения Node,
и фиксация начального порядка кандидатов при cache reorder с отказом повреждённым settings.
Они скомпилированы вместе с прежними provenance/artifact/negotiation целями, **NOT_RUN**.

Concrete HTTP adapter пока не подключён. Исходник locked ureq2.12.1 отмечает отсутствие
deadline у DNS lookup (`src/stream.rs`, TODO перед resolver). Один `Request::timeout` не
обеспечивает hard budget для этого этапа; injected fetch contract остаётся явным, адаптер
должен обеспечивать timeout и bounded чтение, а не объявлять их выполненными по одной опции.
Никаких сетевых запросов или запусков ядра/служб для этого этапа не было.
