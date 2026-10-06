# Аудит unsafe (H.09)

Дата: 2026-10-06. Ревизия: после разбиения `vpn`/`helper` (H.06/H.07).

## Итог

- `unsafe fn` и `unsafe impl` в крейте нет. Все вхождения — блоки вокруг libc-вызовов, `from_raw_fd`, `std::env::set_var`/`remove_var` (в тестах) и `pre_exec`.
- У каждого блока есть комментарий `// SAFETY:`. Gate (`tests/check_audit.sh`) запускает `cargo clippy --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks` по всему крейту `cm` (lib, bin, тесты). Раньше clippy проверял только `src/sources`, `src/profiles` и `src/migration`.
- `unsafe_op_in_unsafe_fn` в Rust 2024 включён по умолчанию, отдельный `deny` не нужен.
- Безопасные обёртки: `common::sys::{euid, uid, egid}` заменили 26 блоков `geteuid`/`getuid`/`getegid`.

## Классы и обоснование

| Класс | Где | Почему безопасно |
|---|---|---|
| Вызовы без указателей: `flock`, `fcntl(F_GETFL/F_SETFL/F_GETFD)`, `kill`/`killpg`, `getpgid`, `getpgrp`, `tgkill(…, 0)`, `gettid` | store, leases, summary, transaction, helper, tui/process, probe | Память не читается и не пишется. Fd заимствован у живого `File`/`OwnedFd` на время вызова. |
| Вызовы с выходным буфером: `getpwuid_r`/`getpwnam_r`, `localtime_r`, `statvfs`, `fstatat`, `waitid`, `getsockopt`, `getrandom`, `recv` | common, helper/auth, vpn/core, store, probe, tui/process | Буферы — живые объекты стека или `Vec` правильного размера. Длина передаётся из `size_of_val` или длины среза. |
| `mem::zeroed` для C-структур (`passwd`, `tm`, `stat`, `statvfs`, `siginfo_t`, `termios`, `sockaddr_un`, `ucred`) | там же | Для plain C struct нулевой битовый шаблон — допустимое значение. |
| Поля union `siginfo_t` (`si_pid`, `si_status`) | probe, tui/process | Структура обнулена до `waitid`, при событии её заполняет ядро. |
| Пути `*at`: `openat`, `mkdirat`, `unlinkat`, `renameat`, `renameat2`, `lchown` | store, transaction | Fd каталога открыт, имя — `CString`, живущий до конца вызова. |
| `from_raw_fd` | common (PTY, dup), store, helper/ops_run | Fd только что вернул syscall, он проверен на `>= 0` и больше никому не принадлежит. `UnixListener::from_raw_fd(3)` — только когда `LISTEN_PID` совпадает с этим процессом и `LISTEN_FDS >= 1`. |
| `pre_exec` | helper/ops, tui/process, tests/audit_contracts | После fork вызываются только async-signal-safe функции (`setsid`, `ioctl(TIOCSCTTY)`, `close_range`), без аллокаций. |
| `libc::signal(SIGPIPE, …)` | main, тест tui/process | Переключение между `SIG_DFL` и прежним обработчиком; обработчик на Rust не ставится. |
| `std::env::set_var`/`remove_var` | только тесты и `contract_fixtures` | Запись в окружение упорядочена через `contract_fixtures::isolation_lock`. |

## Найдено и исправлено

- `common::contract_tests::data04_unreadable_config_is_error_without_clobber` менял `CM_CONF` без `isolation_lock` и мог пересечься с другим тестом, который читает окружение. Тест берёт lock.

## Не сделано (осознанно)

- Перевод на `rustix`/`nix` не делался: сборка `--offline --locked`, новых зависимостей в vendored-кэше нет. Обёртки в `common::sys` оставляют возможность заменить вызовы точечно, когда зависимость появится.
