# Приёмка D17 — 2026-10-01

Автоматическая матрица: 143 PNG, popup 320/360×480, окна 640×480 и
900×700, масштаб 100/200%, RU/EN/AR, обе темы; DE/IT/ZH — smoke.
Loading, denied polkit, длинная ошибка, stale, empty, 500 пакетов,
helper unavailable, starting/disconnected/failed, длинный prompt и 6000 строк
журнала представлены отдельными fixtures. Данные фиктивные; состояния отказа
не требуют настоящих privileged операций.

`cd cosmic && UPD_SNAPSHOTS=/tmp/upd-screens cargo test --offline snapshots`
создаёт всю матрицу. Без переменной обязательны два реальных smoke PNG;
отсутствие renderer, ошибка записи/декодирования, пустой или однородный
результат завершают тест ошибкой. DPI проверяется по физическим размерам PNG.

Сохранённые before/after показывают исправления:

* `*-popup-ru-dark-320-dpi1.png`: нижняя часть popup теперь доступна прокруткой.
* `*-case-long-error-window.png`: длинное предупреждение прокручивается,
  кнопка закрытия и основной интерфейс остаются видимыми.
* `*-case-long-prompt-operation.png`: длинный вопрос прокручивается,
  Да/Нет, журнал и Отменить остаются видимыми на 640×480.
* `*-operation-ar-light-640-dpi1.png`: контроль смешанного RTL/Latin текста,
  светлой темы и footer; произвольного redesign нет.

Проверены визуально указанные пары, также Settings EN/light/200% и VPN AR/dark.
Все PNG матрицы проверены декодером, а не все просмотрены человеком.
В operation fixtures проверяются границы всех фокусируемых контролов,
порядок `focus_next` (механизм Tab) и ровно одно сообщение при Enter.
Это headless проверка дерева виджетов, не системная проверка compositor.

TUI: сохранены 12 текстовых поверхностей, шесть языков × 80×24/60×18.
Тест проверяет видимость раскрытой ошибки HTTP 403 и footer, Esc возвращает
меню. Отдельный PTY fixture проверяет UTF-8 Ж中🙂, resize 80×24→60×18→80×24,
Ctrl+C и завершение с кодом 130. Используется фиктивный shell, terminal пользователя
не переключается в raw mode.

## Ручные проверки в Arch VM / COSMIC

Эти пункты здесь **не выполнены**: доступной Arch VM и средств Wayland-ввода
нет. По указанию пользователя от 2026-10-01 обязательная внешняя приёмка снята
как условие сборки и коммита. Эти проверки не выполнены и не выдаются за автотесты.

- [ ] AUR check/build текущим UID и root→invoking user; чужой UID отклонён.
- [ ] Реальный denied polkit, helper unavailable/restart и reconnect операции.
- [ ] Popup прокручивается до Открыть/Настройки на 320/360 px при 100/200%.
- [ ] Tab/Shift+Tab/Enter по реальному popup и окну; видимый фокус,
      клавиатурный доступ к Да/Нет/Отменить и ошибкам.
- [ ] Все страницы на 640×480, большие системные шрифты, обе темы;
      длинные имена/URL читаются через перенос/прокрутку.
- [ ] RTL с арабским текстом и латинскими именами серверов в compositor.
- [ ] TUI в реальном terminal: resize во время операции, Ctrl+C,
      завершение/ошибка и восстановление echo/raw/alternate-screen.

## Настоящие release-пакеты

На этом окружении выполнены `CARGO_NET_OFFLINE=true ./package.sh` с nfpm 2.47.0 и `python3 tests/verify_real_packages.py` (нужен bsdtar). Все шесть Arch/deb/rpm пакетов версии 0.2.7 прочитаны; metadata и SHA-256 вложенных CLI/COSMIC ELF совпали с immutable inputs в `dist/package-manifest.tsv`. [Сохранённый результат](REAL-PACKAGES.json). Обычный `check_audit.sh` собирает production binaries, но не требует nfpm. UPD_NO_GUI=1 проверен отдельно на настоящем packager: только три CLI packages. Эти проверки не заменяют установку и запуск в disposable VM.

## Повторная приёмка по запросу пользователя

`UPD_FULL_VISUAL=1 UPD_SNAPSHOTS=/tmp/upd-acceptance-current/screens ./tests/check_audit.sh` завершился с кодом 0: 242 теста плюс отдельная полная матрица 143 PNG. [Полный журнал](LATEST-GATE.log), [размеры и SHA-256 снимков](LATEST-VISUAL-MANIFEST.json). Повторно просмотрены long-prompt 640×480, RU dark popup 320×480 и AR light operation 640×480 при 200%. Реальные шесть пакетов повторно прошли `python3 tests/verify_real_packages.py`.

Обнаружен доступный COSMIC/Wayland на host Garuda. `python3 tests/wayland_smoke.py` воспроизводит дополнительный opt-in smoke: пять страниц release GUI, отдельный D-Bus, временные config/state/cache/data, PATH без пакетных менеджеров и отсутствующий helper. Все окна живы через 5 s, отправляют Wayland buffer attach/commit, panic отсутствует. [Результат](WAYLAND-SMOKE.json). Это отрисовка состояния без поддерживаемого backend, а не полноценная интерактивная приёмка. Первая попытка smoke не запустила частный D-Bus из-за изолированного PATH; harness исправлен явным абсолютным путём к dbus-daemon, повторный прогон прошёл.

**Вердикт: локальная автоматическая приёмка пройдена; обязательная внешняя приёмка снята пользователем 2026-10-01 перед сборкой и коммитом.** Нет disposable Arch VM, QEMU/libvirt и /dev/kvm. Пункты ручного списка выше (реальные AUR/polkit/systemd/proxy, popup/keyboard/RTL в compositor и terminal interaction) не выполнены. Снятие требования не означает прохождения этих проверок.
