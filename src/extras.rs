//! Дополнения к обновлению: Flatpak, AUR, прошивки, новости Arch, снапшоты, перезапуск служб.

use crate::common::*;
use std::path::Path;
use std::process::{Command, Stdio};

// ---------- Flatpak ----------

#[derive(Default)]
pub struct FlatpakCheck {
    pub updates: Vec<String>,
    pub error: String,
}

#[derive(Clone, Copy)]
enum FlatpakScope {
    System,
    User,
}

impl FlatpakScope {
    fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }

    fn option(self) -> &'static str {
        match self {
            Self::System => "--system",
            Self::User => "--user",
        }
    }
}

fn invoking_user_command(user: &str, args: &[&str]) -> Result<(String, Vec<String>), String> {
    let (passwd, code) = out("getent", &["passwd", user]);
    if code != 0 {
        return Err(t!("не удалось получить данные пользователя {0}", user));
    }
    let fields: Vec<&str> = passwd.lines().find(|line| line.split(':').next() == Some(user)).ok_or_else(|| t!("пользователь {0} не найден в NSS", user))?.split(':').collect();
    if fields.len() < 7 {
        return Err(t!("неверная запись NSS для пользователя {0}", user));
    }
    let uid: u32 = fields[2].parse().map_err(|_| t!("неверный UID пользователя {0}", user))?;
    let home = fields[5];
    if !Path::new(home).is_absolute() {
        return Err(t!("неверный HOME пользователя {0}", user));
    }
    let runtime = format!("/run/user/{uid}");
    let mut command = if have("runuser") {
        vec!["-u".into(), user.into(), "--".into(), "env".into()]
    } else if have("sudo") {
        vec!["-u".into(), user.into(), "--".into(), "env".into()]
    } else {
        return Err(t!("для запуска Flatpak от имени пользователя нужен runuser или sudo").into());
    };
    command.push(format!("HOME={home}"));
    command.push(format!("XDG_RUNTIME_DIR={runtime}"));
    command.push("LC_ALL=C".into());
    if Path::new(&runtime).join("bus").exists() {
        command.push(format!("DBUS_SESSION_BUS_ADDRESS=unix:path={runtime}/bus"));
    }
    command.extend(args.iter().map(|arg| (*arg).to_string()));
    Ok((if have("runuser") { "runuser" } else { "sudo" }.into(), command))
}

fn out_as_user(user: &str, args: &[&str]) -> Result<(String, i32), String> {
    let (runner, command_args) = invoking_user_command(user, args)?;
    let output = capture(Command::new(&runner).args(&command_args), Some(OUT_MAX))?;
    Ok((String::from_utf8_lossy(&output.stdout).into_owned(), output.status.code().unwrap_or(-1)))
}

fn run_as_user(quiet: bool, user: &str, args: &[&str]) -> Result<(), String> {
    let (runner, command_args) = invoking_user_command(user, args)?;
    let mut command = Command::new(&runner);
    command.args(&command_args);
    if quiet {
        command.stdin(Stdio::null());
        let output = capture(&mut command, None)?;
        if output.status.success() {
            return Ok(());
        }
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{runner}: {}", last_line(&error).unwrap_or(t!("ошибка"))));
    }
    let status = command.status().map_err(|e| format!("{runner}: {e}"))?;
    if status.success() { Ok(()) } else { Err(t!("{1}: код {}", status.code().unwrap_or(-1), runner)) }
}

fn parse_flatpak_updates(output: &str, scope: FlatpakScope) -> Result<Vec<String>, String> {
    let mut updates = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 2 || !fields[0].contains('.') {
            return Err(t!("flatpak remote-ls вернул некорректный список обновлений").into());
        }
        updates.push(format!("[{}] {}", scope.label(), fields.join(" ")));
    }
    Ok(updates)
}

fn flatpak_scope_updates(scope: FlatpakScope, invoking_user: Option<&str>) -> Result<Vec<String>, String> {
    let (output, code) = match scope {
        FlatpakScope::System => {
            let args = ["remote-ls", scope.option(), "--updates", "--columns=application,version"];
            let (output, code) = out("flatpak", &args);
            (output, code)
        }
        FlatpakScope::User => {
            let user = invoking_user.ok_or(t!("нет вызывающего пользователя"))?;
            let args = ["flatpak", "remote-ls", scope.option(), "--updates", "--columns=application,version"];
            out_as_user(user, &args)?
        }
    };
    if code != 0 {
        return Err(t!("flatpak remote-ls завершился с кодом {0}", code));
    }
    parse_flatpak_updates(&output, scope)
}

