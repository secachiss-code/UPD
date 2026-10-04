# I02.T04: контракт atomic store

Продолжает уточнённый model contract. Sol проектирует storage; реализация после ревью typed model, строго T01–T03 → T04 → T05. Код не управляет routes/workers и не импортирует singleton legacy config автоматически.

## Layout и locks

Явно переданный private store root, uid текущего euid, mode0700, без symlink ancestors. `state.json` mode0600, `credentials/` mode0700, immutable `<credential_id>.blob` mode0600, постоянный lock-файл mode0600. ID нельзя использовать как произвольный path; root не берётся из legacy fallback. Проверяются file types, owner/mode, symlinks и hardlinks служебных файлов. Private root может быть внутри sticky `/tmp` для fixture. Blob/snapshot reads bounded; graph reads проверяют наличие, размеры и SHA всех зарегистрированных blobs, а private material API повторно проверяет запрошенный blob. Secret bytes не возвращаются в graph/public status.

Shared lock для чтения, exclusive lock для публикации/удаления. Writers сверяют ожидаемую revision под lock; победитель публикует одну полную revision+1, второй получает Conflict(expected,current). Bounded lock wait и IO имеют самостоятельные ошибки; default2s, configurable timeout ограничен30s, deadline проверяется также после EINTR. Каждая операция самостоятельно открывает permanent lock-файл: reuse одного fd или dup/try_clone не сериализует concurrent вызовы одного Store, поскольку flock относится к open file description. [Linux man-pages: flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html). CAS проверяется и для двух вызовов на одном Arc<Store>, и для разных Store handles.

Root create/init — отдельный явный метод, не side effect публичного статуса. Ошибка посреди initial create может оставить private incomplete root: повторная initialize его не переписывает, требуется явный разбор. Store не считает такой каталог валидным snapshot.

## Publication

1. Прочитать и validate текущий graph; проверить expected revision и transition invariants.
2. Validate candidate graph и metadata перед любых new blob writes. Новый immutable credential_ref должен иметь material с matching size/SHA; существующий ref не перезаписывается. Graph не содержит byte material.
3. Создать только новые blobs с O_EXCL/O_NOFOLLOW mode0600, fsync file и credentials directory. При ошибке/обрыве blobs остаются private unpublished orphan, старый graph и используемые blobs не изменяются. Идентификатор, занятый orphan file, нельзя перезаписать другими bytes: нужен новый ref или отдельный ручной разбор. Это не новый Source generation.
4. Serialize candidate state в unique O_EXCL temp, fsync, atomic rename state.json, fsync root. Reader видит только старый или новый graph целиком. Не обрабатывать rename-success+fsync-failure как доказанный rollback; вернуть indeterminate durability с новым state при следующем чтении.

При initial publication state ещё отсутствует: используется Linux renameat2(RENAME_NOREPLACE), без fallback на overwriting rename; неожиданно появившийся target сохраняется. [Linux man-pages: rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html). Проверяемые гарантийные сценарии относятся к local Linux filesystem с working file/directory fsync и rename semantics; реальный filesystem проверяется при установке.

Store не копирует secret bytes в Debug/Display/public status/errors. Передавать только refs/metadata, error code, safe ids/revisions. Оставшиеся unknown/orphan files не удаляются по prefix/name mask автоматически.

## Source updates и pins

API Source update принимает expected graph revision/Source generation. Новый Source generation целиком содержит current_node_ids, новые Nodes и credential refs. Retained old Nodes остаются для profile/activeSession pins. Исчезнувший digest pinned Node блокирует NET соответствующей Session с timestamp, не заменяет node/profile/tunnelgeneration. Generation другие оси не обновляет сама собой. Source kind/id не меняются задним числом.

## Removal

begin_removal(expected_revision, plan_id, source_id) проверяет live references, сохраняет pending removal record и убирает разрешённые source/nodes из graph; issued_ids registry сохраняется. Пока plan pending, данные material не доступны как live Source, но blobs могут оставаться. Исторические endedSession pins сохраняются как history и не мешают удалению.

finish_removal(plan_id) под lock проверяет все планируемые blob files owner/type/mode/digests до удаления; missing file допустим только у уже durable pending plan. Отказ на foreign/corrupt blob не удаляет посторонний файл ради успешного удаления. После unlink fsync credentials directory, затем durable completion snapshot убирает unused credential metadata и отмечает plan complete. Если процесс прерван между удалениями и completion, pending plan остаётся читаемым; повтор продолжает и не создаёт blob/source. Уже complete plan идемпотентен. Live refs, появившиеся во время pending, запрещаются model validator.

Перед первым unlink проверяется весь набор; непосредственно перед каждым unlink повторно сверяются pathname inode и stamp удерживаемых descriptors. Гарантия сериализации относится к cooperating Store clients, соблюдающим advisory flock; внепротокольная запись тем же UID не имеет абсолютной атомарной защиты между последней проверкой pathname и unlink. [Linux man-pages: flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html).

Обычный remove_source удерживает один exclusive lock для обеих стадий, но S06 отдельно наблюдает pending. Никакого удаления использованного Source с выбором другого случайного узла.

## Ошибки и границы API

Ошибки различают: InvalidModel, UnsupportedSchema/CorruptState, Conflict(expected,current), Busy, UnsafeFilesystem, CredentialMismatch, CredentialOccupied, MissingCredential, SourceInUse(safe reference ids), PendingRemoval, NotFound, Io(operation code), DurabilityIndeterminate. Raw serde error, response body, blob bytes, URL, argv/environment values в Display/Debug ошибки не копируются. Debug material input/result показывает только redacted marker и длину, публичный DTO строится по whitelist.

Writer перед snapshot rename повторно проверяет identity текущего state/lock и private directories; неожиданная подмена останавливает публикацию. Служебный lock никогда не удаляется в unlock. Bounded lock ожидание использует monotonic time и не смешивает contention с IO ошибкой. State temp cleanup разрешён только для собственного созданного inode; чужой temp по имени/префиксу не трогать. Обрыв после rename не снимается стиранием опубликованного snapshot.

Обновление Source/удаление выполняются поверх одной locked revision, а не sequence нескольких независимых graph commits. Pending removal record обязан перечислять точные credential ids и metadata для проверки остатка после restart. Credential, который ещё используется другим Source/Node, в removal plan не включается. Задвоенный plan id с другим source — Conflict, завершённый plan id для того же source — идемпотентный успех. Другой план не заменяет уже pending plan того же Source.

## T05 подготовка

Synthetic tests по S01–S13: двеSources/триapps/однаgroup, safe public metadata, crash/failure publication boundaries, CAS concurrency, immutableNode replacement, active/endedSession removal, pendingRemoval restart/retry, missingNode NET-only invalidation, hostStop isolation, axes independence, foreignschema rejection и safe filesystem negatives. Сейчас тесты только подготовить и compile. Runtime — при установке, ни одному Sxx не приписывать PASS от чтения или компиляции.
