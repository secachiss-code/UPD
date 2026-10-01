# Ресурсные/security gates D18 — 2026-10-01

Используются только временные файлы, Unix sockets и фиктивные исполняемые
программы. Установленные polkit policy, systemd и пакетные менеджеры не проверены:
для них нужен disposable Arch VM snapshot, см. ручной чеклист README.md.

## Процессный ресурсный gate

`cargo test --offline --test audit_contracts helper_overload -- --nocapture`

Семь серий: три прогревочных и четыре измеряемых. Каждая серия включает
32 частичных запроса до авторизации (проверяется абсолютный deadline),
100 attach/drop, fake child с burst 8000×1025 байт, slow reader с SO_RCVBUF=4096,
отказ второму start и повторный cancel. Читаются task/fd/status и children
всех потоков из /proc helper. У каждой проверки конечный deadline;
готовность burst подтверждается файлом-барьером, завершение — Status.

Бюджеты: до 8 дополнительных connection threads одного UID; до 10 с runner
и reader; до 64 дополнительных FD в работающей серии; менее 64 MiB RSS.
За 4 секунды после каждой серии FD/threads должны точно вернуться к baseline,
список children должен стать пустым. VmHWM после прогрева может вырасти
не более чем на 16 MiB; возврат allocator cache ОС не требуется.

Наблюдение первого успешного запуска: baseline 1 thread / 4 FD;
прогретый peak 18748 KiB; следующие четыре peak:
18748 / 18748 / 18748 / 18748 KiB. Тест занял 26.12 секунды.
Это результат конкретного запуска, не обещание одинакового RSS на каждой ОС.

## Связанные gates

- `delayed_fake_authorizer_denies_foreign_uid_and_cannot_cancel_next_operation`:
  handle вызывает внешний fake pkcheck с PID/start time/UID; чужой UID получает
  denied. При разрешении после файлового барьера старая операция уже заменена:
  cancel возвращает StaleOperation, runner не получает команду; callback A
  не меняет журнал B. Peer здесь синтетический, policy не установлена.
- `helper_restart_does_not_fabricate_success_for_untracked_operation`:
  crash локального helper закрывает stream без выдуманного Exit; fake child
  остаётся вне helper, новый Status не выдумывает result, старый cancel stale.
- Cosmic `restarted_helper_and_delayed_callbacks_never_report_false_success`:
  пустой Status после disconnect сохраняет неизвестный исход; 100 задержанных
  callbacks/frames A и старого поколения не завершают B.
- `concurrent_cli_language_and_helper_field_preserve_separate_state_files`:
  20 синхронизированных записей CLI lang и helper keep сохраняют оба поля;
  посторонний state-файл побайтно неизменен.
- Уже существующие gates проверяют чужого owner после authorization barrier,
  stale/replayed prompt, секрет без echo/replay, slowloris/write deadlines,
  subscriber byte/event quotas и global/per-UID guards при unwind/spawn failure.
- Конкурентный CLI/helper VPN YAML и SavedButNotApplied проверяются отдельными
  fixtures с fake systemctl/gsettings, без изменения настоящего VPN/proxy.
