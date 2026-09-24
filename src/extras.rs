//! Дополнения к обновлению: Flatpak, AUR, прошивки, новости Arch, снапшоты, перезапуск служб.

use crate::common::*;
use std::path::Path;

// ---------- Flatpak ----------

pub fn flatpak_updates() -> Vec<String> {
    if !have("flatpak") {
        return vec![];
    }
    out("flatpak", &["remote-ls", "--updates", "--columns=application,version"])
        .0
        .lines()
        .filter(|l| l.split_whitespace().next().map(|a| a.contains('.')).unwrap_or(false))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

pub fn flatpak_prefetch(quiet: bool) -> Result<(), String> {
    run(quiet, &[], "flatpak", &["update", "--no-deploy", "--noninteractive", "-y"])
}

pub fn flatpak_upgrade() -> Result<(), String> {
    run(false, &[], "flatpak", &["update", "--noninteractive", "-y"])
}

// ---------- AUR (paru / yay) — только от имени пользователя, не root ----------

pub fn aur_helper() -> Option<&'static str> {
    ["paru", "yay"].into_iter().find(|h| have(h))
}

fn as_user(user: &str, cmd: &str, args: &[&str]) -> (String, Vec<String>) {
    let mut a: Vec<String> = if have("runuser") { vec!["-u".into(), user.into(), "--".into()] } else { vec!["-u".into(), user.into()] };
    a.push(cmd.into());
    a.extend(args.iter().map(|s| s.to_string()));
    (if have("runuser") { "runuser" } else { "sudo" }.into(), a)
}

pub fn aur_updates(user: &str) -> Vec<String> {
    let Some(h) = aur_helper() else { return vec![] };
    let (cmd, args) = as_user(user, h, &["-Qua"]);
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    lines(&out(&cmd, &a).0).into_iter().filter(|l| l.contains("->")).collect()
}

pub fn aur_upgrade(user: &str) -> Result<(), String> {
    let h = aur_helper().ok_or("нет paru/yay")?;
    let (cmd, args) = as_user(user, h, &["-Sua"]);
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    run(false, &[], &cmd, &a)
}

// ---------- прошивки (fwupd) ----------

pub fn has_fwupd() -> bool {
    have("fwupdmgr")
}

pub fn firmware_refresh() {
    let _ = run(true, &[], "fwupdmgr", &["refresh", "--assume-yes"]);
}