pub fn flatpak_updates(invoking_user: Option<&str>) -> FlatpakCheck {
    if !have("flatpak") {
        return FlatpakCheck::default();
    }
    let mut result = FlatpakCheck::default();
    let mut errors = Vec::new();
    for scope in [FlatpakScope::System, FlatpakScope::User] {
        if matches!(scope, FlatpakScope::User) && invoking_user.is_none() {
            continue;
        }
        match flatpak_scope_updates(scope, invoking_user) {
            Ok(updates) => result.updates.extend(updates),
            Err(e) => errors.push(format!("{}: {e}", scope.label())),
        }
    }
    if !errors.is_empty() {
        result.error = errors.join("; ");
    }
    result
}

fn flatpak_scopes(updates: &[String]) -> Vec<FlatpakScope> {
    let mut system = false;
    let mut user = false;
    for update in updates {
        if update.starts_with("[user] ") {
            user = true;
        } else if update.starts_with("[system] ") {
            system = true;
        } else {
            // Older saved state had no scope marker; treat it as a system update.
            system = true;
        }
    }
    [system.then_some(FlatpakScope::System), user.then_some(FlatpakScope::User)].into_iter().flatten().collect()
}

fn flatpak_operation(quiet: bool, invoking_user: Option<&str>, updates: &[String], extra: &[&str]) -> Result<(), String> {
    let mut errors = Vec::new();
    for scope in flatpak_scopes(updates) {
        let mut args = vec!["update", scope.option()];
        args.extend_from_slice(extra);
        let result = match scope {
            FlatpakScope::System => run(quiet, &[], "flatpak", &args),
            FlatpakScope::User => match invoking_user {
                Some(user) => {
                    let mut user_args = vec!["flatpak"];
                    user_args.extend(args);
                    run_as_user(quiet, user, &user_args)
                }
                None => Err(t!("невозможно обновить per-user Flatpak без вызывающего пользователя").into()),
            },
        };
        if let Err(e) = result {
            errors.push(format!("{}: {e}", scope.label()));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

pub fn flatpak_prefetch(quiet: bool, invoking_user: Option<&str>, updates: &[String]) -> Result<(), String> {
    flatpak_operation(quiet, invoking_user, updates, &["--no-deploy", "--noninteractive", "-y"])
}

pub fn flatpak_upgrade(invoking_user: Option<&str>, updates: &[String]) -> Result<(), String> {
    flatpak_operation(false, invoking_user, updates, &["--noninteractive", "-y"])
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

pub fn aur_updates(user: &str) -> Result<Vec<String>, String> {
    let Some(h) = aur_helper() else { return Ok(vec![]) };
    let (cmd, args) = as_user(user, h, &["-Qua"]);
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // обрезанный список обновлений не выдаётся за полный: превышение предела — ошибка
    let o = capture(Command::new(&cmd).args(&a).env("LC_ALL", "C").stdin(Stdio::null()), Some(OUT_MAX))?;
    let output = String::from_utf8_lossy(&o.stdout);
    let code = o.status.code().unwrap_or(-1);
    // как и pacman -Qu, paru/yay -Qua выходят с кодом 1 без вывода, когда обновлений нет
    if code == 1 && output.trim().is_empty() && o.stderr.iter().all(u8::is_ascii_whitespace) {
        return Ok(vec![]);
    }
    if code != 0 {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(match last_line(&err) {
            Some(l) => t!("{0} -Qua завершился с кодом {1}: {2}", h, code, l),
            None => t!("{0} -Qua завершился с кодом {1}", h, code),
        });
    }
    let mut updates = Vec::new();
    for (index, line) in lines(&output).into_iter().enumerate() {
        let parts: Vec<&str> = line.split("->").collect();
        if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(t!("{1} вернул некорректный список обновлений AUR (строка {})", index + 1, h));
        }
        updates.push(line.trim().to_string());
    }
    Ok(updates)
}

pub fn aur_upgrade(user: &str) -> Result<(), String> {
    let h = aur_helper().ok_or(t!("нет paru/yay"))?;
    let (cmd, args) = as_user(user, h, &["-Sua"]);
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    run(false, &[], &cmd, &a)
}

// ---------- поиск и установка из AUR ----------

#[derive(Clone, Default, Debug)]
pub struct AurPkg {
    pub name: String,
    pub version: String,
    pub desc: String,
    pub votes: u64,
    pub popularity: f64,
    pub out_of_date: bool,
    /// установленная версия, если пакет уже стоит
    pub installed: Option<String>,
}

/// Имя пакета Arch: строчные буквы, цифры и @._+- (защищает и командную строку помощника).
pub fn valid_pkg_name(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('-') && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@._+-".contains(&b))
}

/// Поиск через AUR RPC (по имени и описанию); точное совпадение имени — первым, дальше по популярности.
pub fn aur_search(q: &str) -> Result<Vec<AurPkg>, String> {
    let q = q.trim();
    if q.chars().count() < 2 {
        return Err(t!("для поиска нужно хотя бы 2 символа").into());
    }
    let url = format!("https://aur.archlinux.org/rpc/v5/search/{}?by=name-desc", crate::vpn::pct_encode(q));
    let body = read_text(crate::mirrors::agent(20).get(&url).call().map_err(|e| t!("AUR недоступен: {0}", e))?, 8 << 20)?;
    let mut r = parse_aur_search(&body)?;
    let names: Vec<&str> = r.iter().map(|p| p.name.as_str()).collect();
    let installed = installed_versions(&names);
    for p in &mut r {
        p.installed = installed.get(&p.name).cloned();
    }
    let ql = q.to_lowercase();
    r.sort_by(|a, b| (b.name == ql).cmp(&(a.name == ql)).then(b.popularity.total_cmp(&a.popularity)).then(b.votes.cmp(&a.votes)));
    r.truncate(250);
    Ok(r)
}

fn parse_aur_search(body: &str) -> Result<Vec<AurPkg>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| t!("AUR вернул неверный JSON: {0}", e))?;
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        return Err(format!("AUR: {}", v.get("error").and_then(|e| e.as_str()).unwrap_or(t!("ошибка"))));
    }
    let list = v.get("results").and_then(|r| r.as_array()).ok_or(t!("AUR: в ответе нет results"))?;
    Ok(list
        .iter()
        .filter_map(|p| {
            let name = p.get("Name")?.as_str()?.to_string();
            valid_pkg_name(&name).then(|| AurPkg {
                name,
                version: p.get("Version").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
                desc: p.get("Description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                votes: p.get("NumVotes").and_then(|x| x.as_u64()).unwrap_or(0),
                popularity: p.get("Popularity").and_then(|x| x.as_f64()).unwrap_or(0.0),
                out_of_date: p.get("OutOfDate").map(|x| !x.is_null()).unwrap_or(false),
                installed: None,
            })
        })
        .collect())
}

fn installed_versions(names: &[&str]) -> std::collections::HashMap<String, String> {
    if names.is_empty() {
        return Default::default();
    }
    let mut args = vec!["-Q"];
    args.extend(names);
    // ненайденные пакеты pacman пишет в stderr — их просто нет в выводе
    out("pacman", &args)
        .0
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(n, v)| (n.to_string(), v.trim().to_string())))
        .collect()
}

