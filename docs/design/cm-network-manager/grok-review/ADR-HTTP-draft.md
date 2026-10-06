# Черновик ADR: HTTP-транспорт с общим deadline (H.04 / D4)

Статус: материал к решению **D4**. Это не решение. Реализацию `src/sources/transport.rs` не начинать, пока пользователь не запишет D4.

Дата: 2026-10-06. Исполнитель: Grok, ревью по первичным источникам. Продуктовый код не менялся.

Проверенный артефакт locked-клиента: crates.io `ureq-2.12.1.crate`, sha256 `02d1a66277ed75f640d608235660df48c8e3c19f3b4edb6a263315626cc3c01d` — совпадает с `Cargo.lock`. Git исходника этой публикации: `ba4555d54fd72e16fc737f7e8fcc812efb131260`. Рядом сверены опубликованные `ureq 3.4.2` (`2e9ef24a80e1e7ecd0e6604f98ba49aceb7ec322`) и `minreq 3.0.0` (`eb528443b54d1f38ff9b089bae2832f0753f7fe7`). `reqwest` построчно не открывался и в выбор не входит.

## Вопрос пользователю

Какой клиент делать `FetchTransport` для `sources::negotiation`: обёртку над уже locked `ureq 2.12.1` с резолвом в отдельном потоке и одним deadline на остальные фазы, или переход нового пути на `ureq 3.4.2` с `timeout_global`, пока `src/vpn.rs` остаётся на ureq 2? `minreq` и собственный HTTP-клиент критерий общего deadline в том виде, в каком его требует `RequestSpec`, не закрывают.

## Что транспорт обязан гарантировать

Контракт уже записан в `src/sources/negotiation.rs`, не в клиенте:

- `RequestSpec::request_timeout` — оставшийся budget этого запроса (`Duration`, может быть меньше секунды). Callback обязан его соблюсти. `negotiate` не может прервать зависший callback (`negotiation.rs`, комментарий у `negotiate`).
- `RequestSpec::max_redirects` всегда `0`. Ответ 3xx возвращается наверх; `300..=399` становится `NegotiationError::RedirectRejected`.
- Тело больше `max_body_bytes` — ошибка. Значение по умолчанию 8 МиБ, потолок политики 32 МиБ.
- `ConfiguredEndpoint` разрешает `http` только при явном `allow_http`. Исходный `https` при этом не должен молча уехать на `http`.
- `FetchFailure` — закрытый класс (`Transport`, `Timeout`, `Tls`, `Redirect`). В `Display` ошибок переговоров нет URL, тела и заголовков.
- Legacy `src/vpn.rs` этот транспорт не подключает. Там остаётся свой `AgentBuilder`.

Общий deadline из H.04 — это DNS + TCP connect + TLS + чтение тела в пределах одного `request_timeout`. Лимит тела, запрет `https`→`http` и повторная проверка лимитов после redirect относятся к транспорту, даже если переговоры сами redirect не следуют: тот же транспорт позже нужен загрузчикам, которым лимит redirect больше нуля.

## Факт про ureq 2.12.1

Утверждение очереди верно: таймаут запроса не ограничивает DNS lookup.

