# Материалы к D2: `skip-cert-verify`

Статус: материал. Решение пользователя записано в [I03-DECISIONS-2026-10-06.md](../I03-DECISIONS-2026-10-06.md): явный insecure-признак, поле узла `tls_verification`. Дата материала: 2026-10-06. Пин: mihomo `v1.19.32`, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`.

## Вопрос пользователю

Что делать с полем `skip-cert-verify` у узла: отвергать фиксированным кодом, как очередь велит до решения; принимать только как явный insecure-признак с отдельной отметкой, которую пользователь видит; или принимать молча и потом отдавать в ядро как есть?

## Что поле делает в ядре

Поле попадает в `tls.Config.InsecureSkipVerify` соединения с самим прокси, не с сайтами за туннелем. Включённый флаг отключает проверку цепочки и имени сертификата на этом hop. Пароль, uuid и ключ узла идут по каналу, который можно подменить.

Общая сборка TLS: [`component/ca/config.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/component/ca/config.go#L101). Если задан `fingerprint` или `name-cert-verify`, ядро тоже ставит `InsecureSkipVerify = true`, но затем вешает свой `VerifyConnection` (pin сертификата или проверка другого имени). Голый `skip-cert-verify: true` без этих двух полей своего verifier не ставит.

Где у pinned исходника есть тег `skip-cert-verify`:

| Протокол | Файл и строка | Куда садится |
|---|---|---|
| VLESS | [`vless.go` 84](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vless.go#L84) | `InsecureSkipVerify` в `ca.GetTLSConfig`; у shadow-tls/restls/jls то же поле уходит в `SkipCertVerify` |
| VMess | [`vmess.go` 65](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/vmess.go#L65) | тот же TLS hop |
| Trojan | [`trojan.go` 53](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/trojan.go#L53) | тот же TLS hop |
| HTTP | [`http.go` 37](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/http.go#L37) | TLS только при `tls: true` |
| SOCKS5 | [`socks5.go` 39](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/socks5.go#L39) | TLS только при `tls: true` |
| Hysteria2 | [`hysteria2.go` 55 и 84](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/hysteria2.go#L55) | основной TLS и вложенный realm |
| TUIC | [`tuic.go` 56](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/tuic.go#L56) | `InsecureSkipVerify` |
| Shadowsocks | [`shadowsocks.go` 73, 90, 101, 113](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/shadowsocks.go#L73) | тег `obfs`, не сам шифр: v2ray-plugin, gost, restls и соседние obfs-опции |

В `adapter/outbound/wireguard.go` этого тега нет.

Конвертер URI того же commit сам выставляет поле из query: hysteria/hysteria2 `insecure`, trojan `allowInsecure`, socks принудительно `true`, vmess в этом конвертере пишет `false`. Это поведение ядра при разборе URI, не решение импорта cm. Источник: [`common/convert/converter.go`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/common/convert/converter.go#L68).

`Verification` в `src/profiles/model.rs` сейчас имеет оси `Net`, `Region`, `State`, `App` и значения `Unknown`, `Partial`, `Verified`, `Blocked`, `Error`. Оси «сертификат прокси не проверяется» нет.

До решения очередь Composer отвергает поле фиксированным кодом. Принять его или выкинуть молча она запрещает.

## Варианты

1. Отказ фиксированным кодом. Узел с `skip-cert-verify: true` не импортируется. Издатели часто ставят флаг и при нормальном сертификате: такие узлы пользователь не получит, даже если сертификат был бы валиден. Поле не пропадает молча, документ или узел падает с типизированной ошибкой. В ядро небезопасный hop не попадает. Отметка в `Verification` не нужна.

2. Явный insecure-признак. Узел импортируется, флаг хранится отдельно от обычного TLS, генератор конфига передаёт его в ядро только когда признак включён. Пользователь видит, что hop к этому узлу сертификат не проверяет. В текущем `Verification` такой оси нет: отметка — новое поле узла или новая ось, а не запись в существующий enum. Пока отметки нет, вариант совпадает с молчаливым принятием. `false` и отсутствие поля остаются обычным путём без отметки.

3. Принимать молча. Узел выглядит так же, как узел с проверкой сертификата, а в ядро уходит `InsecureSkipVerify: true`. Это не silent dropping поля: поле принято и действует. Прячется снижение проверки. Для подписки, где флаг стоит на каждом узле, весь выход к прокси становится без проверки сертификата, и граф этого не показывает.

## Мнение рецензента, не решение

План остатка уже пишет рекомендацию: отказ по умолчанию и явный insecure-флаг с отметкой. Это вариант 2, и он требует нового места для отметки, потому что `Verification` такой оси не содержит. Вариант 1 безопасен и отрезает узлы, которые издатель пометил флагом по привычке. Вариант 3 оставляет подмену hop незаметной.
