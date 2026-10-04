# I03.T01: закреплённые первичные источники

Sol сверил матрицу импорта с upstream **mihomo v1.19.32**, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`: [релиз](https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.32), [commit](https://github.com/MetaCubeX/mihomo/commit/88dcbf7f1614a67c3b36b848ee3592dfa92ada36). Это выбранная версия контракта, не утверждение о версии установленного бинарника. FlClash fork этим pin не отождествляется.

| Протокол | CM import transport subset | Проверенный источник pin |
|---|---|---|
| VLESS | TCP, WS, HTTP, H2, gRPC, XHTTP | [vless.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/vless.go) |
| VMess | TCP, WS, HTTP, H2, gRPC | [vmess.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/vmess.go) |
| Trojan | TCP, WS, gRPC | [trojan.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/trojan.go) |
| Shadowsocks | TCP; UDP отдельно от stream transport | [shadowsocks.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/shadowsocks.go) |
| HTTP / SOCKS5 | TCP | [http.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/http.go), [socks5.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/socks5.go) |
| Hysteria2 / TUIC | QUIC | [hysteria2.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/hysteria2.go), [tuic.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/tuic.go) |
| WireGuard | native WireGuard | [wireguard.go](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/wireguard.go) |

Матрица задаёт разрешённое подмножество импорта CM. Транспорт сам по себе не подтверждает совместимость всех security/plugin/packet settings и не доказывает работу endpoint. Runtime/core validation отложены до установки и последующих этапов. В частности, наличие опции upstream не означает её автоматического допуска CM: расширенные ECH/ShadowTLS/Restls/JLS, незнакомые plugins и дополнительные transports требуют отдельной явной поддержки. Их отказ не должен превращаться в молчаливую замену на TCP.

[BasicOption](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/base.go) содержит interface/routing/dialer controls. CM import не получает через них право управлять хостовой сетью: `interface-name`, `routing-mark`, `dialer-proxy` ограничиваются политикой A+C. Native listeners, TUN, controller, ports и routes также не принимаются как параметры узла. WG DNS/ip-stack controls требуют явной последующей политики контроллера. Отсутствие разрешения на поле означает явную диагностику, а не отбрасывание влияющего параметра.

Форматы контракта: URI list, base64 URI list, mihomo YAML и mihomo JSON. WireGuard представлен native outbound, собственный URI convention не выдумывается. Native Xray/sing-box не выдаются за mihomo-конфиг; их текущий результат — unsupported до соответствующего adapter. Конвертация через облачный сервис в контракт не входит.

Статус проверки здесь — **статический обзор первичных источников**. Бинарник mihomo и службы не запускались, сеть провайдера не опрашивалась; runtime **NOT_RUN**.
