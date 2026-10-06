const VPN_CORES: [&str; 7] = ["FlClashCore", "mihomo", "clash", "clash-meta", "verge-mihomo", "sing-box", "xray"];

struct ProcInfo {
    pid: String,
    comm: String,
    ours: bool,
    sockets: Vec<String>,
    tuns: Vec<String>,
}

/// Процессы с их сокетами и TUN-интерфейсами (/proc/PID/fd и fdinfo); ours — наше ядро mihomo.
fn processes() -> Vec<ProcInfo> {
    let core = fs::canonicalize(core_bin()).ok();
    let mut list = vec![];
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().filter(|p| p.bytes().all(|b| b.is_ascii_digit())) else { continue };
        let dir = e.path();
        let ours = core.is_some() && fs::read_link(dir.join("exe")).ok() == core;
        let (mut sockets, mut tuns) = (vec![], vec![]);
        for fd in fs::read_dir(dir.join("fd")).into_iter().flatten().flatten() {
            let Ok(target) = fs::read_link(fd.path()) else { continue };
            let target = target.to_string_lossy();
            if let Some(i) = target.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']')) {
                sockets.push(i.to_string());
            } else if target == "/dev/net/tun" {
                let info = fs::read_to_string(dir.join("fdinfo").join(fd.file_name())).unwrap_or_default();
                if let Some(iff) = info.lines().find_map(|l| l.strip_prefix("iff:")) {
                    tuns.push(iff.trim().to_string());
                }
            }
        }
        if sockets.is_empty() && tuns.is_empty() {
            continue;
        }
        let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default().trim().to_string();
        list.push(ProcInfo { pid: pid.to_string(), comm, ours, sockets, tuns });
    }
    list
}

/// inode сокетов, слушающих порт: TCP в состоянии LISTEN, UDP без соединения; любой адрес.
fn listening_inodes(table: &str, port: u16, udp: bool) -> Vec<String> {
    let want = if udp { "07" } else { "0A" };
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let p = f.get(1)?.rsplit_once(':')?.1;
            (u16::from_str_radix(p, 16).ok()? == port && f.get(3) == Some(&want)).then(|| f.get(9).map(|s| s.to_string()))?
        })
        .collect()
}

/// Конфликт из снимка /proc: чужой слушатель на порту прокси/DNS или TUN другого ядра VPN.
fn find_conflict(c: &Config, procs: &[ProcInfo], tables: &[(&str, bool)]) -> Option<String> {
    let mut ports = vec![c.vpn_port];
    if c.vpn_dns {
        ports.push(DNS_PORT);
    }
    for port in ports {
        for (table, udp) in tables {
            for inode in listening_inodes(table, port, *udp) {
                match procs.iter().find(|p| p.sockets.contains(&inode)) {
                    Some(p) if p.ours => {}
                    Some(p) if p.comm == "FlClashCore" => return Some(flclash_message()),
                    Some(p) => return Some(t!("порт {0} занят: {1} (PID {2}) — освободи его или смени vpn_port", port, p.comm, p.pid)),
                    None => return Some(t!("порт {0} занят другим процессом — освободи его или смени vpn_port", port)),
                }
            }
        }
    }
    if c.vpn_tun {
        for p in procs.iter().filter(|p| !p.ours && VPN_CORES.contains(&p.comm.as_str())) {
            if let Some(dev) = p.tuns.iter().find(|d| d.as_str() != TUN_DEV) {
                if p.comm == "FlClashCore" {
                    return Some(flclash_message());
                }
                return Some(t!("работает другой VPN с TUN {0}: {1} (PID {2}) — выключи его, иначе два VPN помешают друг другу", dev, p.comm, p.pid));
            }
        }
    }
    None
}

fn flclash_message() -> String {
    t!("запущен FlClash — закрой его (и выключи его автозапуск), иначе два VPN помешают друг другу").into()
}

/// Реальный конфликт перед запуском: чужой TUN другого ядра VPN или занятый порт прокси либо DNS.
/// Процесс без TUN и без такого порта запуску не мешает, как бы он ни назывался.
pub fn conflict(c: &Config) -> Option<String> {
    let procs = processes();
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    let (tcp, tcp6, udp, udp6) = (read("/proc/net/tcp"), read("/proc/net/tcp6"), read("/proc/net/udp"), read("/proc/net/udp6"));
    find_conflict(c, &procs, &[(&tcp, false), (&tcp6, false), (&udp, true), (&udp6, true)])
}

