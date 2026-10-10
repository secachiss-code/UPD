//! Сводка состояния системы: общая для CLI, TUI и графического интерфейса.

use crate::backend::Backend;

pub mod tunnels;
use crate::common::*;
use crate::mirrors::load_mirror_state;
use crate::{extras, vpn};

pub fn sub_info(i: &vpn::SubInfo) -> String {
    let mut s = String::new();
    if i.total > 0 {
        s += &t!(" · трафик {} из {}", vpn::fmt_bytes_wide(i.used()), fmt_bytes(i.total));
    }
    if i.expire > 0 {
        s += &t!(" · до {}", fmt_time(i.expire).split(' ').next().unwrap_or(""));
    }
    s
}

// ---------- статус ----------

#[derive(Clone, Default)]
pub struct Status {
    pub name: String,
    pub cm: UpdState,
    pub mir: MirrorState,
    pub net_label: String,
    pub pinned: Vec<String>,
    pub last_tx: i64,
    pub reboot: bool,
    pub pending: Vec<String>,
    pub orphans: usize,
    pub cache: u64,
    pub failed: usize,
    pub auto_timer: String,
    pub net_timer: String,
    pub managed: bool,
    pub mirror_note: String,
    pub restart: Restart,
    pub battery: bool,
    pub metered: bool,
    pub free: Option<u64>,
    pub snapshots: String,
    pub vpn: String,
}

pub fn gather_status(b: &dyn Backend) -> Status {
    let net = fingerprint();
    let cache_dirs = b.cache_dirs();
    Status {
        name: b.name(),
        cm: load_json("updates.json"),
        mir: load_mirror_state(),
        net_label: net.label.clone(),
        pinned: b.pinned(),
        last_tx: b.last_upgrade(),
        reboot: reboot_needed(),
        pending: b.pending_configs(),
        orphans: b.orphans().len(),
        cache: dir_size(&cache_dirs),
        failed: failed_units(),
        auto_timer: unit_state("cm-auto.timer"),
        net_timer: unit_state("cm-net.timer"),
        managed: b.mirrors_managed(),
        mirror_note: b.mirror_note(),
        restart: needs_restart(),
        battery: on_battery(),
        metered: metered(&net.dev),
        free: free_space(cache_dirs.first().copied().unwrap_or("/")).ok(),
        snapshots: match (b.auto_snapshots(), extras::snap_tool()) {
            (true, _) => t!("snap-pac (автоматически)").into(),
            (_, extras::SnapTool::Snapper) => t!("snapper (делает cm)").into(),
            (_, extras::SnapTool::Timeshift) => t!("timeshift (делает cm)").into(),
            _ => t!("нет").into(),
        },
        vpn: vpn_line(),
    }
}

/// Короткая строка о VPN без обращения к API (для статуса от имени пользователя).
pub fn vpn_line() -> String {
    let st = vpn::load_state();
    let state = match unit_state(vpn::SERVICE).as_str() {
        "active" => t!("работает"),
        "failed" => t!("ОШИБКА (journalctl -u cm-vpn)"),
        "" => t!("не установлен"),
        _ => t!("выключен"),
    };
    let sub = st.subs.iter().find(|s| s.active).map(|s| format!(" · «{}»", s.name)).unwrap_or_else(|| t!(" · нет подписки").into());
    let core = if st.core_version.is_empty() { String::new() } else { format!(" · mihomo {}", st.core_version) };
    format!("{state}{sub}{core}")
}

pub fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

