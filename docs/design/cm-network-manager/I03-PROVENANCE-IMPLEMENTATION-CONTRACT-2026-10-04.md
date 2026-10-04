# I03.T03: private artifacts и durable provenance

Следует после завершённого ревью T02. T03 реализует сохранение результата, T04 — полный форматный parser, ручной сервер и входы независимых источников. Ни сеть, ни запуск core, ни runtime tests в текущем этапе не исполняются.

## Граница и происхождение

Accepted subscription берёт actual UA, исходные bytes, SHA256 и время из `Negotiated<T>`, а не из preferred UA или часов после сохранения. Typed definition input передаётся доверенным classifier/parser; T03 проверяет структурные bounds, протокол/transport против pinned capabilities и согласованность artifact. Это ещё не готовый parser произвольного URI/YAML. T04 обязан отклонять неизвестные/restricted поля до construction и publication.

Source получает optional strict provenance metadata (`serde(default)` для прежних неопубликованных schema1 fixtures): формат, время принятия, SHA256 raw body, core version и full upstream commit, origin negotiated/local и optional SHA256 actual UA. Никаких raw UA, URL или credential bytes в graph/status DTO. Metadata digest совпадает с Source.content_digest_sha256; negative time, некорректный digest/pin/origin дают safe typed отказ. Metadata не может исчезать при переходе уже имеющего provenance Source; время не регрессирует. Public views остаются whitelist.

Raw actual UA сохраняется только в private immutable artifact blob вместе с raw body, полными разрешёнными node definitions и влияющими global defaults. `Source.credential` указывает на этот artifact; Node.credential_refs удерживает artifact своего поколения. Сериализуемый private artifact ограничен до передачи Store; Debug и ошибки скрывают payload, UA, endpoint и parser text. Explicit private getter для будущего core adapter возвращает полное определение, без сокращённой реконструкции из model Node.

Artifact хранит version, формат и origin каждого разрешённого определения/default. Node digest вычисляется из deterministic полного определения вместе с влияющими defaults: смена TLS/REALITY/transport/разрешённых defaults не может выглядеть тем же определением. Имя не используется как stable ID. Duplicate definitions допустимы только при явном различении позиций/IDs; silent collapse запрещён. Bounded node count/serialized bytes/depth проверяются до durable writes. Full format/protocol field validation завершается в T04; T03 не называет произвольный JSON проверенным core config.

Constrained `log-level` допускает `silent/error/warning/info/debug`, как в [pinned upstream config example](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/docs/config.yaml). Не заменять `warning` на Rust-style `warn`. Byte limit проверяется во время кодирования artifact, а не после неограниченного `to_vec`.

## Publication

Новый Source создаётся с generation1, новыми entropy IDs и typed issued-ID registry. Update использует ожидаемые graph revision и source generation, `Store.update_source`, immutable Nodes и прежние Session pins. Parsing/classification/validation errors не публикуют новую revision и не запускают внешние команды. Не трогать legacy singleton CLI/config.

Changed raw body digest означает generation+1 и свежие Node IDs для всех текущих definitions; прежние Nodes/artifacts сохраняются для pins. Исчезнувший definition вызывает только уже реализованную NET invalidation, не перенос сессии на другой узел. Другие Sources остаются прежними.

Same raw body и same accepted UA переиспользуют private artifact и прежние Node IDs, обновляя лишь provenance metadata/time и graph revision. Same body при другом actual UA сохраняет новый private artifact для Source.credential, не меняя generation/Node IDs/старые immutable Node refs. Artifact не включает меняющееся время принятия, иначе каждый refresh создавал бы лишний blob. Same body с другим normalized payload/defaults/format не принимается как незаметная правка определения: конфликт либо явное перепланирование требуется до publication.

Source removal использует существующий durable plan и ownership registry: устаревшие private artifacts того же Source не теряются, общие references не удаляются. Corrupt artifact или metadata mismatch дают отказ до публикации. DurabilityIndeterminate Store сохраняет своё значение; импорт не выдаёт success или ложный rollback.

## Подготовленные проверки

Actual winner второго UA после retry сохраняется и доступен только через private artifact; read/reopen сохраняет формат/time/pin/digest. Same-body refresh не меняет generation/Node IDs, а different-UA refresh переиспользует Nodes и сохраняет правильный новый header. Changed body создаёт generation+1/fresh Nodes и сохраняет старые Session pins; исчезновение меняет только NET. Протокол/defaults/credential payload сохраняются полностью. Bad artifact/negative time/unknown schema/stale revision не заменяют рабочий Source. Независимый Source остаётся byte-equivalent. Форматирование/public views не содержат synthetic secrets. Все fixtures — NOT_RUN до установки; compile-only evidence отдельно.

Уточнение T04 review 2026-10-05: global-client-fingerprint удалён в [pinned parseGeneral](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/config/config.go),
его прием в T03 typed defaults исправлен на UnsupportedFeature. Непроверенные fixtures
предыдущего снимка не создавали опубликованных artifact файлов; старое compile evidence
сохраняется как история, новые исходники требуют нового evidence.