/// Ставит paru (или yay) из репозиториев, если их там предлагает дистрибутив (chaotic-aur, EndeavourOS, CachyOS…).
fn install_aur_helper() -> Result<&'static str, String> {
    for h in ["paru", "yay"] {
        if out("pacman", &["-Si", h]).1 == 0 {
            if !confirm(&t!("Для AUR нужен помощник. Установить {0} из репозитория?", h), true) {
                return Err(t!("без paru или yay пакеты из AUR не ставятся").into());
            }
            let args = ["pacman", "-S", "--needed", h];
            if is_root() {
                run(false, &[], args[0], &args[1..])?;
            } else {
                run(false, &[], "sudo", &args)?;
            }
            return Ok(h);
        }
    }
    Err(t!("paru и yay нет в репозиториях — поставь один из них вручную: https://github.com/Morganamilo/paru#installation").into())
}

/// Сборка и установка через paru/yay от имени обычного пользователя: помощник сам покажет PKGBUILD и спросит подтверждение.
pub fn aur_install(user: Option<&str>, pkgs: &[String]) -> Result<(), String> {
    if let Some(bad) = pkgs.iter().find(|p| !valid_pkg_name(p)) {
        return Err(t!("неверное имя пакета: {0}", bad));
    }
    if pkgs.is_empty() {
        return Err(t!("не указан пакет").into());
    }
    let h = match aur_helper() {
        Some(h) => h,
        None => install_aur_helper()?,
    };
    // --aur есть и у paru, и у yay: ставим именно из AUR, даже если в репозиториях есть одноимённый пакет
    let mut args = vec!["-S", "--aur"];
    args.extend(pkgs.iter().map(String::as_str));
    if !is_root() {
        return run(false, &[], h, &args);
    }
    // от root помощники AUR не собирают — нужен пользователь, запустивший upd через sudo
    let user = user.ok_or(t!("AUR собирается от обычного пользователя: запусти upd без sudo или через sudo из своей сессии"))?;
    let (cmd, a) = as_user(user, h, &args);
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    run(false, &[], &cmd, &a)
}

