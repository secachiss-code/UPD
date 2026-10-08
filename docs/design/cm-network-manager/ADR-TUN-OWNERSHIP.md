# ADR: владение TUN (I04.T03.b)

Статус: выбран вариант A. Продуктовый путь VPN на хосте этим решением не меняется. Подтверждение — harness `tests/audit_i04_tun.rs` в user+net namespace, без правок сети хоста.

Дата замера: 2026-10-08. Ядро: Mihomo Meta v1.19.32, `CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo`.

## Выбор

CM (привилегированная сторона) заранее создаёт persistent TUN, назначает owner равным uid воркера, ставит MTU и те префиксы, которые этот бинарник реально занимает, и только потом запускает ядро. Ядро открывает уже существующее устройство с `CapEff = 0` и `auto-route: false`. Остановка по команде CM удаляет устройство. После `kill -9` устройство остаётся, чтобы следующий старт мог к нему привязаться снова.

Вариант B — ядро создаёт TUN само — не берём: процессу ядра нужен `CAP_NET_ADMIN`, после `kill -9` устройства нет, и system stack этой сборки не ставит глобальный IPv6, когда устройство создаёт само ядро.

## Как устроен harness

Внешний uid и начало диапазона `/etc/subuid` читаются в тесте (`euid`, строка `USER` или `id -un`). Число 1000 в тест не зашито.

```text
unshare --user --net \
  --map-users=0:<euid>:1 --map-users=1:<subuid>:1 \
  --map-groups=0:<euid>:1 --map-groups=1:<subuid>:1
ip link set lo up
```

Внутри namespace uid 0 — это внешний пользователь. uid 1 — первый подчиненный id из `/etc/subuid`. `CAP_NET_ADMIN` — бит 12 в `CapEff` из `/proc/<pid>/status`.

Общий фрагмент конфига обоих вариантов (`stack: system`, маршруты выключены):

```json
{"mode":"direct","log-level":"info","find-process-mode":"off","ipv6":true,
 "tun":{"enable":true,"stack":"system","auto-route":false,"auto-redirect":false,
        "mtu":1400,"inet4-address":["172.19.0.1/30"],"inet6-address":["fd00:c::1/64"]}}
```

Имена устройств в замере: `cmtunb` (вариант B) и `cmtun2` (вариант A).

## Вариант B — ядро создаёт устройство

Команда: `mihomo -d <dir> -f config.json` от uid 0 namespace, без `setpriv`.

| Критерий | Замер |
|---|---|
| `CapEff` ядра | `000001ffffffffff` (бит 12 установлен) |
| Строка прослушивания | `cmtunb([198.18.0.1/30],[])`, `mtu: 1400`, `auto route: false` |
| Что просили в конфиге | IPv4 `172.19.0.1/30`, IPv6 `fd00:c::1/64` |
| Что заняло ядро | `198.18.0.1/30`, список IPv6 пуст |
| `ip -d link show cmtunb` | `mtu 1400`, `persist off` |
| `ip -6 addr show cmtunb` | нет адреса `scope global` |
| `ip route` | нет строки `default` |
| после `kill -9` | `ip link show cmtunb` не находит устройство |

System stack этой сборки подменяет запрошенный IPv4 на `198.18.0.1/30` и не применяет `inet6-address`, когда устройство создаёт само ядро.

## Вариант A — устройство создаёт CM

Создание (uid 0 namespace), затем ядро без capabilities:

```text
python3: open /dev/net/tun
  ioctl TUNSETIFF  0x400454ca  IFF_TUN|IFF_NO_PI (0x0001|0x1000), name cmtun2
  ioctl TUNSETOWNER 0x400454cc owner = 1
  ioctl TUNSETPERSIST 0x400454cb persist = 1
ip link set cmtun2 mtu 1400
ip addr add 198.18.0.1/30 dev cmtun2
ip -6 addr add fd00:c::1/64 dev cmtun2
ip link set cmtun2 up
setpriv --reuid=1 --regid=1 --clear-groups --inh-caps=-all --bounding-set=-all -- \
  mihomo -d <dir> -f config.json
```

Префиксы на устройстве — те, которые этот бинарник пытается занять (`198.18.0.1/30` и `fd00:c::1/64`). Если их нет, старт от uid 1 падает: `listen tcp6 [fd00:c::1]:0: bind: cannot assign requested address` (для IPv4 та же ошибка на `198.18.0.1`). Конфиг по-прежнему просит `172.19.0.1/30`; на уже существующем устройстве ядро оставляет заранее назначенный `198.18.0.1/30`.

| Критерий | Замер |
|---|---|
| `CapEff` ядра | `0000000000000000` |
| Строка прослушивания | `cmtun2([198.18.0.1/30],[fd00:c::1/64])`, `auto route: false` |
| `ip -d link show cmtun2` | `mtu 1400`, `persist on`, `state UP` |
| после `kill -9` | `ip link show cmtun2` устройство находит |
| снятие | `ip link del cmtun2` выполняет сторона CM, не ядро |

`ip link` на хосте может показать owner как имя uid 1 из `/etc/passwd` (часто `bin`). В namespace владелец — uid 1, тот же, что у `setpriv --reuid=1`.

Строка `[TUN] default interface lost by monitor` в netns без маршрута по умолчанию не считается отказом TUN: прослушивание при этом уже есть, default-маршрута нет.

## Что это значит для CM

Привилегия `CAP_NET_ADMIN` остаётся у процесса, который создаёт и удаляет устройство. Воркер ядра её не получает. MTU задаёт CM (в замере 1400, ядро его сохраняет). IPv6 появляется только если префикс уже висит на устройстве; запрос `inet6-address` в конфиге сам по себе глобальный адрес не создаёт. Перезапуск после падения ядра не требует заново создавать устройство.
