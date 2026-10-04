//! Frozen UPD 0.2.7 generators from commit 07ef2381fb962fe74958c504a896c584f6a4e4be.
//! Exact ownership references; never run an installed legacy executable.
mod vpn {
    pub const SERVICE: &str = "upd-vpn.service";
}
mod helper {
    pub const ACTION_STATUS: &str = "io.github.upd.status";
    pub const ACTION_CHECK: &str = "io.github.upd.check";
    pub const ACTION_VPN: &str = "io.github.upd.vpn";
    pub const ACTION_MANAGE: &str = "io.github.upd.manage";
}
pub const CLI_SHA256: &str = "58520fbe70796d5bd018d56ad4ebbb120cea92c8edcb2cb52f5983ea694b14c2";
pub const GUI_SHA256: &str = "2ded86911b6f895fbc0f866ebac09b62fd6138f448b8c7b8a332938f98ab2ad5";
pub fn system_units(bin: &str, watch: Option<&str>) -> Vec<(&'static str, String)> {
    let mut u = vec![
        (
            "upd-auto.service",
            format!("[Unit]\nDescription=upd: network change, mirrors, VPN, update check and prefetch\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} auto\nNice=10\nIOSchedulingClass=idle\n"),
        ),
        (
            "upd-auto.timer",
            "[Unit]\nDescription=upd: periodic update check\n\n[Timer]\nOnBootSec=10min\nOnUnitActiveSec=6h\nRandomizedDelaySec=10min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            "upd-net.service",
            format!("[Unit]\nDescription=upd: mirror selection on network change\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} net\nNice=10\n"),
        ),
        (
            "upd-net.timer",
            "[Unit]\nDescription=upd: network change check (cheap, no network requests)\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec=15min\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            vpn::SERVICE,
            format!(
                "[Unit]\nDescription=upd: VPN (mihomo core)\nWants=network-online.target\nAfter=network-online.target\n\
                 StartLimitIntervalSec=10min\nStartLimitBurst=5\n\n[Service]\nType=simple\n\
                 ExecStartPre={bin} vpn prepare\nExecStart=/var/lib/upd/vpn/bin/mihomo -d /var/lib/upd/vpn -f /var/lib/upd/vpn/config.yaml\n\
                 Restart=on-failure\nRestartSec=5\nTimeoutStartSec=5min\nLimitNOFILE=1048576\n\
                 UMask=0077\nNoNewPrivileges=yes\n\
                 CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
                 AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
                 ProtectSystem=strict\nReadWritePaths=-/var/lib/upd/vpn\nProtectHome=yes\nPrivateTmp=yes\n\
                 ProtectKernelModules=yes\nProtectControlGroups=yes\n\n[Install]\nWantedBy=multi-user.target\n"
            ),
        ),
    ];
    // помощник графического интерфейса: запускается по обращению к сокету, права проверяет через polkit
    u.push((
        "upd-helper.socket",
        "[Unit]\nDescription=upd: helper socket for the graphical interface\n\n[Socket]\nListenStream=/run/upd/helper.sock\nSocketMode=0666\nDirectoryMode=0755\nRemoveOnStop=yes\n\n[Install]\nWantedBy=sockets.target\n".to_string(),
    ));
    u.push((
        "upd-helper.service",
        format!("[Unit]\nDescription=upd: privileged helper for the graphical interface\nRequires=upd-helper.socket\nAfter=upd-helper.socket\n\n[Service]\nType=simple\nExecStart={bin} helper\nKillMode=process\n"),
    ));
    if let Some(w) = watch {
        u.push((
            "upd-mirrors.path",
            format!("[Unit]\nDescription=upd: restore pinned mirrors if the list is overwritten\n\n[Path]\nPathChanged={w}\nUnit=upd-mirrors-apply.service\n\n[Install]\nWantedBy=paths.target\n"),
        ));
        u.push((
            "upd-mirrors-apply.service",
            format!("[Unit]\nDescription=upd: put pinned mirrors back on top\n\n[Service]\nType=oneshot\nExecStart={bin} mirrors apply\n"),
        ));
    }
    u
}

pub fn user_units(bin: &str) -> Vec<(&'static str, String)> {
    vec![
        ("upd-notify.service", format!("[Unit]\nDescription=upd: notifications\n\n[Service]\nType=oneshot\nExecStart={bin} notify\n")),
        ("upd-notify.timer", "[Unit]\nDescription=upd: notifications\n\n[Timer]\nOnActiveSec=3min\nOnUnitActiveSec=30min\n\n[Install]\nWantedBy=timers.target\n".to_string()),
    ]
}

