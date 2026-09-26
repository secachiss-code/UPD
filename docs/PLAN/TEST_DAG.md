# DAG контрактов и испытаний

Каждая вершина — один контракт. Читать её detail только после завершения всех зависимостей ниже и соответствующего исправления из [FIX_DAG.md](FIX_DAG.md). Для каждого контракта используется собственный изолированный fixture; `TEST-01` относится только к TUI.

| Порядок | Контракт | Предусловия теста |
|---:|---|---|
| 1 ✓ | [SEC-01](test/SEC-01.md) — Исключить URL подписки из ошибок? | [fix/SEC-01](fix/SEC-01.md) |
| 2 ✓ | [FS-01](test/FS-01.md) — Выделять закрытое временное место для зеркал? | [fix/FS-01](fix/FS-01.md) |
| 3 ✓ | [SEC-02](test/SEC-02.md) — Требовать проверенный digest перед запуском mihomo? | [fix/SEC-02](fix/SEC-02.md) |
| 4 ✓ | [SEC-04A](test/SEC-04A.md) — Сделать APT backup закрытым? | [fix/SEC-04A](fix/SEC-04A.md) |
| 5 ✓ | [UPD-04A](test/UPD-04A.md) — Отличать сбой проверки Flatpak от пустого списка? | [fix/UPD-04A](fix/UPD-04A.md) |
| 6 ✓ | [UPD-04B](test/UPD-04B.md) — Отличать сбой проверки fwupd от пустого списка? | [fix/UPD-04B](fix/UPD-04B.md) |
| 7 ✓ | [UPD-05](test/UPD-05.md) — Проверять статус zypper list-updates? | [fix/UPD-05](fix/UPD-05.md) |
| 8 ✓ | [NET-01](test/NET-01.md) — Учитывать IPv6 default route в отпечатке сети? | [fix/NET-01](fix/NET-01.md) |
| 9 ✓ | [NET-02](test/NET-02.md) — Повторять выбранные зеркала при полном первом отказе? | [fix/NET-02](fix/NET-02.md) |
| 10 ✓ | [NET-03](test/NET-03.md) — Проверять короткий успешный ответ зеркала? | [fix/NET-03](fix/NET-03.md) |
| 11 ✓ | [SPACE-01A](test/SPACE-01A.md) — Не подставлять ноль при неизвестном размере загрузки? | [fix/SPACE-01A](fix/SPACE-01A.md) |
| 12 ✓ | [SPACE-01B](test/SPACE-01B.md) — Проверять доступный каталог кэша? | [fix/SPACE-01B](fix/SPACE-01B.md) |
| 13 ✓ | [GEO-01](test/GEO-01.md) — Отвергать усечённый геофайл? | [fix/GEO-01](fix/GEO-01.md) |
| 14 ✓ | [INSTALL-01](test/INSTALL-01.md) — Запретить ручную установку из пакетного бинарника? | [fix/INSTALL-01](fix/INSTALL-01.md) |
| 15 ✓ | [INSTALL-02](test/INSTALL-02.md) — Удалять только принадлежащие upd unit-файлы? | [fix/INSTALL-02](fix/INSTALL-02.md) |
| 16 ✓ | [DATA-02](test/DATA-02.md) — Различать отсутствие и повреждение subs.json? | [fix/DATA-02](fix/DATA-02.md) |
| 17 ✓ | [SYS-01](test/SYS-01.md) — Разбирать service cgroup v1? | [fix/SYS-01](fix/SYS-01.md) |
| 18 ✓ | [SYS-02](test/SYS-02.md) — Искать модули текущего ядра в обеих раскладках? | [fix/SYS-02](fix/SYS-02.md) |
| 19 ✓ | [PARSE-01](test/PARSE-01.md) — Декодировать filename* без паники? | [fix/PARSE-01](fix/PARSE-01.md) |
| 20 ✓ | [VPN-02](test/VPN-02.md) — Находить IPv6 listener FlClash? | [fix/VPN-02](fix/VPN-02.md) |
| 21 ✓ | [TEST-01](test/TEST-01.md) — Сделать TUI тест детерминированным? | [fix/TEST-01](fix/TEST-01.md) |
| 22 ✓ | [META-01](test/META-01.md) — Согласовать лицензионные метаданные с правообладателем? | [fix/META-01](fix/META-01.md) |
| 23 ✓ | [SEC-06](test/SEC-06.md) — Отказывать при ошибке создания секрета API? | [fix/SEC-06](fix/SEC-06.md) |
| 24 ✓ | [DATA-04](test/DATA-04.md) — Сохранять существующий конфиг при ошибке чтения? | [fix/DATA-04](fix/DATA-04.md) |
| 25 ✓ | [SEC-03](test/SEC-03.md) — Запретить незашифрованные URL подписок? | [fix/SEC-03](fix/SEC-03.md), [SEC-01](test/SEC-01.md) |
| 26 ✓ | [SEC-04B](test/SEC-04B.md) — Защитить сохранённый APT URI? | [fix/SEC-04B](fix/SEC-04B.md), [SEC-04A](test/SEC-04A.md) |
| 27 ✓ | [SEC-04C](test/SEC-04C.md) — Сохранить права APT source-файла? | [fix/SEC-04C](fix/SEC-04C.md), [SEC-04A](test/SEC-04A.md) |
| 28 ✓ | [SEC-05](test/SEC-05.md) — Удалять управляющие символы из названия профиля? | [fix/SEC-05](fix/SEC-05.md), [PARSE-01](test/PARSE-01.md) |
| 29 ✓ | [UPD-01](test/UPD-01.md) — Возвращать ошибку при неудачной проверке системных пакетов? | [fix/UPD-01](fix/UPD-01.md), [UPD-05](test/UPD-05.md) |
| 30 ✓ | [UPD-02A](test/UPD-02A.md) — Учитывать ошибку установки Flatpak в результате update? | [fix/UPD-02A](fix/UPD-02A.md), [UPD-04A](test/UPD-04A.md) |
| 31 ✓ | [UPD-02B](test/UPD-02B.md) — Учитывать ошибку установки прошивки в результате update? | [fix/UPD-02B](fix/UPD-02B.md), [UPD-04B](test/UPD-04B.md) |
| 32 ✓ | [UPD-06](test/UPD-06.md) — Охватить пользовательские Flatpak установки? | [fix/UPD-06](fix/UPD-06.md), [UPD-04A](test/UPD-04A.md) |
| 33 ✓ | [SPACE-01C](test/SPACE-01C.md) — Считать ошибку statvfs ошибкой проверки места? | [fix/SPACE-01C](fix/SPACE-01C.md), [SPACE-01B](test/SPACE-01B.md) |
| 34 ✓ | [DATA-01](test/DATA-01.md) — Сериализовать изменения списка VPN подписок? | [fix/DATA-01](fix/DATA-01.md), [DATA-02](test/DATA-02.md) |
| 35 ✓ | [NET-04](test/NET-04.md) — Подтверждать смену сети только после применения зеркал? | [fix/NET-04](fix/NET-04.md), [NET-02](test/NET-02.md) |
| 36 ✓ | [VPN-03](test/VPN-03.md) — Сообщать об ошибке перезапуска после обновления ядра? | [fix/VPN-03](fix/VPN-03.md), [SEC-02](test/SEC-02.md) |
| 37 ✓ | [UPD-03](test/UPD-03.md) — Передавать ошибку фонового auto через exit code? | [fix/UPD-03](fix/UPD-03.md), [UPD-01](test/UPD-01.md), [UPD-04A](test/UPD-04A.md), [UPD-04B](test/UPD-04B.md) |
| 38 ✓ | [UPD-04C](test/UPD-04C.md) — Отличать сбой проверки AUR от пустого списка? | [fix/UPD-04C](fix/UPD-04C.md), [UPD-01](test/UPD-01.md) |
| 39 ✓ | [APT-02](test/APT-02.md) — Делать переход APT на mirror+file восстановимым? | [fix/APT-02](fix/APT-02.md), [SEC-04A](test/SEC-04A.md), [SEC-04B](test/SEC-04B.md), [SEC-04C](test/SEC-04C.md) |
| 40 ✓ | [DATA-03](test/DATA-03.md) — Сохранять профиль до удаления старого? | [fix/DATA-03](fix/DATA-03.md), [DATA-01](test/DATA-01.md), [DATA-02](test/DATA-02.md) |
| 41 ✓ | [UPD-07](test/UPD-07.md) — Считать невозможность запуска pacman ошибкой проверки? | [fix/UPD-07](fix/UPD-07.md), [UPD-01](test/UPD-01.md) |
| 42 ✓ | [VPN-01](test/VPN-01.md) — Остановить VPN при удалении последней подписки? | [fix/VPN-01](fix/VPN-01.md), [DATA-02](test/DATA-02.md), [DATA-01](test/DATA-01.md), [DATA-03](test/DATA-03.md) |
| 43 ✓ | [APT-01](test/APT-01.md) — Сохранять поздние изменения APT при удалении upd? | [fix/APT-01](fix/APT-01.md), [SEC-04C](test/SEC-04C.md), [APT-02](test/APT-02.md) |

