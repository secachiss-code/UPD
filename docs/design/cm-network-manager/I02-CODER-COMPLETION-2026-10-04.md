# I02: завершение кодовой части

Sol завершил ревью последовательных T01–T05; рутинную реализацию выполнила Luna xhigh. Astra не использовалась. По актуальному поручению пользователя реальные проверки остаются на установку бинарника, а кодовая очередь продолжает I03.T01.

## Реализовано

`src/profiles/model.rs` содержит строгую schema1, типизированные сущности, ссылки и ownership, поколения, immutable Nodes и Session pins, самостоятельные оси Verification, общий реестр ID с типизированными tombstones, глобальные metadata credentials и whitelist DTO статуса. Разрешения и границы заданы [контрактом модели](I02-MODEL-IMPLEMENTATION-CONTRACT-2026-10-04.md).

`src/profiles/store.rs` предоставляет explicit private Store API: CAS полной revision и Source generation, shared/exclusive bounded advisory locks, immutable blobs, fsync и atomic publication, staged removal с повтором после обрыва, Source update с сохранением архивных pins и NET-only invalidation, model-only host stop. Прерывание после rename не выдаётся за rollback. Посторонние и orphan files не перезаписываются. Sol исправил отдельные lock descriptors для concurrent вызовов одного Store, final unlink checks, initial no-replace publication, duplicate JSON rejection, безопасные ошибки и fsync служебных записей. [Контракт хранения](I02-STORAGE-IMPLEMENTATION-CONTRACT-2026-10-04.md), [контракт ошибок](I02-ERROR-CONTRACT-2026-10-04.md).

Три [примера схемы](i02-examples/README.md) показывают два источника/три приложения/группу, независимый туннель хоста и смену поколения с сохранением прежних pins. Материала credentials в JSON нет. Примеры являются графами модели; сами по себе они не создают готовый Store.

## Проверки и сборка

Подготовлено **35** проверок: 13 модели, 13 Store integration, 6 failure-boundary unit и3 examples. Есть отдельный reader в новом процессе, same/different Store CAS, Busy, повреждённые/чужие записи, immutable Nodes, независимость осей, host isolation и повтор staged removal. Per-Store injection существует только в cfg(test), без production environment switches.

`cargo check --offline --locked --tests`, CLI offline build, COSMIC compile и Python syntax завершились успешно. Все runtime tests **NOT_RUN**. Полные логи, SHA исходников и артефакта: [build manifest](i02-evidence/coder-t05-build-2026-10-04/summary.json). Текущий CLI SHA256: `26478ee0307d1a1d1bffa0c92838d35d68f7f99551bf1d6e768901a103644a89`; предыдущие сборки сохранены в `.audit/build-snapshots/`.

После установки matching binary последовательные synthetic checks запускает `tests/check_i02_regressions.py --run --installed-binary <path> --evidence <new-directory>`. До запуска driver сверяет SHA бинарника и исходников, требует non-root и новый evidence directory; сохраняет stdout/stderr каждого случая, останавливается на первом отказе и отвергает zero-tests PASS. Для I01 regressions с этой сборкой передать тот же manifest через `--manifest`, поскольку исходный I01 default описывает предыдущую сборку. Будущие изменения кода требуют нового matching manifest.

## Открытая приёмка

S01–S13 остаются NOT_RUN до установки; compile не доказывает фактический restart, crash durability и файловую семантику целевой системы. Advisory flock сериализует cooperating clients; абсолютной atomic compare-and-unlink защиты от записи вне протокола тем же UID нет. Initial create failure может оставить private incomplete root для явного разбора. Гарантийные сценарии требуют local Linux filesystem с соответствующей fsync/rename semantics.

Production CLI/UI пока использует прежний singleton VPN. Новый Store API готов для последующей интеграции, он не запускает workers, не меняет routes/services и автоматически не импортирует legacy config. Реальные host/systemd/network проверки I01 и downstream acceptance остаются отдельными открытыми задачами; заявление «VPN исправлен» не сделано.