/// Действия polkit для помощника: чтение состояния без пароля в активном сеансе, изменения — с паролем администратора.
pub fn polkit_policy() -> String {
    let msg = |en: &str, tr: &[(&str, &str)]| {
        let mut s = format!("    <message>{en}</message>\n");
        for (l, m) in tr {
            s += &format!("    <message xml:lang=\"{l}\">{m}</message>\n");
        }
        s
    };
    let actions = [
        (
            helper::ACTION_STATUS,
            "yes",
            msg(
                "Read the update and VPN status",
                &[
                    ("ru", "Просмотр состояния обновлений и VPN"),
                    ("de", "Status von Updates und VPN lesen"),
                    ("it", "Leggere lo stato di aggiornamenti e VPN"),
                    ("zh", "读取更新和 VPN 状态"),
                    ("ar", "قراءة حالة التحديثات والشبكة الافتراضية"),
                ],
            ),
        ),
        (
            helper::ACTION_CHECK,
            "yes",
            msg(
                "Check for updates",
                &[
                    ("ru", "Проверка обновлений"),
                    ("de", "Nach Updates suchen"),
                    ("it", "Cercare aggiornamenti"),
                    ("zh", "检查更新"),
                    ("ar", "البحث عن تحديثات"),
                ],
            ),
        ),
        (
            helper::ACTION_VPN,
            "yes",
            msg(
                "Turn the VPN on or off and choose a server",
                &[
                    ("ru", "Включение и выключение VPN, выбор сервера"),
                    ("de", "VPN ein- oder ausschalten und Server wählen"),
                    ("it", "Attivare o disattivare la VPN e scegliere il server"),
                    ("zh", "开关 VPN 并选择服务器"),
                    ("ar", "تشغيل الشبكة الافتراضية أو إيقافها واختيار الخادم"),
                ],
            ),
        ),
        (
            helper::ACTION_MANAGE,
            "auth_admin_keep",
            msg(
                "Install updates and manage mirrors and VPN",
                &[
                    ("ru", "Установка обновлений, управление зеркалами и VPN"),
                    (
                        "de",
                        "Updates installieren sowie Spiegelserver und VPN verwalten",
                    ),
                    ("it", "Installare aggiornamenti e gestire mirror e VPN"),
                    ("zh", "安装更新并管理镜像和 VPN"),
                    ("ar", "تثبيت التحديثات وإدارة المرايا والشبكة الافتراضية"),
                ],
            ),
        ),
    ];
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE policyconfig PUBLIC \"-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN\"\n \"http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd\">\n<!-- Managed by upd -->\n<policyconfig>\n  <vendor>upd</vendor>\n  <icon_name>system-software-update</icon_name>\n",
    );
    for (id, active, message) in actions {
        // вне активного сеанса (ssh, другой пользователь за экраном) — только с паролем администратора
        let other = if active == "yes" {
            "auth_admin"
        } else {
            active.trim_end_matches("_keep")
        };
        s += &format!(
            "  <action id=\"{id}\">\n{message}    <defaults>\n      <allow_any>{other}</allow_any>\n      <allow_inactive>{other}</allow_inactive>\n      <allow_active>{active}</allow_active>\n    </defaults>\n  </action>\n"
        );
    }
    s + "</policyconfig>\n"
}

pub fn pacman_hook(bin: &str) -> String {
    format!(
        "# upd: reconcile the list of available updates after any transaction (offline)\n[Trigger]\nOperation = Install\nOperation = Upgrade\nOperation = Remove\nType = Package\nTarget = *\n\n[Action]\nDescription = upd: reconciling the update list...\nWhen = PostTransaction\nExec = {bin} reconcile\n"
    )
}

pub fn apt_hook(bin: &str) -> String {
    format!(
        "// upd: reconcile the list of available updates after any install (offline)\nDPkg::Post-Invoke {{ \"{bin} reconcile >/dev/null 2>&1 || true\"; }};\n"
    )
}

pub fn nm_dispatcher() -> &'static str {
    "#!/bin/sh\n# upd: on network change, check whether other mirrors should be selected\ncase \"$2\" in\n  up|down|vpn-up|vpn-down|connectivity-change) systemctl start --no-block upd-net.service ;;\nesac\n"
}