// ---------- прошивки (fwupd) ----------

pub fn has_fwupd() -> bool {
    have("fwupdmgr")
}

pub fn firmware_refresh() -> Result<(), String> {
    let o = capture(Command::new("fwupdmgr").args(["refresh", "--assume-yes"]).stdin(Stdio::null()), Some(OUT_MAX))?;
    // код 2 — «нечего делать»: метаданные и так свежие
    match o.status.code() {
        Some(0 | 2) => Ok(()),
        _ => Err(format!("fwupdmgr: {}", last_line(&String::from_utf8_lossy(&o.stderr)).unwrap_or(t!("ошибка")))),
    }
}

pub fn firmware_updates() -> Result<Vec<String>, String> {
    let (s, code) = out("fwupdmgr", &["get-updates", "--json", "--assume-yes"]);
    // Код 2 — штатный пустой результат, но его нужно подтвердить JSON-ответом.
    // Остальные ненулевые коды сообщаем до разбора stdout: при сбое он часто пустой.
    if code != 0 && code != 2 {
        return Err(t!("fwupdmgr get-updates завершился с кодом {0}", code));
    }
    let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| t!("fwupdmgr вернул неверный JSON: {0}", e))?;
    let devices = v
        .get("Devices")
        .and_then(serde_json::Value::as_array)
        .ok_or(t!("fwupdmgr JSON не содержит массив Devices"))?;
    // fwupd использует код 2, когда обновлений нет; JSON в этом случае содержит пустой Devices.
    if code == 2 && devices.is_empty() {
        return Ok(vec![]);
    }
    if code != 0 {
        return Err(t!("fwupdmgr get-updates завершился с кодом {0}", code));
    }
    devices
        .iter()
        .map(|d| {
            let name = d.get("Name").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()).ok_or(t!("fwupdmgr JSON содержит запись Devices без Name"))?;
            let cur = d.get("Version").and_then(serde_json::Value::as_str).unwrap_or("?");
            let new = d
                .get("Releases")
                .and_then(serde_json::Value::as_array)
                .and_then(|r| r.first())
                .and_then(|r| r.get("Version"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            Ok(format!("{name} {cur} -> {new}"))
        })
        .collect()
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
    let body = read_text(crate::mirrors::agent(15).get("https://archlinux.org/feeds/news/").call().map_err(|e| e.to_string())?, 4 << 20)?;
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

/// Описания снапшотов записываются по-английски: их показывают GRUB (grub-btrfs), snapper и btrfs-assistant,
/// а шрифт GRUB может не иметь кириллицы, китайских и арабских букв. В списке upd они переводятся (snap_desc_label).
pub const SNAP_PRE_DESC: &str = "upd: before update";
pub const SNAP_POST_DESC: &str = "upd: after update";

/// Снапшот перед обновлением. Возвращает номер (для snapper — чтобы связать с «после»).
pub fn snap_pre() -> Result<Option<String>, String> {
    match snap_tool() {
        SnapTool::Snapper => {
            let (s, code) = out("snapper", &["-c", "root", "create", "-t", "pre", "-p", "-c", "number", "-d", SNAP_PRE_DESC]);
            if code != 0 {
                return Err(t!("snapper не создал снапшот").into());
            }
            Ok(Some(s.trim().to_string()))
        }
        SnapTool::Timeshift => run(false, &[], "timeshift", &["--create", "--comments", SNAP_PRE_DESC, "--scripted"]).map(|_| None),
        SnapTool::None => Err(t!("нет snapper (с конфигом root) или timeshift").into()),
    }
}

pub fn snap_post(pre: &str) {
    if snap_tool() == SnapTool::Snapper {
        let _ = run(true, &[], "snapper", &["-c", "root", "create", "-t", "post", "--pre-number", pre, "-c", "number", "-d", SNAP_POST_DESC]);
    }
}

/// Последние снапшоты, новые сверху.
/// Описание снапшота для показа: свои (и прежние русские) — на языке интерфейса, чужие — как есть.
fn snap_desc_label(d: &str) -> String {
    match d {
        SNAP_PRE_DESC | "upd: перед обновлением" => t!("upd: перед обновлением").into(),
        SNAP_POST_DESC | "upd: после обновления" => t!("upd: после обновления").into(),
        _ => d.into(),
    }
}

pub fn snap_list(n: usize) -> Vec<String> {
    match snap_tool() {
        SnapTool::Snapper => {
            let (s, _) = out("snapper", &["--jsonout", "-c", "root", "list", "--disable-used-space"]);
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return vec![t!("snapper: не удалось прочитать список (нужны права root)").into()] };
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
                                snap_desc_label(x["description"].as_str().unwrap_or(""))
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
            let mut r: Vec<String> = out("timeshift", &["--list"])
                .0
                .lines()
                .filter(|l| l.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                .map(|l| l.replace(SNAP_PRE_DESC, t!("upd: перед обновлением")))
                .collect();
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
            t!("Как откатиться:").into(),
            t!("  1. Перезагрузись, в меню GRUB выбери «… snapshots» → снапшот «upd: before update».").into(),
            t!("  2. Если система в нём работает — сделай его постоянным:").into(),
            if have("btrfs-assistant") { "     btrfs-assistant → Snapper → Browse/Restore → Restore".into() } else { t!("     sudo snapper rollback <номер>").into() },
            t!("  3. Перезагрузись ещё раз.").into(),
        ],
        SnapTool::Snapper => vec![t!("Как откатиться: sudo snapper rollback <номер> и перезагрузка.").into()],
        SnapTool::Timeshift => vec![t!("Как откатиться: sudo timeshift --restore (или из live-USB через Timeshift).").into()],
        SnapTool::None => vec![t!("Снапшотов нет: не установлен snapper (с конфигом root) или timeshift.").into()],
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
    if r.services.is_empty() && r.critical.is_empty() && r.apps.is_empty() && r.unknown.is_empty() {
        println!("{}", t!("перезапускать ничего не нужно"));
        return;
    }
    if !r.services.is_empty() {
        println!("{}", t!("службы со старыми библиотеками ({}): {}", r.services.len(), r.services.join(", ")));
    }
    if !r.critical.is_empty() {
        println!("{}", t!("перезапуск оборвёт сеанс — нужна перезагрузка: {}", r.critical.join(", ")));
    }
    if !r.apps.is_empty() {
        println!("{}", t!("программы — перезапусти вручную или перелогинься: {}", r.apps.join(", ")));
    }
    if !r.unknown.is_empty() {
        println!("{}", t!("процессы без распознанного cgroup: {}", r.unknown.join(", ")));
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    struct DirGuard(PathBuf);

    impl DirGuard {
        fn new(path: PathBuf) -> Self {
            Self(path)
        }
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn bin_fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("upd-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- UPD-04A ---
    #[test]
    fn upd04a_missing_optional_flatpak_is_skipped() {
        let _isolation = crate::common::contract_fixtures::isolation_lock();
        let dir = bin_fixture("flatpak-absent");
        let _guard = DirGuard::new(dir.clone());
        let old_path = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", &dir); }
        let result = flatpak_updates(None);
        match old_path {
            Some(path) => unsafe { std::env::set_var("PATH", path); },
            None => unsafe { std::env::remove_var("PATH"); },
        }
        assert!(result.updates.is_empty());
        assert!(result.error.is_empty());
    }

    #[test]
    fn upd04a_flatpak_error_is_not_empty_list() {
        let dir = bin_fixture("flatpak-err");
        let _g = DirGuard::new(dir.clone());
        write_executable(&dir.join("flatpak"), "#!/bin/sh\nexit 3\n");
        let result = crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(None));
        assert!(result.error.contains('3'), "ожидали код в ошибке: {}", result.error);
    }

    #[test]
    fn upd04a_flatpak_empty_stdout_is_ok() {
        let dir = bin_fixture("flatpak-empty");
        write_executable(&dir.join("flatpak"), "#!/bin/sh\nexit 0\n");
        let _g = DirGuard::new(dir.clone());
        let result = crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(None));
        assert!(result.error.is_empty());
        assert!(result.updates.is_empty());
    }

    #[test]
    fn upd04a_flatpak_valid_line_parsed() {
        let dir = bin_fixture("flatpak-line");
        write_executable(&dir.join("flatpak"), "#!/bin/sh\necho 'org.example.App 1.2.3'\nexit 0\n");
        let _g = DirGuard::new(dir.clone());
        let result = crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(None));
        assert!(result.error.is_empty());
        assert_eq!(result.updates, vec!["[system] org.example.App 1.2.3"]);
    }

    #[test]
    fn upd04a_flatpak_malformed_is_error() {
        let dir = bin_fixture("flatpak-bad");
        write_executable(&dir.join("flatpak"), "#!/bin/sh\necho 'not-a-flatpak-ref'\nexit 0\n");
        let _g = DirGuard::new(dir.clone());
        assert!(!crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(None)).error.is_empty());
    }

    // --- UPD-04B ---
    #[test]
    fn upd04b_fwupd_command_failure_is_error() {
        let dir = bin_fixture("fwupd-err");
        write_executable(&dir.join("fwupdmgr"), "#!/bin/sh\nexit 5\n");
        let _g = DirGuard::new(dir.clone());
        let err = crate::common::contract_fixtures::with_prepend_path(&dir, firmware_updates).unwrap_err();
        assert!(err.contains('5'), "ожидали код в ошибке: {err}");
    }

    #[test]
    fn upd04b_fwupd_empty_devices_is_ok() {
        let dir = bin_fixture("fwupd-empty");
        write_executable(&dir.join("fwupdmgr"), "#!/bin/sh\necho '{\"Devices\":[]}'\nexit 2\n");
        let _g = DirGuard::new(dir.clone());
        assert!(crate::common::contract_fixtures::with_prepend_path(&dir, || firmware_updates().unwrap().is_empty()));
    }

    #[test]
    fn upd04b_fwupd_invalid_json_is_error() {
        let dir = bin_fixture("fwupd-json");
        write_executable(&dir.join("fwupdmgr"), "#!/bin/sh\necho 'not-json'\nexit 0\n");
        let _g = DirGuard::new(dir.clone());
        let err = crate::common::contract_fixtures::with_prepend_path(&dir, firmware_updates).unwrap_err();
        assert!(err.contains("JSON"));
    }

    #[test]
    fn upd04b_fwupd_refresh_nothing_to_do_is_ok() {
        let dir = bin_fixture("fwupd-fresh");
        write_executable(&dir.join("fwupdmgr"), "#!/bin/sh\necho 'Metadata is up to date' >&2\nexit 2\n");
        let _g = DirGuard::new(dir.clone());
        assert!(crate::common::contract_fixtures::with_prepend_path(&dir, firmware_refresh).is_ok());
    }

    #[test]
    fn aur_search_parses_and_rejects_bad_names() {
        let body = r#"{"version":5,"type":"search","resultcount":3,"results":[
            {"Name":"paru-bin","Version":"2.0.4-1","Description":"Feature packed AUR helper","NumVotes":120,"Popularity":3.5,"OutOfDate":null},
            {"Name":"old-thing","Version":"1-1","Description":null,"NumVotes":1,"Popularity":0.0,"OutOfDate":1700000000},
            {"Name":"--evil","Version":"1-1"}]}"#;
        let r = parse_aur_search(body).unwrap();
        assert_eq!(r.len(), 2, "имя с ведущим дефисом отброшено");
        assert_eq!(r[0].name, "paru-bin");
        assert_eq!(r[0].votes, 120);
        assert!(!r[0].out_of_date && r[1].out_of_date);
        assert!(parse_aur_search(r#"{"type":"error","error":"Too many package results."}"#).unwrap_err().contains("Too many"));
        assert!(valid_pkg_name("lib32-mesa-git") && valid_pkg_name("python-foo_bar+1@x"));
        assert!(!valid_pkg_name("Foo") && !valid_pkg_name("a b") && !valid_pkg_name("a;rm"));
    }

    // --- UPD-04C ---
    #[test]
    fn upd04c_aur_exit1_without_output_is_empty() {
        let dir = bin_fixture("aur-none");
        write_executable(&dir.join("paru"), "#!/bin/sh\nexit 1\n");
        write_executable(&dir.join("runuser"), "#!/bin/sh\nshift; shift; shift; exec \"$@\"\n");
        let _g = DirGuard::new(dir.clone());
        assert!(crate::common::contract_fixtures::with_prepend_path(&dir, || aur_updates("testuser")).unwrap().is_empty());
    }

    #[test]
    fn upd04c_aur_exit1_with_stderr_is_error() {
        let dir = bin_fixture("aur-fail");
        write_executable(&dir.join("paru"), "#!/bin/sh\necho 'error: failed to connect' >&2\nexit 1\n");
        write_executable(&dir.join("runuser"), "#!/bin/sh\nshift; shift; shift; exec \"$@\"\n");
        let _g = DirGuard::new(dir.clone());
        let err = crate::common::contract_fixtures::with_prepend_path(&dir, || aur_updates("testuser")).unwrap_err();
        assert!(err.contains("failed to connect"), "{err}");
    }

    #[test]
    fn upd04c_aur_empty_stdout_is_checked() {
        let dir = bin_fixture("aur-empty");
        write_executable(&dir.join("paru"), "#!/bin/sh\nexit 0\n");
        write_executable(&dir.join("runuser"), "#!/bin/sh\nshift; shift; shift; exec \"$@\"\n");
        let _g = DirGuard::new(dir.clone());
        let updates = crate::common::contract_fixtures::with_prepend_path(&dir, || aur_updates("testuser")).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn upd04c_aur_failure_is_not_empty_list() {
        let dir = bin_fixture("aur-err");
        write_executable(&dir.join("paru"), "#!/bin/sh\nexit 4\n");
        write_executable(&dir.join("runuser"), "#!/bin/sh\nshift; shift; shift; exec \"$@\"\n");
        let _g = DirGuard::new(dir.clone());
        let err = crate::common::contract_fixtures::with_prepend_path(&dir, || aur_updates("testuser")).unwrap_err();
        assert!(err.contains('4'), "{err}");
    }

    // --- UPD-06 ---
    #[test]
    fn upd06_flatpak_checks_user_and_system_scopes() {
        let dir = bin_fixture("flatpak-both");
        write_executable(
            &dir.join("flatpak"),
            "#!/bin/sh\ncase \"$*\" in *--user*) echo 'org.example.User 2.0';; *) echo 'org.example.Sys 1.0';; esac\nexit 0\n",
        );
        write_executable(&dir.join("getent"), "#!/bin/sh\necho 'updtest:x:1000:1000:upd:/tmp:/bin/sh'\n");
        write_executable(&dir.join("runuser"), "#!/bin/sh\nflatpak \"$@\"\n");
        let _g = DirGuard::new(dir.clone());
        let result = crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(Some("updtest")));
        assert!(result.error.is_empty(), "{}", result.error);
        assert!(result.updates.iter().any(|u| u == "[system] org.example.Sys 1.0"), "{:?}", result.updates);
        assert!(result.updates.iter().any(|u| u == "[user] org.example.User 2.0"), "{:?}", result.updates);
        let err = flatpak_upgrade(None, &["[user] org.example.User 2.0".into()]).unwrap_err();
        assert!(err.contains("per-user"), "{err}");
    }

    #[test]
    fn upd06_flatpak_checks_system_scope_without_user() {
        let dir = bin_fixture("flatpak-scope");
        write_executable(&dir.join("flatpak"), "#!/bin/sh\necho 'org.example.App 1.0'\nexit 0\n");
        let _g = DirGuard::new(dir.clone());
        let result = crate::common::contract_fixtures::with_prepend_path(&dir, || flatpak_updates(None));
        assert!(result.error.is_empty());
        assert_eq!(result.updates, vec!["[system] org.example.App 1.0"]);
    }
}
