//! Короткий снимок состояния для панели и всплывающего окна: только чтение общедоступных файлов
//! состояния, без сети, root и тяжёлых подсчётов (кэш, сироты). Приоритеты значка и заголовка —
//! чистые функции, чтобы их поведение было проверяемым.

use crate::backend::Backend;
use crate::common::*;
use crate::helper::{OperationId, PromptId};
use crate::mirrors::load_mirror_state;
use crate::vpn;
use serde::{Deserialize, Serialize};

/// Проверка старше этого срока считается устаревшей (фоновая проверка идёт раз в 6 ч).
pub const STALE_AFTER: i64 = 24 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MirrorSummary {
    /// зеркалами управляет cm (иначе — сам пакетный менеджер или дистрибутив)
    pub managed: bool,
    pub pinned: usize,
    pub checked: i64,
    /// сколько закреплённых зеркал не ответило при последнем замере
    pub failing: usize,
    /// зеркала выбраны, но применить их не удалось
    pub apply_error: String,
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct VpnSummary {
    /// служба cm-vpn установлена
    pub installed: bool,
    pub active: bool,
    pub failed: bool,
    pub has_subs: bool,
    /// название активной подписки
    pub subscription: String,
    pub event: String,
    #[serde(default)]
    pub error: String,
}

/// Что доступно на этой системе: недоступное интерфейс скрывает.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Features {
    pub backend: String,
    pub flatpak: bool,
    pub aur: bool,
    pub firmware: bool,
    pub news: bool,
    pub merge: bool,
    pub vpn: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// время последней проверки обновлений (0 — не было)
    pub checked: i64,
    pub packages: Vec<String>,
    pub flatpak: Vec<String>,
    pub firmware: Vec<String>,
    pub downloaded: bool,
    pub download_size: Option<u64>,
    /// первая ошибка проверки (пакеты, Flatpak, прошивки); пусто — проверка прошла
    pub error: String,
    /// предупреждение, не мешающее установке (мало места, пропуск фоновой проверки)
    pub warning: String,
    pub news: Vec<News>,
    pub reboot: bool,
    pub pending_configs: usize,
    pub mirrors: MirrorSummary,
    pub vpn: VpnSummary,
    pub features: Features,
    /// когда собран снимок
    pub taken: i64,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.packages.len() + self.flatpak.len() + self.firmware.len()
    }

    /// Данные старые или проверки ещё не было — число на панели нельзя подавать как актуальное.
    pub fn stale(&self) -> bool {
        self.checked == 0 || self.taken.saturating_sub(self.checked) > STALE_AFTER
    }

    /// Зеркала требуют внимания (строка во всплывающем окне показывается только тогда).
    pub fn mirror_problem(&self) -> bool {
        self.mirrors.managed && (self.mirrors.failing > 0 || !self.mirrors.apply_error.is_empty())
    }
}

/// Состояние долгой операции, как его видит интерфейс.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct OpStatus {
    pub running: bool,
    /// команда cm: update, check, clean…
    pub command: String,
    /// этап «[3/6] Загрузка» → (3, 6, «Загрузка»)
    pub stage: Option<(u32, u32, String)>,
    pub started: i64,
    /// код завершения последней операции и когда она закончилась
    pub last_exit: Option<i32>,
    pub last_command: String,
    pub finished: i64,
    /// операция ждёт ответа да/нет
    pub waiting: bool,
    /// идентификатор текущей или последней операции helper
    pub operation_id: Option<OperationId>,
    /// id вопроса, если команда ждёт однократный ответ
    pub prompt_id: Option<PromptId>,
}

/// Значок на панели: из нескольких состояний показывается одно — с наивысшим приоритетом.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Badge {
    /// идёт установка (этап n из m, если известен)
    Busy(Option<(u32, u32)>),
    Waiting,
    Error,
    Reboot,
    Updates(usize),
    /// число с прошлой проверки, которая уже устарела
    Stale(usize),
    Unverified,
    Idle,
}

pub fn badge(s: &Summary, op: &OpStatus) -> Badge {
    if op.running {
        if op.waiting { return Badge::Waiting; }
        return Badge::Busy(op.stage.as_ref().map(|(n, m, _)| (*n, *m)));
    }
    if !s.error.is_empty() || !s.vpn.error.is_empty() {
        return Badge::Error;
    }
    if s.reboot {
        return Badge::Reboot;
    }
    let n = s.total();
    match (n, s.stale()) {
        (0, false) => Badge::Idle,
        (0, true) => Badge::Unverified,
        (n, false) => Badge::Updates(n),
        (n, true) => Badge::Stale(n),
    }
}

/// Заголовок всплывающего окна.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Headline {
    Checking,
    Installing { command: String, stage: Option<(u32, u32, String)> },
    CheckFailed(String),
    NoData,
    Updates(usize),
    UpToDate,
}