## Правило развилки

В каждом detail есть две обязательные ветви: корректная работа на положительном входе и отказ/сохранение состояния на отрицательном. Если обе проходят, зафиксировать доказательство и закрыть контракт. Если любая не проходит, записать минимальный вход, фактический и ожидаемый результат, вернуть соответствующую fix-вершину исполнителю. Ошибка harness означает «контракт не проверен», а не успех.

Испытывать без доступа к реальным пакетным менеджерам, systemd, firmware и системному `/etc`; внешние команды и пути подменить. После адресных испытаний запустить нужные существующие тесты и затем полный `cargo test` как интеграционный барьер. Проверка на реальных дистрибутивах нужна лишь для совместимости внешних программ, особенно FS-01, APT, zypper, Flatpak и cgroup.

Интеграционный барьер, полный прогон 2026-09-26: `cargo test --offline` — **83 passed / 0 failed** (контрактные `contract_tests` + `tui::tests::test01_*`). Испытания идут через рабочие функции и заглушки команд; системные транзакции APT/pacman, живой systemd и внешняя сеть в целевой ОС отдельно не испытаны. Подробности: [отчёт проверки](REVIEW_2026-09-26.md).

Всего контрактов: 43. Предыдущие исполнители отметили 42 как закрытые; полный интеграционный контракт для каждой отмеченной вершины этим прогоном не доказан. **META-01** закрыт статической проверкой после подтверждения MIT владельцем проекта 2026-09-26.
