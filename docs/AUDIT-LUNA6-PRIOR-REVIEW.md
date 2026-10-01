# Ревью изменений исходной базы после выполнения локальной части DAG

Дата: 2026-10-01. Проверена база HEAD 31d3b7c, в том числе ввод COSMIC/helper
из f8dbe30 и прежние CLI/TUI контракты. Сопоставлялись git show HEAD и текущий
код, архив B01–B25 сохранён. Дополнительная проверка выполнялась после D19,
как поручил пользователь. Это ревью путей исполнения приложения, не аудит
всех зависимостей/CVE и не проверка установленной системы.

| Находка в прежнем коде | Исправление | Проверка |
| --- | --- | --- |
| R1: TUI выбрасывал reader JoinHandle; spawn reader failure оставлял Child без kill/wait. Drop только сигналил, а после try_wait считал отсутствующий /proc/PGID доказательством владения группой. B06 покрывал только часть lifecycle. | ProcessSession владеет reader/Child, stop+wake работает при полной bounded output queue. Выход наблюдается WNOWAIT, PID зарезервирован до cleanup группы; drain ограничен прежней 1 s, затем kill/reap/join до Finished. Drop и rollback тоже reap. | reader-spawn failure и полный output queue fixtures; B06 descendant/SIGPIPE/reused PID, UTF-8/resize/Ctrl+C passed targeted |
| R2: TUI send_key использовал blocking write_all; заполненный PTY мог остановить UI. | Master nonblocking, reader учитывает WouldBlock/Interrupted; input poll/write имеет deadline 100 ms, ошибка не включает содержимое ввода. | raw/no-echo child не читает stdin; 1 MiB input истекает, poll доступен; queue-full/drop и Ctrl+C passed targeted |
| R3: COSMIC notify создавал новый OS-thread на каждое сообщение с бесконечным output ожиданием notify-send --action. CLI notify тоже имел неограниченный status и отмечал failure как seen. | Максимум 4 notification workers, RAII slot, stdout 256 B/stderr 8 KiB, process group/cleanup и deadline 5 min для действия; quota/spawn/exit/timeout errors видны через launch errors. CLI использует 30 s capture и отмечает seen только после success, позволяя retry. | quota/release, stalled/noisy command, action whitelist, failed notification retry passed targeted |
| R4: ручной install CLI/GUI использовал общий `<bin>.new` для copy+rename. Параллельные установки могли обрезать/переименовать один temp. Системный каталог защищён; произвольная root-запись не заявляется. | Оба install пути используют проверенный unique/exclusive atomic_write с fsync и mode 0755. Файл бинарника читается в память однократно; это oneshot install, не фоновый GUI worker. | 8 конкурентных install в temp-каталоге сохраняют целый файл/mode и не оставляют temp; существующие atomic failure tests |

Продолжение с настоящими release-сборками и nfpm выявило дополнительные проблемы:

| Находка | Исправление | Проверка |
| --- | --- | --- |
| R5: production COSMIC Subscription::map захватывал generation; unit-test сборка не инстанцировала этот путь и скрывала E0080. | Generation передаётся через Subscription::with; map не захватывает окружение. Gate теперь собирает обычные CLI/GUI binaries. | Настоящая release-сборка musl/GNU и operation regressions |
| R6: nfpm tree в корень / создавал пустой archive entry CLI Arch-пакета, отвергаемый libarchive. | Отдельные trees /usr/share и /etc вместо корневого tree. | Шесть настоящих пакетов читаются bsdtar; tests/verify_real_packages.py проверяет metadata и SHA-256 ELF |
| R7: connection permit освобождался до фактического выхода OS-thread; при musl даже join опережает удаление kernel task. | Listener хранит JoinHandle/TID и сохраняет лимит до join и исчезновения task. | Строгий лимит в семи ресурсных сериях; Threads ядра вместо несогласованного обхода каталога |

Restart fixture теперь игнорирует PTY SIGHUP и живёт 60 s: тест намеренно моделирует переживший helper процесс, независимо от закрытия master и нагрузки компиляции. ProcessGuard по-прежнему удаляет его после проверки.

Просмотрены связанные вызовы helper/probes, launcher/notifications, Summary/Status,
установка и генерация policy/units, конфигурационные транзакции. Исправления
F01–F20 из DAG проверялись своими fixtures ранее; новые находки приведены выше,
без повторной маркировки архива B как списка открытых задач.

## Итоговая верификация

Воспроизводимая команда: `./tests/check_audit.sh`. Она использует --locked
--offline и явные musl/GNU targets, core/Cosmic suites, build/package stubs,
синтаксис shell и форматирование новых модулей. Targeted tests и полный финальный gate прошли: 149 lib + 41 CLI + 24 integration,
24 Cosmic, 4 packaging fixtures (242 теста). Детали в [PROGRESS](AUDIT-LUNA6-PROGRESS.md).

Реальные polkit/systemd/VPN/AUR/package install и compositor keyboard не
выполнялись на host: план разрешает их только в disposable VM. Чеклист и
не закрытые критерии F11/F17/F20 остаются в
[FINDINGS](AUDIT-LUNA6-FINDINGS.md) и
[acceptance](audit-luna6-acceptance/README.md).