pub fn firmware_updates() -> Vec<String> {
    let (s, _) = out("fwupdmgr", &["get-updates", "--json", "--assume-yes"]);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return vec![] };
    v["Devices"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let name = d["Name"].as_str()?;
                    let cur = d["Version"].as_str().unwrap_or("?");
                    let new = d["Releases"].as_array().and_then(|r| r.first()).and_then(|r| r["Version"].as_str()).unwrap_or("?");
                    Some(format!("{name} {cur} -> {new}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn firmware_upgrade() -> Result<(), String> {
    run(false, &[], "fwupdmgr", &["update"])
}

// ---------- новости Arch ----------

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

fn tag<'a>(item: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = item.find(&open)? + open.len();
    let b = item[a..].find(&close)? + a;
    Some(item[a..b].trim())
}

/// Новости с archlinux.org, вышедшие после последнего полного обновления.
pub fn arch_news(since: i64) -> Result<Vec<News>, String> {
    let body = crate::mirrors::agent(15)
        .get("https://archlinux.org/feeds/news/")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let mut r = vec![];
    for item in body.split("<item>").skip(1) {
        let date = tag(item, "pubDate").and_then(parse_rfc2822).unwrap_or(0);
        if date <= since {
            continue;
        }
        r.push(News {
            title: unescape(tag(item, "title").unwrap_or("?")),
            link: tag(item, "link").unwrap_or("").to_string(),
            date,
        });
    }
    Ok(r)
}

// ---------- снапшоты ----------

#[derive(PartialEq, Clone, Copy)]
pub enum SnapTool {
    Snapper,
    Timeshift,
    None,
}

pub fn snap_tool() -> SnapTool {
    if have("snapper") && Path::new("/etc/snapper/configs/root").exists() {
        SnapTool::Snapper
    } else if have("timeshift") {
        SnapTool::Timeshift
    } else {
        SnapTool::None
    }
}

/// Снапшот перед обновлением. Возвращает номер (для snapper — чтобы связать с «после»).
pub fn snap_pre() -> Result<Option<String>, String> {
    match snap_tool() {
        SnapTool::Snapper => {
            let (s, code) = out("snapper", &["-c", "root", "create", "-t", "pre", "-p", "-c", "number", "-d", "upd: перед обновлением"]);
            if code != 0 {
                return Err("snapper не создал снапшот".into());
            }
            Ok(Some(s.trim().to_string()))
        }
        SnapTool::Timeshift => run(false, &[], "timeshift", &["--create", "--comments", "upd: перед обновлением", "--scripted"]).map(|_| None),
        SnapTool::None => Err("нет snapper (с конфигом root) или timeshift".into()),
    }
}

pub fn snap_post(pre: &str) {
    if snap_tool() == SnapTool::Snapper {
        let _ = run(true, &[], "snapper", &["-c", "root", "create", "-t", "post", "--pre-number", pre, "-c", "number", "-d", "upd: после обновления"]);
    }
}

/// Последние снапшоты, новые сверху.
pub fn snap_list(n: usize) -> Vec<String> {
    match snap_tool() {
        SnapTool::Snapper => {
            let (s, _) = out("snapper", &["--jsonout", "-c", "root", "list", "--disable-used-space"]);
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return vec!["snapper: не удалось прочитать список (нужны права root)".into()] };
            let mut r: Vec<String> = v["root"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|x| x["number"].as_u64().unwrap_or(0) > 0)
                        .map(|x| {
                            format!(
                                "#{:<5} {:<20} {:<6} {}",
                                x["number"].as_u64().unwrap_or(0),
                                x["date"].as_str().unwrap_or(""),
                                x["type"].as_str().unwrap_or(""),
                                x["description"].as_str().unwrap_or("")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            r.reverse();
            r.truncate(n);
            r
        }
        SnapTool::Timeshift => {
            let mut r: Vec<String> = out("timeshift", &["--list"]).0.lines().filter(|l| l.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).map(String::from).collect();
            r.reverse();
            r.truncate(n);
            r
        }
        SnapTool::None => vec![],
    }
}

pub fn rollback_hint() -> Vec<String> {
    let grub_btrfs = Path::new("/etc/grub.d/41_snapshots-btrfs").exists();
    match snap_tool() {
        SnapTool::Snapper if grub_btrfs => vec![
            "Как откатиться:".into(),
            "  1. Перезагрузись, в меню GRUB выбери «… snapshots» → снапшот «перед обновлением».".into(),
            "  2. Если система в нём работает — сделай его постоянным:".into(),
            if have("btrfs-assistant") { "     btrfs-assistant → Snapper → Browse/Restore → Restore".into() } else { "     sudo snapper rollback <номер>".into() },
            "  3. Перезагрузись ещё раз.".into(),
        ],
        SnapTool::Snapper => vec!["Как откатиться: sudo snapper rollback <номер> и перезагрузка.".into()],
        SnapTool::Timeshift => vec!["Как откатиться: sudo timeshift --restore (или из live-USB через Timeshift).".into()],
        SnapTool::None => vec!["Снапшотов нет: не установлен snapper (с конфигом root) или timeshift.".into()],
    }
}

// ---------- перезапуск служб ----------

pub fn restart_services(list: &[String]) {
    for u in list {
        match run(true, &[], "systemctl", &["restart", u]) {
            Ok(()) => println!("  ✓ {u}"),
            Err(e) => println!("  ✗ {u}: {e}"),
        }
    }
}

pub fn print_restart(r: &Restart) {
    if r.services.is_empty() && r.critical.is_empty() && r.apps.is_empty() {
        println!("перезапускать ничего не нужно");
        return;
    }
    if !r.services.is_empty() {
        println!("службы со старыми библиотеками ({}): {}", r.services.len(), r.services.join(", "));
    }
    if !r.critical.is_empty() {
        println!("перезапуск оборвёт сеанс — нужна перезагрузка: {}", r.critical.join(", "));
    }
    if !r.apps.is_empty() {
        println!("программы — перезапусти вручную или перелогинься: {}", r.apps.join(", "));
    }
}
