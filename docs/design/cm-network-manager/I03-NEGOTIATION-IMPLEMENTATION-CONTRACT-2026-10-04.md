# I03.T02: контракт согласования User-Agent

Продолжает reviewed T01. Sol задаёт контракт, Luna xhigh выполняет рутинную реализацию, Sol проверяет результат. Порядок: T01 → T02 → T03; parser, persistence и реальный HTTP adapter не входят в текущую pure negotiation state machine. Runtime checks остаются NOT_RUN до установки.

## Входы и приватность

Endpoint берётся только из явной конфигурации. Входной и canonical URL ограничены8192 bytes; literal control characters отклоняются; HTTPS по умолчанию, HTTP только с явным opt-in. Fragments и другие schemes отклоняются. Не конструировать provider URL по имени сервиса, не обращаться к converter, не следовать redirects. Getter URL доступен только доверенному fetch adapter; Debug endpoint не показывает адрес, userinfo, query или path.

User-Agent — непустая printable ASCII строка до512 bytes, без управляющих символов. Debug скрывает значение. Actual accepted UA доступен явным getter для последующей private provenance; configured preferred UA не заменяет фактически успешный вариант.

Fetch response и accepted result скрывают bytes и generic parsed value в Debug. Typed errors содержат только code, безопасный HTTP status и счётчики; ни URL/body/header/сырой transport error в них не копируются.

## Бюджеты

| Параметр | Default | Максимум |
|---|---|---|
| Requests | 6 | 8 |
| Total budget | 30s | 60s |
| Per request | 10s | min(10s, total budget) |
| Body | 8MiB | 32MiB |
| Winner TTL | 24h | 7days |

Нулевые/некорректные значения и неверные UA отклоняются до fetch. Отсчёт total budget монотонный, общий для попыток. Remaining timeout каждого запроса — минимум оставшегося общего времени и per-request ceiling. Бюджет проверяется перед и после fetch/classify, а не обновляется заново при смене UA. Поздний successful fetch, превысивший переданный per-request timeout, не принимается даже при оставшемся total budget. Response size проверяется до classifier; HTTP adapter позднее обязан ограничивать чтение до allocation полного response.

Clock seam позволяет подготовить deterministic fixtures без сна и сети. Fetch/classify callbacks доверенные: fetch обязан соблюдать переданный timeout; pure state machine не обещает принудительно прервать произвольный блокирующий callback. Реальные deadline/DNS/body-read ограничения adapter проверяются отдельно при установке.

## Порядок попыток и cache

Configured preferred + configured candidates сохраняют заданный порядок и дедуплицируются. Cached winner ставится первым только при совпадении endpoint digest, ordered agent-set digest, core pin и TTL; winner обязан оставаться в текущем разрешённом наборе. Cache из другого endpoint, после изменения набора UA, из будущего времени или после TTL игнорируется. Digest URL не превращается в публичный адрес.

Попытки ограничены как количеством кандидатов, так и max requests/total budget. Повтор разрешён только после успешного HTTP response с непригодным body. Classifier отдельно обозначает retryable body rejection и terminal rejection. Не повторять transport failure, timeout, HTTP auth, quota, unavailable endpoint или redirect под следующим UA. Не использовать частично разобранный response как принятый Source.

401/403 означают terminal auth failure;429 — quota/rate-limit;404/410 — unavailable;3xx — redirect refusal; остальные HTTP failures дают typed terminal status. HTTP failures не показывают содержимое ответа. Oversize не вызывает classifier и не запускает следующий UA.

Unix epoch берётся один раз и привязывается к monotonic sample сразу после его чтения. Время самого чтения Unix clock входит в total budget, но не прибавляется второй раз к accepted timestamp. Отрицательное время и регрессия любого monotonic sample дают отказ.

Accepted result хранит private response bytes и parsed value, фактически принятый UA, SHA256 исходного body, время принятия, request count и bound winner cache. Запись в Store/provenance выполняется следующим этапом; никакой части текущего Source эта функция сама не меняет.

## Подготовленные проверки

HTML/placeholder → valid body с actual winner; cache reuse/invalidation по endpoint/UA/core/time; auth/quota/transport без дополнительных запросов; request/total/per-request ceilings; oversized response до classifier; invalid policy до fetch; redacted formatting. Fixtures синтетические, callback не обращается к сети. PASS возможен только после фактического исполнения при установке; compile-only сохраняет runtime NOT_RUN.