pub fn headline(s: &Summary, op: &OpStatus) -> Headline {
    if op.running {
        return if op.command == "check" {
            Headline::Checking
        } else {
            Headline::Installing { command: op.command.clone(), stage: op.stage.clone() }
        };
    }
    if !s.error.is_empty() {
        return Headline::CheckFailed(s.error.clone());
    }
    if s.checked == 0 {
        return Headline::NoData;
    }
    match s.total() {
        0 => Headline::UpToDate,
        n => Headline::Updates(n),
    }
}

impl Headline {
    pub fn title(&self) -> String {
        match self {
            Headline::Checking => t!("Проверка…").into(),
            Headline::Installing { command, stage } => {
                let what = op_title(command);
                match stage {
                    Some((n, m, name)) => t!("{0} · этап {1} из {2}: {3}", what, n, m, name),
                    None => what.into(),
                }
            }
            Headline::CheckFailed(_) => t!("Не удалось проверить").into(),
            Headline::NoData => t!("Обновления ещё не проверялись").into(),
            Headline::Updates(n) => t!("Доступно обновлений: {0}", n),
            Headline::UpToDate => t!("Система актуальна").into(),
        }
    }
}

/// Строка под заголовком: когда проверено и насколько свежи данные.
pub fn checked_line(s: &Summary) -> String {
    if s.checked == 0 {
        return t!("Проверки ещё не было").into();
    }
    if s.stale() {
        t!("Данные от {0} — могли устареть", fmt_time(s.checked))
    } else {
        t!("Проверено {0}", fmt_ago(s.checked))
    }
}

/// Название операции для заголовков и уведомлений.
pub fn op_title(command: &str) -> &'static str {
    match command.split_whitespace().next().unwrap_or("") {
        "update" => t!("Обновление"),
        "check" => t!("Проверка обновлений"),
        "clean" => t!("Очистка"),
        "restart" => t!("Перезапуск служб"),
        "mirrors" => t!("Зеркала"),
        "vpn" => "VPN",
        "aur" => "AUR",
        "news" => t!("Новости Arch"),
        _ => "cm",
    }
}

fn state_error(u: &UpdState) -> String {
    [u.error.as_str(), u.flatpak_error.as_str(), u.firmware_error.as_str()]
        .into_iter()
        .find(|e| !e.is_empty() && !e.starts_with(t!("мало места")))
        .unwrap_or("")
        .to_string()
}

/// Сводка из общедоступных файлов состояния; можно вызывать от обычного пользователя.
pub fn gather(b: &dyn Backend) -> Summary {
    let u: UpdState = load_json("updates.json");
    let m = load_mirror_state();
    let v = vpn::load_state();
    let pinned = if b.mirrors_managed() { b.pinned() } else { vec![] };
    let failing = m.results.iter().filter(|p| !p.ok && pinned.iter().any(|x| x == &p.url)).count();
    let unit = unit_state(vpn::SERVICE);
    let warning = if u.error.starts_with(t!("мало места")) {
        u.error.clone()
    } else {
        u.space_check_error.clone().unwrap_or_else(|| u.skipped.clone())
    };
    Summary {
        checked: u.checked,
        error: state_error(&u),
        warning,
        packages: u.list,
        flatpak: u.flatpak,
        firmware: u.firmware,
        downloaded: u.downloaded,
        download_size: u.download_size,
        news: u.news,
        // процессы со старыми библиотеками от имени пользователя не видны целиком — их показывает помощник
        reboot: reboot_needed(),
        pending_configs: b.pending_configs().len(),
        mirrors: MirrorSummary {
            managed: b.mirrors_managed(),
            pinned: pinned.len(),
            checked: m.checked,
            failing,
            apply_error: m.apply_error,
            note: if b.mirrors_managed() { String::new() } else { b.mirror_note() },
        },
        vpn: VpnSummary {
            installed: !unit.is_empty(),
            active: unit == "active",
            failed: unit == "failed",
            has_subs: !v.subs.is_empty(),
            subscription: v.subs.iter().find(|s| s.active).map(|s| s.name.clone()).unwrap_or_default(),
            error: v.last_failure.as_ref().filter(|e| e.resolved.is_none()).map(|e| e.reason.clone()).unwrap_or_default(),
            event: v.event,
        },
        features: Features {
            backend: b.name(),
            flatpak: have("flatpak"),
            aur: b.aur(),
            firmware: have("fwupdmgr"),
            news: b.arch_news(),
            // pacdiff есть только у pacman (у него же AUR)
            merge: b.aur(),
            vpn: systemd(),
        },
        taken: now(),
    }
}

