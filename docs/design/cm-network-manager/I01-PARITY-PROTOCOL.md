# I01: воспроизводимое сравнение UPD/CM и FlClash

Исполнитель: отдельный тестовый агент. Координатор задаёт [контракт и допуски](I01-TEST-CONTRACT.md), не выполняет проверки. Пакет/установка/переключение живого VPN запрещены.

## Исходная граница

Свежий baseline исполнителя: установлен UPD, CM в системе отсутствует, `upd-vpn.service` inactive, FlClash/FlClashCore живы. Runtime/source UPD содержат 62 одинаковых полных node definitions; UPD/FlClash saved имеют лишь одно полное совпадение из 62. Это исходные снимки, не одинаковая экспериментальная установка.

FlClashCore требует собственного desktop IPC, а не CLI standalone mihomo. Matching-tag [main.go](https://github.com/chen08209/FlClash/blob/v0.8.98/core/main.go) запускает IPC по аргументу, [server.go](https://github.com/chen08209/FlClash/blob/v0.8.98/core/server.go) задаёт framing. Несовместимый `-h/-f/-t` не доказывает неработоспособность транспорта. Развернуть только собственный test worker; mutating IPC живого FlClash не использовать.

## Шаги C05: до передачи внешнего трафика

1. Зафиксировать digests A/B binaries и доступную build/revision metadata. Release tag — источник протокола, не автоматическое доказательство исходного commit бинарника.
2. Использовать локальную неизменённую копию одного полного node definition для обоих workers. Полное равенство проверяется локально, включая credentials; в экспорт идут только boolean/counts и допустимые digests. Принцип выбора test node и исходный source generation указать явно.
3. Сформировать одинаковый минимальный effective config. Fixed selection, один узел, MATCH к нему; providers/remote assets/autoupdate/NTP/system-time writes отключены; TUN/auto-route/auto-redirect/redirect/TProxy/listeners для чужих сетей отключены; DNS/IPv6/transport defaults заданы явно. Удаление global fields documented, а не скрытое изменение тестируемой стороны.
4. Все proxy/control endpoints — выделенные loopback/socket paths, временный private home. Workers без CAP_NET_ADMIN, CAP_SYS_TIME и `/dev/net/tun`; host filesystem read-only, write только в выделенный disposable mount. Final конфиги с секретами не записывать в репозиторий/evidence.
5. Проверить native validate A и фактический init/setup B в private net namespace без внешней сети. Подтвердить effective config и fixed selected node обоих workers доступным verified механизмом. Отсутствие native operation не выдавать за success.
6. Readiness и RPC bootstrap — отдельные assertions. Принятый TCP listener и API не доказывают remote connectivity. Если сопоставимый effective config подтвердить нельзя, C05 = BLOCKED и C06 не запускать.

## Шаги C06: ограниченный pilot

Публичный test endpoint один и тот же. Concurrency 1, timeout не более 10 секунд, всего не более 30 measured requests плюс 3 warm-up, общий deadline 15 минут. План A1/B/A2: по 10 measured + 1 warm-up на фазу. Для каждой фазы документировать lifecycle, повторное применение config и состояние кеша; запрещено незаметно менять endpoint или узел между фазами.

Записать UTC, outcome, error stage, HTTP status и latency/TTFB если инструмент их действительно измеряет. Failed/timeout observations не выкидывать. Размер/порог окончательного performance experiment определить по разбросу пилота; 30 запросов не подтверждают устойчивый p95 или долгосрочную надёжность.

При работающем FlClash host TUN испытания новых proxy workers могут иметь внешний путь через него. До pilot установить и записать route scope. Если общий внешний путь доказуемо одинаков и не меняется, допускается **host-conditioned proxy pilot**: он доказывает только этот путь и не объясняет прежний отказ host UPD. Если равенство outer path не доказано, performance parity — INCONCLUSIVE; не обходить ограничение изменением живой сети.

## Что считать результатом

| Наблюдение | Допустимый вывод | Недопустимый вывод |
|---|---|---|
| RPC bootstrap работает | Драйвер связался с собственным Fl worker | VPN работает |
| Одинаковый node принят обоими | Совместимость этого определения с проверенными configs | Все subscriptions/REALITY/WS работают |
| Generate-204 проходит proxy worker | Connectivity этого node/core/path в момент проверки | Host TUN исправлен, приложение изолировано |
| Разные error classes/успехи | Гипотеза для следующего контролируемого изменения | Причина старого WS 502 доказана |
| Оба работают при active host FlClash | Proxy path в текущем внешнем окружении работает | Оба независимо работают без FlClash |
| Низкая latency в пилоте | Наблюдаемая выборка и разброс | Доказанная лучшая долгосрочная скорость |

Каждый дополнительный эксперимент меняет один фактор: full node provenance, effective global knob либо binary revision. Старые config timestamps и журналы сохраняются отдельной цепочкой; успех текущего теста не опровергает прошлый отказ.

## Завершение

C14 проверяет cleanup собственных workers/sockets/configs и снимки host assets до/после. Dynamic cache живого FlClash может изменяться независимо; такие изменения отделяются от test-owned writes. При ошибке cleanup сохраняется redacted trace, не удаляются чужие ресурсы.

Настоящее подключение хоста и TUN path требуют следующего явно ограниченного испытания; этот protocol его не маскирует и не разрешает переключать текущий VPN. Полноценная миграция и восстановление данных относятся к отдельной незакрытой части I01.T04.