В [`src/stream.rs`](https://github.com/algesten/ureq/blob/ba4555d54fd72e16fc737f7e8fcc812efb131260/src/stream.rs#L364) `connect_host` сначала вызывает резолвер и только потом смотрит deadline соединения:

```text
// TODO: Find a way to apply deadline to DNS lookup.
let sock_addrs = unit.resolver().resolve(&netloc)...
```

Резолвер по умолчанию — блокирующий `ToSocketAddrs::to_socket_addrs` ([`src/resolve.rs`](https://github.com/algesten/ureq/blob/ba4555d54fd72e16fc737f7e8fcc812efb131260/src/resolve.rs#L14)). `getaddrinfo` из этого вызова не прерывается. Документация `AgentBuilder::timeout` говорит то же самое: общий timeout «включая DNS», но медленный DNS может выйти за timeout, «because the DNS request cannot be interrupted with the available APIs» ([`src/agent.rs`](https://github.com/algesten/ureq/blob/ba4555d54fd72e16fc737f7e8fcc812efb131260/src/agent.rs#L471)).

Второй, отдельный разрыв того же клиента: `timeout_connect` **замещает** общий deadline на фазе connect, а не урезается им. В `connect_host` при заданном `timeout_connect` дедлайн connect считается от него, иначе берётся `unit.deadline`. Значение по умолчанию — `Some(30s)` (`agent.rs`, поле `timeout_connect` в `AgentBuilder::default` / конструкторе конфигурации). Поэтому один вызов `.timeout(budget)` не ограничивает ни DNS, ни TCP connect: connect может идти до 30 с. Документация прямо говорит, что при обоих значениях `.timeout_connect()` имеет приоритет.

Ошибка DNS ещё и содержит имя: `` resolve dns name '{}' ``, в `netloc` лежит `host:port`. После `call` транспортная ошибка получает полный URL (`request.rs`: `.map_err(|e| e.url(url))`). `Display` для `Transport` печатает этот URL первым полем (`error.rs`). `Debug` структуры тоже содержит `url`. Текущий `safe_ureq_error` в `src/vpn.rs` этого избегает: он смотрит только `ErrorKind` и не форматирует ошибку. Новый адаптер обязан делать так же. `{e}` и `{e:?}` для `ureq::Error` уже утечка секретного URL.

При `redirects == 0` цикл в `unit.rs` возвращает ответ 3xx и не ходит по `Location`. Это совпадает с контрактом переговоров. При ненулевом лимите клиент следует `301/302/303/307/308` для GET, а в текст ошибки «Bad redirection» попадает сам `Location`. `debug!` пишет оба URL. `https_only(true)` отвергает любой не-https, в том числе исходный `http` с opt-in, поэтому этим флагом нельзя выразить политику «исходный http разрешён, понижение схемы запрещено».

Прокси HTTP CONNECT для `https` есть без feature `socks-proxy`. SOCKS в этот транспорт не включать: ветка SOCKS при таймауте оставляет фоновый поток и пишет `host:port` в текст ошибки (`stream.rs`, `connect_socks`). `try_proxy_from_env` по умолчанию выключен; так и оставить.

Сборка: default features `tls` + `gzip`, TLS — `rustls` 0.23 с `ring`, без OpenSSL. Это уже то, что в `Cargo.lock`. musl для текущего бинарника этим клиентом уже платится. Лицензия MIT OR Apache-2.0.

`AgentBuilder::resolver` уже есть: синхронный `Resolver`, в том числе замыкание `Fn(&str) -> io::Result<Vec<SocketAddr>>`. Крючок для своего deadline — этот trait, не патч ureq.

## Что считается «общим deadline»

`getaddrinfo` переносимо не отменяется. Варианты ниже, где ожидание DNS идёт в другом потоке, возвращают **вызывающему** управление по deadline. Поток в `getaddrinfo` при этом остаётся, пока libc сам не вернётся. Это не отмена системного вызова. Сколько он ещё живёт, задаёт резолвер libc, не библиотека; исходник musl в этом прогоне не открывался, число секунд не утверждаю.

Поэтому критерий H.04 «медленный DNS завершается в пределах budget» проверяем как время возврата из `FetchTransport`, а не как отсутствие заблокированного потока. Остаточный риск: пачка зависших lookup копит потоки, пока libc их не отпустит. На один запрос переговоров потолок — `max_requests` (не больше 8).

## Варианты

### B. Оставить ureq 2.12.1 и ограничить резолв потоком

Свой `Resolver`: `thread::spawn` + `recv_timeout(deadline)`. По таймауту вернуть ошибку, которую адаптер превращает в `FetchFailure::Timeout`, не печатая `netloc`. После успешного резолва в ureq передать уже IP и выставить **все** таймауты фазы в остаток deadline: `.timeout`, `.timeout_connect`, `.timeout_read`, `.timeout_write`. Иначе дефолтные 30 с connect снова растянут запрос. `redirects(0)` для переговоров. Чтение тела — не больше `max_body_bytes` байт, которые попадут в `HttpResponse`. Feature `gzip` для этого агента не использовать как способ соблюсти лимит: см. общий запрет ниже. Прокси — только `http://127.0.0.1:port` или `http://[::1]:port`, как `agent_with_redirects` в `src/vpn.rs`.

Последствия: второй HTTP-стек не появляется. `src/vpn.rs` не трогается. Дыра DNS закрывается в адаптере, ровно в том месте, где в ureq стоит TODO. Код резолва и склеивания остатка deadline — наш; ошибка здесь снова откроет зависание.

Риски: поток lookup не уничтожается по таймауту. Ошибки ureq по-прежнему содержат URL, если их напечатать. `https_only` не выражает нужную redirect-политику — её пишет адаптер, если лимит redirect когда-нибудь станет ненулевым.

Проверка на `127.0.0.1`: резолвер теста спит дольше budget и только потом вернул бы `127.0.0.1:port`. Ожидание: возврат `FetchFailure::Timeout` быстрее budget (для budget 200 мс — уложиться в 500 мс) и ноль `accept` на слушателе. Контрольный прогон с мгновенным резолвером доходит до сервера на `127.0.0.1`. Slow-loris, тело длиннее лимита и `302` на самого себя — отдельные тесты того же слушателя. Маркер из URL отсутствует в `Display` и `Debug` ошибки адаптера.

### C. ureq 3.4.2, `timeout_global`

`timeout_global` документирован как срок от DNS lookup до конца тела и покрывает остальные таймауты. По умолчанию он `None`, как и `timeout_resolve`. Если задать `timeout_global` равным `request_timeout`, `CallTimings::next_timeout` для фазы Resolve смотрит и Global, и PerCall (`timings.rs`: в список проверки всегда входят Global и PerCall). Тогда `DefaultResolver` не идёт в синхронный `to_socket_addrs`, а вызывает `resolve_async`.

`resolve_async` ([`resolver.rs`](https://github.com/algesten/ureq/blob/2e9ef24a80e1e7ecd0e6604f98ba49aceb7ec322/src/unversioned/resolver.rs#L142)): поток делает `to_socket_addrs`, вызывающий ждёт `recv_timeout`. При таймауте возвращается `Error::Timeout`. Поток не присоединяется и syscall не прерывается. Комментарий у `timeout_resolve` говорит «spawn a thread»; комментарий в функции упоминает `getaddrinfo_a` как несделанное. Trait `Resolver` лежит в `ureq::unversioned` и явно не следует semver. Для продакшена хватает `DefaultResolver`; свой trait нужен только тесту.

`max_redirects(0)` оставляет 3xx вызывающему. `https_only` снова слишком груб для opt-in `http`. `Error::RequireHttpsOnly` и `Error::BadUri` кладут строку URI в сам вариант и в `Display` (`error.rs`, `util.rs` `ensure_valid_url`). Маппинг в `FetchFailure` — по варианту enum, без `Display`. `debug!` в `run.rs` печатает URI, если в процессе появится logger на crate `log`. Сейчас своего logger в `Cargo.toml` нет; адаптер не должен его включать.

Тело: `Body::read_to_vec` имеет удобный потолок 10 МиБ, но `BodyWithConfig::limit` по умолчанию `u64::MAX`. Лимит надо задавать явно. Стек чтения — `GzipDecoder` поверх `LimitReader` (`body/mod.rs`): лимит считает сжатые байты с сокета, а в память попадает распакованное. Gzip-bomb укладывается в лимит сжатого ввода и раздувает `Vec`. Для этого транспорта gzip-feature не включать либо считать байты уже после распаковки и останавливаться на `max_body_bytes`.

Прокси: HTTP и HTTPS CONNECT есть без `socks-proxy`. SOCKS не включать.

Сборка: edition 2024, `rust-version = 1.85`. В дереве сейчас `rustc 1.98.1`. Default TLS — rustls 0.23 + ring, не aws-lc-rs. Лицензия MIT OR Apache-2.0. Прямые добавки поверх уже имеющихся rustls/webpki-roots: сам ureq 3, `ureq-proto`, `http`, `utf8-zero`, `percent-encoding`. `cargo tree` в этом прогоне не снимался, точный размер бинарника не утверждаю. Пока `vpn.rs` на ureq 2, в бинарнике два клиента. rustls 0.23 у обоих, Cargo может свести одну версию; код клиентов не сводится.

Последствия: deadline DNS лежит в библиотеке, а не в нашем потоке. Цена — вторая копия клиента до переноса legacy fetch и более широкая поверхность ошибок со строкой URI.

Риски: тот же несбрасываемый поток `getaddrinfo`. `unversioned` Resolver, если тест или прод его реализует, может сломаться на минорном ureq 3. Лимит тела библиотеки не равен лимиту байт в `HttpResponse`, если gzip включён.

Проверка на `127.0.0.1`: свой `Resolver`, который спит и затем вернул бы `127.0.0.1`. С `timeout_global` 200 мс ожидание — `Error::Timeout` без `accept`. `DefaultResolver` так не проверить: он зовёт системный `getaddrinfo` и не принимает адрес DNS-сервера, а подменять `/etc/resolv.conf` нельзя. Slow-loris, большое тело и redirect — слушатель на `127.0.0.1`, как в B.

### D. minreq 3.0.0

`enforce_timeout` ([`connection.rs`](https://github.com/neonmoe/minreq/blob/eb528443b54d1f38ff9b089bae2832f0753f7fe7/src/connection.rs#L349)) запускает весь запрос, включая `to_socket_addrs`, в потоке и ждёт `recv_timeout`. Вызывающий возвращается по deadline. Поток при таймауте отсоединяется, syscall не отменяется. Это честный общий deadline по времени возврата.

Дальше критерий ломается. `with_timeout` задаётся целыми секундами. `RequestSpec` передаёт `Duration`; остаток budget бывает меньше секунды, потолок одного запроса — 10 с, но не «только целые секунды». Выразить 200 мс этим API нельзя.

Redirect по умолчанию включён, потолок 100, коды только `301/302/303/307` (нет `308`), запрета `https`→`http` нет. `log::debug!` пишет URL назначения. Своего резолвера нет. Прокси — отдельный feature, только HTTP CONNECT. TLS — feature `https-rustls` (rustls 0.23). Лицензия ISC. Таймаут в одну секунду на slow-loris `127.0.0.1` проверить можно; sub-second deadline и подменный DNS — нет.

### E. Свой клиент на rustls

Полный контроль deadline, лимита и redirect. rustls в дереве уже есть. Придётся самим держать HTTP/1.1, chunked, TLS handshake timeout, CONNECT и чтение с лимитом. Это больше кода, чем обёртка резолва, и ошибки этой поверхности (контрабанда заголовков, недочитанный chunked, CONNECT) дороже дыры, которую закрывает вариант B. Имеет смысл только если B и C оба отвергнуты.

Проверка на `127.0.0.1` та же, что у B: подменный резолвер, slow-loris, тело, redirect.

### Не вариант: ureq 2.12.1 как есть

Один `.timeout()` не ограничивает DNS и при дефолтном `timeout_connect` не ограничивает connect. H.04 этим не закрывается.

### Не разобран построчно: reqwest

В этот черновик не входит. В `docs/ARCHITECTURE.md` текущий бинарник описан без tokio; тащить runtime ради блокирующего клиента — другой продукт, не латание deadline.

## Общие ограничения любого выбранного варианта

Их не выбирает D4. Они остаются в задании Composer:

1. Наружу только `FetchFailure` и `HttpResponse`. Не вызывать `Display`/`Debug` ошибки клиента. Не логировать URL, `Location`, `netloc`.
2. Не читать переменные `HTTP(S)_PROXY`. Прокси только loopback FlClash, HTTP CONNECT, без SOCKS.
3. Для переговоров не следовать redirect. Если у транспорта есть отдельный режим с лимитом больше нуля: считать хопы, отвергать `https`→`http` до соединения, заново применять остаток deadline и лимит тела, не переносить `Authorization`.
4. Лимит — это байты, которые адаптер кладёт в `HttpResponse.body`. Автораспаковка gzip/br до этого подсчёта запрещена, иначе bomb обходит `max_body_bytes`. Распаковка, если она нужна, только своим `flate2` с потолком на выход.
5. Имя в ошибке DNS (`resolve dns name`) и URL в `ureq::Transport` считаются секретом наравне с полным URL.

## Мнение рецензента, не решение

Для H.04 я бы брал **вариант B**: locked ureq 2.12.1, резолв через уже существующий `Resolver` с `recv_timeout`, остальные фазы на остатке того же deadline, `redirects(0)`, лимит тела свой, gzip клиентом не распаковывать. Причина: дыра подтверждена и закрывается в адаптере, второй стек не нужен, субсекундный `Duration` сохраняется, tokio не появляется. Вариант C честнее переносит DNS-ожидание в библиотеку, но до переноса `vpn.rs` платит вторым клиентом и строками URI внутри `Error`, а поток `getaddrinfo` всё равно не убивает. D и E я бы не брал: у minreq таймаут в секундах, свой клиент повторяет ureq.

## Что считать закрытием черновика

Утверждение про ureq 2 подтверждено строками `stream.rs` и `agent.rs` публикации 2.12.1. У B, C, D и E выше есть способ проверить deadline на `127.0.0.1` без внешнего DNS и без правки резолвера хоста. D4 по-прежнему открыт.