/// Блокировка живого TUI: после аварийного выхода ядро освобождает её само.
/// Отдельный файл на процесс позволяет открыть несколько терминалов.
pub struct TuiSession {
    _file: std::fs::File,
    path: std::path::PathBuf,
}
impl TuiSession {
    pub fn register() -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        let directory = std::path::PathBuf::from(state_dir()).join("tui");
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))?;
        let path = directory.join(format!("{}.lock", std::process::id()));
        let file = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        // SAFETY: flock on an open fd borrowed for the call; it accesses no memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _file: file, path })
    }
}
impl Drop for TuiSession {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

pub fn tui_running() -> bool {
    use std::os::fd::AsRawFd;
    let directory = std::path::PathBuf::from(state_dir()).join("tui");
    let Ok(entries) = std::fs::read_dir(directory) else { return false; };
    entries.flatten().any(|entry| {
        let Ok(file) = std::fs::File::open(entry.path()) else { return false; };
        // SAFETY: flock on an open fd borrowed for the call; it accesses no memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
            false
        } else {
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
        }
    })
}

/// Файл с pid работающего апплета: пока он жив, `cm notify` не дублирует его уведомления.
pub fn applet_pid_file() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(|d| std::path::PathBuf::from(d).join("cm-applet.pid"))
}

pub fn applet_running() -> bool {
    let Some(pid) = applet_pid_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| s.trim().parse::<u32>().ok()) else {
        return false;
    };
    // pid мог достаться другому процессу: сверяем имя
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim().starts_with("cm-cosmic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_indicator_tracks_live_session_and_ignores_stale_files() {
        use crate::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        let _isolation = isolation_lock();
        let dir = TempDirGuard::new("tui-indicator").unwrap();
        let mut env = EnvGuard::new(); env.set("CM_STATE_DIR", dir.path());
        assert!(!tui_running());
        let session = TuiSession::register().unwrap();
        assert!(tui_running());
        std::fs::write(dir.path().join("tui/stale.lock"), "").unwrap();
        drop(session);
        assert!(!tui_running());
    }

    fn fresh(n: usize) -> Summary {
        Summary { checked: 1000, taken: 1100, packages: (0..n).map(|i| format!("p{i}")).collect(), ..Default::default() }
    }

    #[test]
    fn badge_priority_follows_design_table() {
        let idle = OpStatus::default();
        let busy = OpStatus { running: true, command: "update".into(), stage: Some((3, 6, "x".into())), ..Default::default() };
        let mut s = fresh(4);
        s.error = "boom".into();
        s.reboot = true;
        assert_eq!(badge(&s, &busy), Badge::Busy(Some((3, 6))), "установка важнее всего");
        assert_eq!(badge(&s, &idle), Badge::Error, "ошибка важнее перезагрузки");
        s.error.clear();
        assert_eq!(badge(&s, &idle), Badge::Reboot, "перезагрузка важнее числа");
        s.reboot = false;
        assert_eq!(badge(&s, &idle), Badge::Updates(4));
        s.taken = s.checked + STALE_AFTER + 1;
        assert_eq!(badge(&s, &idle), Badge::Stale(4), "старое число показывается приглушённым");
        assert_eq!(badge(&fresh(0), &idle), Badge::Idle);
    }

    #[test]
    fn stale_data_never_reads_as_zero_updates() {
        let s = Summary::default();
        assert!(s.stale());
        assert_eq!(headline(&s, &OpStatus::default()), Headline::NoData);
        assert_ne!(headline(&s, &OpStatus::default()), Headline::UpToDate);
        assert_eq!(badge(&s, &OpStatus::default()), Badge::Unverified);
        let mut old = fresh(0);
        old.taken = old.checked + STALE_AFTER + 1;
        assert_eq!(badge(&old, &OpStatus::default()), Badge::Unverified);
        let waiting = OpStatus { running: true, waiting: true, ..Default::default() };
        assert_eq!(badge(&old, &waiting), Badge::Waiting);
    }

    #[test]
    fn headline_states() {
        let check = OpStatus { running: true, command: "check".into(), ..Default::default() };
        assert_eq!(headline(&fresh(2), &check), Headline::Checking);
        let cm = OpStatus { running: true, command: "update".into(), ..Default::default() };
        assert!(matches!(headline(&fresh(2), &cm), Headline::Installing { .. }));
        assert_eq!(headline(&fresh(2), &OpStatus::default()), Headline::Updates(2));
        assert_eq!(headline(&fresh(0), &OpStatus::default()), Headline::UpToDate);
        let mut f = fresh(2);
        f.error = "нет сети".into();
        assert_eq!(headline(&f, &OpStatus::default()), Headline::CheckFailed("нет сети".into()));
    }

    #[test]
    fn low_space_is_a_warning_not_a_check_failure() {
        let u = UpdState { error: format!("{}: 1 ГБ", t!("мало места")), ..Default::default() };
        assert_eq!(state_error(&u), "");
        let u = UpdState { flatpak_error: "x".into(), ..Default::default() };
        assert_eq!(state_error(&u), "x");
    }

    #[test]
    fn mirror_problem_only_when_managed() {
        let mut s = fresh(0);
        s.mirrors.failing = 2;
        assert!(!s.mirror_problem());
        s.mirrors.managed = true;
        assert!(s.mirror_problem());
    }
}