/// Применить изменения настроек: пересобрать конфиг и перезагрузить работающее ядро.
pub fn apply(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    let (changed, latest) = {
        let _vpn_files = vpn_config_lock(true)?;
        let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
        (write_config_locked(&latest)?, latest)
    };
    let c = &latest;
    if !service_active() {
        log(t!("конфиг собран; VPN не запущен"));
        sysproxy(c, user).map_err(|error| format!("VPN configuration saved, but user proxy failed: {error}; retry: cm vpn restart"))?;
        return Ok(());
    }
    if changed {
        // TUN включается/выключается надёжнее перезапуском, остальное — горячей перезагрузкой
        let tun_now = snapshot().tun;
        if tun_now != c.vpn_tun {
            restart(c)?;
            log(t!("VPN перезапущен"));
        } else {
            reload()?;
            log(t!("конфиг перезагружен"));
        }
    } else {
        log(t!("конфиг не изменился"));
    }
    sysproxy(c, user).map_err(|error| format!("VPN core applied, but user proxy failed: {error}; retry: cm vpn restart"))?;
    Ok(())
}

/// Configuration is already persisted; absent subscriptions are an explicit pending state.
pub fn apply_saved(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    if load_subs()?.list.is_empty() {
        log("settings saved; VPN pending until a subscription is added");
        return Ok(());
    }
    let result = apply(c, user, log).map_err(|error| format!("settings saved, but not applied: {error}; retry: cm vpn restart"));
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("settings", error) }
    result
}

pub fn apply_saved_mode(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    if load_subs()?.list.is_empty() { log("settings saved; VPN pending until a subscription is added"); return Ok(()); }
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    write_config_locked(&latest).and_then(|_| if running() { set_mode(latest.vpn_mode_name()) } else { Ok(()) })
        .and_then(|_| sysproxy(&latest, user).map(|_| ()))
        .map_err(|error| format!("settings saved, but not applied: {error}; retry: cm vpn restart"))
}

/// Режим «только прокси»: включить системный прокси GNOME пользователю; в TUN — выключить.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SysproxyStatus { BackgroundSkipped, Applied { uid: u32, enabled: bool, port: u16 } }

pub fn sysproxy(c: &Config, user: Option<&UserContext>) -> Result<SysproxyStatus, String> {
    let Some(user) = user else { return Ok(SysproxyStatus::BackgroundSkipped); };
    if !have("gsettings") { return Err(format!("UID {}: gsettings is unavailable", user.uid)); }
    let runtime = user.runtime_dir.to_string_lossy().into_owned();
    let bus = format!("unix:path={runtime}/bus");
    let on = !c.vpn_tun && service_active();
    let mut cmds: Vec<Vec<String>> = vec![vec!["org.gnome.system.proxy".into(), "mode".into(), if on { "manual" } else { "none" }.into()]];
    if on {
        for kind in ["http", "https", "socks"] {
            let schema = format!("org.gnome.system.proxy.{kind}");
            cmds.push(vec![schema.clone(), "host".into(), "127.0.0.1".into()]);
            cmds.push(vec![schema, "port".into(), c.vpn_port.to_string()]);
        }
        cmds.push(vec!["org.gnome.system.proxy".into(), "ignore-hosts".into(), "['localhost', '127.0.0.0/8', '::1', '192.168.0.0/16', '10.0.0.0/8', '172.16.0.0/12']".into()]);
    }
    for a in cmds {
        let result = if unsafe { libc::geteuid() } == user.uid {
            let mut args = vec!["set"]; args.extend(a.iter().map(String::as_str));
            run(true, &[("XDG_RUNTIME_DIR", &runtime), ("DBUS_SESSION_BUS_ADDRESS", &bus)], "gsettings", &args)
        } else {
            let runtime_env = format!("XDG_RUNTIME_DIR={runtime}");
            let bus_env = format!("DBUS_SESSION_BUS_ADDRESS={bus}");
            let mut args = vec!["-u", user.name.as_str(), "--", "env", &runtime_env, &bus_env, "gsettings", "set"];
            args.extend(a.iter().map(String::as_str));
            run(true, &[], "runuser", &args)
        };
        result.map_err(|error| format!("UID {} ({}): {error}", user.uid, user.name))?;
    }
    Ok(SysproxyStatus::Applied { uid: user.uid, enabled: on, port: c.vpn_port })
}

// ======================= ядро: mihomo, обновление по релизам FlClash =======================

