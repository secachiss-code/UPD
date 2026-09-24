//! Всё, что зависит от пакетного менеджера: pacman, apt, dnf/yum, zypper.

use crate::common::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub trait Backend {
    fn name(&self) -> String;
    /// upd сам выбирает и прописывает зеркала (иначе это делает дистрибутив)
    fn mirrors_managed(&self) -> bool;
    fn mirror_note(&self) -> String;
    /// Файл, который могут перезаписать чужие утилиты (rate-mirrors, garuda-update)
    fn watch_path(&self) -> Option<String> {
        None
    }

    fn probe_url(&self, m: &str) -> String;
    /// Адрес, по которому видно, когда зеркало синхронизировалось в последний раз
    fn fresh_url(&self, _m: &str) -> Option<String> {
        None
    }
    fn parse_fresh(&self, _body: &str) -> Option<i64> {
        None
    }
    fn valid_mirror(&self, s: &str) -> Result<(), String>;
    fn default_mirrors(&self) -> Vec<String>;
    fn pinned(&self) -> Vec<String>;
    fn list_mirrors(&self) -> Vec<String>;
    /// Зеркала рядом с текущим местом; второй элемент — полный запасной список (если есть)
    fn discover(&self, n: usize, log: Log) -> Result<(Vec<String>, Option<Vec<String>>), String>;
    fn apply_mirrors(&self, best: &[String], fallback: Option<&[String]>) -> Result<bool, String>;
    fn remove_mirrors(&self) -> Result<(), String>;

    fn refresh(&self, quiet: bool) -> Result<(), String>;
    fn updates(&self) -> Result<Vec<String>, String>;
    fn download_size(&self, _pkgs: &[String]) -> Option<u64> {
        None
    }
    fn prefetch(&self, pkgs: &[String], quiet: bool) -> Result<(), String>;
    /// Установка обновлений (интерактивно). aur — обновить и AUR, если это делает сам установщик.
    fn upgrade(&self, aur: bool) -> Result<(), String>;
    /// Сам ли установщик обновляет AUR (garuda-update)
    fn upgrade_handles_aur(&self) -> bool {
        false
    }
    fn clean(&self) -> Result<(), String>;
    fn orphans(&self) -> Vec<String>;
    fn pending_configs(&self) -> Vec<String>;
    fn merge(&self) -> Result<(), String>;
    fn cache_dirs(&self) -> Vec<&'static str>;
    fn db_path(&self) -> &'static str;
    fn history(&self, n: usize) -> Vec<String>;
    /// Новости Arch имеют смысл (Arch и производные, кроме Manjaro)
    fn arch_news(&self) -> bool {
        false
    }
    /// Время последнего полного обновления (для отбора непрочитанных новостей)
    fn last_upgrade(&self) -> i64 {
        fs::metadata(self.db_path())
            .and_then(|m| m.modified())
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
            .unwrap_or(0)
    }
    /// Снапшоты при обновлении делает пакетный менеджер сам (snap-pac и т.п.)
    fn auto_snapshots(&self) -> bool {
        false
    }
}

pub fn detect() -> Result<Box<dyn Backend>, String> {
    let osr = os_release();
    if have("pacman") {
        Ok(Box::new(Pacman::new(&osr)))
    } else if have("apt-get") {
        Ok(Box::new(Apt::new(&osr)))
    } else if have("zypper") {
        Ok(Box::new(Rpm::zypper(&osr)))
    } else if have("dnf5") || have("dnf") || have("yum") {
        Ok(Box::new(Rpm::dnf(&osr)))
    } else {
        Err(format!("{}: пакетный менеджер не поддерживается (есть: pacman, apt, dnf, zypper)", osr["PRETTY_NAME"]))
    }
}

fn tmp_path(tag: &str) -> String {
    format!("/tmp/upd-{tag}.{}", std::process::id())
}

fn hist_from(path: &str, n: usize, keep: impl Fn(&str) -> bool) -> Vec<String> {
    tail_file(path, 512 << 10).into_iter().rev().filter(|l| !l.trim().is_empty() && keep(l)).take(n).collect()
}

// ======================= pacman (Arch, Garuda, EndeavourOS, CachyOS…) =======================

const PIN_BEGIN: &str = "## >>> upd: закреплённые зеркала (управляется автоматически, см. upd) >>>";
const PIN_END: &str = "## <<< upd <<<";

pub struct Pacman {
    mirrorlist: String,
    distro: String,
    manjaro: bool,
}

impl Pacman {
    fn new(osr: &BTreeMap<String, String>) -> Self {
        Pacman {
            mirrorlist: env_or("UPD_MIRRORLIST", "/etc/pacman.d/mirrorlist"),
            distro: osr["PRETTY_NAME"].clone(),
            manjaro: osr.get("ID").map(|s| s == "manjaro").unwrap_or(false),
        }
    }

    fn read_list(&self) -> Vec<String> {
        fs::read_to_string(&self.mirrorlist).map(|s| s.trim_end_matches('\n').lines().map(String::from).collect()).unwrap_or_default()
    }

    fn sync_db() -> String {
        format!("{}/syncdb", state_dir())
    }
}

fn server_url(l: &str) -> Option<String> {
    let (k, v) = l.trim().split_once('=')?;
    (k.trim() == "Server").then(|| v.trim().to_string())
}

/// Делит mirrorlist на закреплённые зеркала и остальной файл.
fn split_pin(ls: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut pinned, mut rest, mut inside) = (vec![], vec![], false);
    for l in ls {
        if l == PIN_BEGIN {
            inside = true;
        } else if l == PIN_END {
            inside = false;
        } else if inside {
            if let Some(u) = server_url(l) {
                pinned.push(u);
            }
        } else {
            rest.push(l.clone());
        }
    }
    (pinned, rest)
}

fn mirror_root(m: &str) -> String {
    // https://host/archlinux/$repo/os/$arch → https://host/archlinux/
    m.split("$repo").next().unwrap_or(m).to_string()
}

impl Backend for Pacman {
    fn name(&self) -> String {
        format!("{} · pacman", self.distro)
    }
    fn mirrors_managed(&self) -> bool {
        !self.manjaro
    }
    fn mirror_note(&self) -> String {
        "в Manjaro зеркалами управляет pacman-mirrors (sudo pacman-mirrors --fasttrack)".into()
    }
    fn watch_path(&self) -> Option<String> {
        Some(self.mirrorlist.clone())
    }
    fn probe_url(&self, m: &str) -> String {
        format!("{}/extra.db", m.replace("$repo", "extra").replace("$arch", "x86_64").trim_end_matches('/'))
    }
    fn fresh_url(&self, m: &str) -> Option<String> {
        Some(format!("{}lastsync", mirror_root(m)))
    }
    fn parse_fresh(&self, body: &str) -> Option<i64> {
        body.trim().parse().ok()
    }
    fn valid_mirror(&self, s: &str) -> Result<(), String> {
        if s.starts_with("http") && s.contains("$repo") {
            Ok(())
        } else {
            Err("нужен http(s)://…/$repo/os/$arch".into())
        }
    }
    fn default_mirrors(&self) -> Vec<String> {
        vec!["https://geo.mirror.pkgbuild.com/$repo/os/$arch".into()]
    }
    fn pinned(&self) -> Vec<String> {
        split_pin(&self.read_list()).0
    }
    fn list_mirrors(&self) -> Vec<String> {
        split_pin(&self.read_list()).1.iter().filter_map(|l| server_url(l)).collect()
    }

    fn discover(&self, n: usize, log: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
        let tmp = tmp_path("mirrors");
        let mut file: Vec<String> = vec![];
        let tool = if have("rate-mirrors") {
            log("  rate-mirrors: зеркала рядом с тобой...");
            Some(run(true, &[], "rate-mirrors", &["--allow-root", &format!("--save={tmp}"), "arch", "--max-delay=21600"]))
        } else if have("reflector") {
            log("  reflector: свежие зеркала...");
            Some(run(true, &[], "reflector", &["--latest", "40", "--protocol", "https", "--sort", "score", "--save", &tmp]))
        } else {
            None
        };
        match tool {
            Some(Ok(())) => file = fs::read_to_string(&tmp).unwrap_or_default().lines().map(String::from).collect(),
            Some(Err(e)) => log(&format!("  не сработал: {e}")),
            None => {}
        }
        let _ = fs::remove_file(&tmp);
        let mut servers: Vec<String> = file.iter().filter_map(|l| server_url(l)).collect();
        if servers.len() < 5 {
            log("  список зеркал с archlinux.org...");
            servers = arch_status_mirrors()?;
            file = vec![format!("# upd: список с archlinux.org/mirrors/status, {}", fmt_time(now()))];
            file.extend(servers.iter().map(|s| format!("Server = {s}")));
        }
        servers.truncate(n);
        Ok((servers, Some(file)))
    }

    /// Ставит блок закреплённых зеркал наверх. Без изменений файл не трогает — слежение не зацикливается.
    fn apply_mirrors(&self, best: &[String], fallback: Option<&[String]>) -> Result<bool, String> {
        let cur = self.read_list();
        let rest = match fallback {
            Some(f) => f.to_vec(),
            None => split_pin(&cur).1,
        };
        let mut body: Vec<String> = rest.into_iter().filter(|l| server_url(l).map(|u| !contains(best, &u)).unwrap_or(true)).collect();
        while body.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
            body.remove(0);
        }
        let mut nw = vec![PIN_BEGIN.to_string()];
        nw.extend(best.iter().map(|u| format!("Server = {u}")));
        nw.push(PIN_END.into());
        nw.push(String::new());
        nw.extend(body);
        if nw == cur {
            return Ok(false);
        }
        atomic_write(Path::new(&self.mirrorlist), (nw.join("\n") + "\n").as_bytes(), 0o644).map_err(|e| e.to_string())?;
        Ok(true)
    }

    fn remove_mirrors(&self) -> Result<(), String> {
        let mut rest = split_pin(&self.read_list()).1;
        while rest.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
            rest.remove(0);
        }
        atomic_write(Path::new(&self.mirrorlist), (rest.join("\n") + "\n").as_bytes(), 0o644).map_err(|e| e.to_string())
    }

    // --- обновления через временную базу (как checkupdates): системная база не трогается ---

    fn refresh(&self, quiet: bool) -> Result<(), String> {
        let db = Self::sync_db();
        fs::create_dir_all(&db).map_err(|e| e.to_string())?;
        let local = format!("{db}/local");
        if fs::symlink_metadata(&local).is_err() {
            std::os::unix::fs::symlink("/var/lib/pacman/local", &local).map_err(|e| e.to_string())?;
        }
        let mut args = vec!["-Sy", "--dbpath", &db, "--logfile", "/dev/null", "--disable-download-timeout"];
        if out("pacman", &["-Sh"]).0.contains("--disable-sandbox-filesystem") {
            // загрузчик pacman работает от пользователя alpm и не может писать во временную базу
            args.push("--disable-sandbox-filesystem");
        }
        run(quiet, &[], "pacman", &args)
    }

    fn updates(&self) -> Result<Vec<String>, String> {
        // без скачанной временной базы pacman честно скажет «обновлений нет» — это была бы ложь
        if !Path::new(&format!("{}/sync", Self::sync_db())).is_dir() {
            return Err("временная база ещё не скачана".into());
        }
        let (s, code) = out("pacman", &["-Qu", "--dbpath", &Self::sync_db()]);
        if code > 1 {
            return Err(format!("pacman -Qu: код {code}"));
        }
        Ok(lines(&s).into_iter().filter(|l| !l.contains("[ignored]")).collect())
    }

    fn download_size(&self, pkgs: &[String]) -> Option<u64> {
        let db = Self::sync_db();
        let mut args: Vec<&str> = vec!["-Sp", "--print-format", "%s", "--dbpath", &db];
        args.extend(pkgs.iter().map(String::as_str));
        let (s, code) = out("pacman", &args);
        (code == 0).then(|| s.lines().filter_map(|l| l.trim().parse::<u64>().ok()).sum())
    }

    fn prefetch(&self, pkgs: &[String], quiet: bool) -> Result<(), String> {
        let db = Self::sync_db();
        let mut args: Vec<&str> = vec!["-Sw", "--noconfirm", "--disable-download-timeout", "--dbpath", &db, "--logfile", "/dev/null"];
        args.extend(pkgs.iter().map(String::as_str));
        run(quiet, &[], "pacman", &args)
    }

    fn upgrade(&self, aur: bool) -> Result<(), String> {
        if have("garuda-update") {
            // зеркалами управляет upd — garuda-update не должен их перезаписывать
            let mut env = vec![("SKIP_MIRRORLIST", "1")];
            if aur {
                env.push(("UPDATE_AUR", "1"));
            }
            return run(false, &env, "garuda-update", &[]);
        }
        run(false, &[], "pacman", &["-Syu"])
    }
    fn upgrade_handles_aur(&self) -> bool {
        have("garuda-update")
    }

    fn clean(&self) -> Result<(), String> {
        if have("paccache") {
            println!("→ кэш: оставляю 2 последние версии установленных пакетов");
            let _ = run(false, &[], "paccache", &["-rk2"]);
            println!("→ кэш: убираю версии удалённых пакетов");
            let _ = run(false, &[], "paccache", &["-ruk0"]);
        } else {
            let _ = run(false, &[], "pacman", &["-Sc"]);
        }
        let o = self.orphans();
        if o.is_empty() {
            println!("→ ненужных пакетов нет");
            return Ok(());
        }
        println!("→ ненужные пакеты (сироты): {}", o.len());
        let mut args = vec!["-Rns"];
        args.extend(o.iter().map(String::as_str));
        run(false, &[], "pacman", &args)
    }
    fn orphans(&self) -> Vec<String> {
        lines(&out("pacman", &["-Qdtq"]).0)
    }
    fn pending_configs(&self) -> Vec<String> {
        find_etc(&[".pacnew"])
    }
    fn merge(&self) -> Result<(), String> {
        if !have("pacdiff") {
            return Err("нет pacdiff (пакет pacman-contrib)".into());
        }
        run(false, &[], "pacdiff", &[])
    }
    fn cache_dirs(&self) -> Vec<&'static str> {
        vec!["/var/cache/pacman/pkg"]
    }
    fn db_path(&self) -> &'static str {
        "/var/lib/pacman/local"
    }
    fn history(&self, n: usize) -> Vec<String> {
        const KEYS: [&str; 5] = ["] upgraded ", "] installed ", "] removed ", "] downgraded ", "starting full system upgrade"];
        hist_from("/var/log/pacman.log", n, |l| KEYS.iter().any(|k| l.contains(k)))
            .into_iter()
            .map(|l| l.replacen("[ALPM] ", "", 1).replacen("[PACMAN] ", "", 1))
            .collect()
    }
    fn arch_news(&self) -> bool {
        !self.manjaro
    }
    fn last_upgrade(&self) -> i64 {
        tail_file("/var/log/pacman.log", 4 << 20)
            .iter()
            .rev()
            .find(|l| l.contains("starting full system upgrade"))
            .and_then(|l| l.split_whitespace().next().and_then(parse_iso))
            .unwrap_or(0)
    }
    fn auto_snapshots(&self) -> bool {
        Path::new("/usr/share/libalpm/hooks/05-snap-pac-pre.hook").exists() || out("pacman", &["-Q", "snap-pac"]).1 == 0
    }
}

/// Запасной путь без rate-mirrors/reflector: официальный список с оценками.
fn arch_status_mirrors() -> Result<Vec<String>, String> {
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .get("https://archlinux.org/mirrors/status/json/")
        .call()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut list: Vec<(f64, String)> = v["urls"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|u| u["active"] == true && u["protocol"] == "https" && u["completion_pct"].as_f64().unwrap_or(0.0) >= 1.0)
                .filter_map(|u| Some((u["score"].as_f64()?, format!("{}$repo/os/$arch", u["url"].as_str()?))))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(list.into_iter().take(60).map(|x| x.1).collect())
}

// ======================= apt (Debian, Ubuntu, Mint, Pop!_OS…) =======================
//
// Зеркала — через штатный механизм apt «mirror+file:»: источник указывает на файл со списком
// зеркал, apt берёт первое и сам переходит к следующему при ошибке. upd переписывает только этот список.

const APT_LIST: &str = "/etc/apt/upd-mirrors.list";

pub struct Apt {
    distro: String,
    codename: String,
    ubuntu: bool,
}

impl Apt {
    fn new(osr: &BTreeMap<String, String>) -> Self {
        let codename = osr.get("UBUNTU_CODENAME").or(osr.get("VERSION_CODENAME")).cloned().unwrap_or_default();
        let ubuntu = osr.get("ID").map(|s| s == "ubuntu").unwrap_or(false) || osr.get("ID_LIKE").map(|s| s.contains("ubuntu")).unwrap_or(false);
        Apt { distro: osr["PRETTY_NAME"].clone(), codename, ubuntu }
    }

    fn source_files() -> Vec<String> {
        let mut v = vec!["/etc/apt/sources.list".to_string()];
        if let Ok(rd) = fs::read_dir("/etc/apt/sources.list.d") {
            let mut more: Vec<String> = rd
                .flatten()
                .map(|e| e.path().to_string_lossy().into_owned())
                .filter(|p| p.ends_with(".list") || p.ends_with(".sources"))
                .collect();
            more.sort();
            v.extend(more);
        }
        v
    }

    /// Адрес основного архива (suite == codename, не security).
    fn primary_uri(&self) -> Option<String> {
        for f in Self::source_files() {
            let Ok(t) = fs::read_to_string(&f) else { continue };
            if f.ends_with(".sources") {
                for stanza in t.split("\n\n") {
                    let (mut uri, mut suite) = (None, false);
                    for l in stanza.lines() {
                        let Some((k, v)) = l.split_once(':') else { continue };
                        match k.trim() {
                            "URIs" => uri = v.split_whitespace().next().map(String::from),
                            "Suites" => suite |= v.split_whitespace().any(|s| s == self.codename),
                            _ => {}
                        }
                    }
                    if suite && uri.is_some() {
                        return uri;
                    }
                }
                continue;
            }
            for l in t.lines() {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() < 3 || f[0] != "deb" {
                    continue;
                }
                let mut i = 1;
                if f[1].starts_with('[') {
                    while i < f.len() && !f[i].ends_with(']') {
                        i += 1;
                    }
                    i += 1;
                }
                if i + 1 < f.len() && f[i + 1] == self.codename {
                    return Some(f[i].to_string());
                }
            }
        }
        None
    }
}

impl Backend for Apt {
    fn name(&self) -> String {
        format!("{} · apt", self.distro)
    }
    fn mirrors_managed(&self) -> bool {
        !self.codename.is_empty()
    }
    fn mirror_note(&self) -> String {
        "для этой системы зеркала apt не настраиваются автоматически".into()
    }
    fn probe_url(&self, m: &str) -> String {
        format!("{}/dists/{}/InRelease", m.trim_end_matches('/'), self.codename)
    }
    fn fresh_url(&self, m: &str) -> Option<String> {
        Some(self.probe_url(m))
    }
    fn parse_fresh(&self, body: &str) -> Option<i64> {
        body.lines().find_map(|l| l.strip_prefix("Date:")).and_then(parse_rfc2822)
    }
    fn valid_mirror(&self, s: &str) -> Result<(), String> {
        if s.starts_with("http") {
            Ok(())
        } else {
            Err("нужен http(s)://…/debian/ или …/ubuntu/".into())
        }
    }
    fn default_mirrors(&self) -> Vec<String> {
        vec![if self.ubuntu { "http://archive.ubuntu.com/ubuntu/" } else { "https://deb.debian.org/debian/" }.into()]
    }
    fn pinned(&self) -> Vec<String> {
        fs::read_to_string(APT_LIST)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_whitespace().next().filter(|x| !x.starts_with('#')).map(String::from))
            .collect()
    }
    fn list_mirrors(&self) -> Vec<String> {
        self.primary_uri().filter(|u| !u.starts_with("mirror+")).into_iter().collect()
    }
    fn discover(&self, n: usize, log: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
        if !self.ubuntu {
            // у Debian нет гео-списка; deb.debian.org — CDN, он сам ведёт к ближайшему узлу
            return Err("для Debian автопоиска нет, используется CDN deb.debian.org".into());
        }
        log("  mirrors.ubuntu.com: зеркала твоей страны...");
        let body = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .get("http://mirrors.ubuntu.com/mirrors.txt")
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        Ok((body.lines().map(str::trim).filter(|l| l.starts_with("http")).take(n).map(String::from).collect(), None))
    }

    fn apply_mirrors(&self, best: &[String], _: Option<&[String]>) -> Result<bool, String> {
        let prim = self.primary_uri().ok_or("не нашёл основной источник apt")?;
        let orig_file = format!("{}/apt-original-uri", state_dir());
        let mut orig = fs::read_to_string(&orig_file).unwrap_or_default();
        let mut changed = false;
        if !prim.starts_with("mirror+file:") {
            // первый запуск: перевести источники на список зеркал, оригиналы сохранить
            let mut backup: BTreeMap<String, String> = load_json("apt-backup.json");
            for f in Self::source_files() {
                let Ok(t) = fs::read_to_string(&f) else { continue };
                if !t.contains(&prim) {
                    continue;
                }
                backup.entry(f.clone()).or_insert_with(|| t.clone());
                save_json("apt-backup.json", &backup).map_err(|e| e.to_string())?;
                atomic_write(Path::new(&f), t.replace(&prim, &format!("mirror+file:{APT_LIST}")).as_bytes(), 0o644).map_err(|e| e.to_string())?;
            }
            let _ = fs::write(&orig_file, &prim);
            orig = prim;
            changed = true;
        }
        let mut list = best.to_vec();
        let o = orig.trim();
        if !o.is_empty() && !contains(&list, o) {
            list.push(o.to_string()); // исходное зеркало — последним запасным
        }
        let body = format!("# upd: зеркала apt по убыванию скорости (управляется автоматически)\n{}\n", list.join("\n"));
        if fs::read_to_string(APT_LIST).map(|c| c == body).unwrap_or(false) {
            return Ok(changed);
        }
        atomic_write(Path::new(APT_LIST), body.as_bytes(), 0o644).map_err(|e| e.to_string())?;
        Ok(true)
    }

    fn remove_mirrors(&self) -> Result<(), String> {
        let backup: BTreeMap<String, String> = load_json("apt-backup.json");
        for (dst, body) in &backup {
            atomic_write(Path::new(dst), body.as_bytes(), 0o644).map_err(|e| e.to_string())?;
        }
        let _ = fs::remove_file(format!("{}/apt-backup.json", state_dir()));
        let _ = fs::remove_file(APT_LIST);
        Ok(())
    }

    fn refresh(&self, quiet: bool) -> Result<(), String> {
        run(quiet, &[], "apt-get", &["update", "-q"])
    }
    fn updates(&self) -> Result<Vec<String>, String> {
        let (s, code) = out("apt-get", &["-s", "-q", "full-upgrade"]);
        if code != 0 {
            return Err(format!("apt-get -s: код {code}"));
        }
        Ok(s.lines()
            .filter(|l| l.starts_with("Inst "))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                let old = f.get(2).filter(|x| x.starts_with('[')).map(|x| x.trim_matches(|c| c == '[' || c == ']')).unwrap_or("новый");
                let new = f.iter().find(|x| x.starts_with('(')).map(|x| x.trim_start_matches('(')).unwrap_or("");
                format!("{} {old} -> {new}", f.get(1).unwrap_or(&"?"))
            })
            .collect())
    }
    fn download_size(&self, _: &[String]) -> Option<u64> {
        let (s, code) = out("apt-get", &["--print-uris", "-qq", "-y", "full-upgrade"]);
        (code == 0).then(|| s.lines().filter_map(|l| l.split_whitespace().nth(2)?.parse::<u64>().ok()).sum())
    }
    fn prefetch(&self, _: &[String], quiet: bool) -> Result<(), String> {
        run(quiet, &[("DEBIAN_FRONTEND", "noninteractive")], "apt-get", &["-y", "-q", "-d", "full-upgrade"])
    }
    fn upgrade(&self, _: bool) -> Result<(), String> {
        run(false, &[], if have("apt") { "apt" } else { "apt-get" }, &["full-upgrade"])
    }
    fn clean(&self) -> Result<(), String> {
        println!("→ кэш: убираю устаревшие пакеты");
        let _ = run(false, &[], "apt-get", &["autoclean"]);
        println!("→ ненужные пакеты");
        run(false, &[], "apt-get", &["autoremove", "--purge"])
    }
    fn orphans(&self) -> Vec<String> {
        out("apt-get", &["-s", "autoremove"])
            .0
            .lines()
            .filter_map(|l| l.strip_prefix("Remv ").and_then(|x| x.split_whitespace().next()).map(String::from))
            .collect()
    }
    fn pending_configs(&self) -> Vec<String> {
        find_etc(&[".dpkg-dist", ".dpkg-new", ".ucf-dist"])
    }
    fn merge(&self) -> Result<(), String> {
        Err("слияние вручную: сравни файл с его .dpkg-dist".into())
    }
    fn cache_dirs(&self) -> Vec<&'static str> {
        vec!["/var/cache/apt/archives"]
    }
    fn db_path(&self) -> &'static str {
        "/var/lib/dpkg/status"
    }
    fn history(&self, n: usize) -> Vec<String> {
        hist_from("/var/log/apt/history.log", n, |_| true)
    }
}

// ======================= dnf/yum (Fedora, RHEL…) и zypper (openSUSE) =======================
// Зеркала там выбирает сам дистрибутив (metalink / MirrorCache по гео и свежести).

pub struct Rpm {
    distro: String,
    bin: &'static str,
    tumbleweed: bool,
}

impl Rpm {
    fn dnf(osr: &BTreeMap<String, String>) -> Self {
        let bin = if have("dnf5") {
            "dnf5"
        } else if have("dnf") {
            "dnf"
        } else {
            "yum"
        };
        Rpm { distro: osr["PRETTY_NAME"].clone(), bin, tumbleweed: false }
    }
    fn zypper(osr: &BTreeMap<String, String>) -> Self {
        let id = osr.get("ID").cloned().unwrap_or_default();
        Rpm { distro: osr["PRETTY_NAME"].clone(), bin: "zypper", tumbleweed: id.contains("tumbleweed") || id.contains("slowroll") }
    }
    fn zyp(&self) -> bool {
        self.bin == "zypper"
    }
    fn up_cmd(&self) -> &'static str {
        if self.tumbleweed {
            "dup"
        } else if self.zyp() {
            "update"
        } else {
            "upgrade"
        }
    }
}

impl Backend for Rpm {
    fn name(&self) -> String {
        format!("{} · {}", self.distro, self.bin)
    }
    fn mirrors_managed(&self) -> bool {
        false
    }
    fn mirror_note(&self) -> String {
        if self.zyp() {
            "openSUSE сама выбирает ближайшее зеркало (MirrorCache) — настраивать нечего".into()
        } else {
            "Fedora/RHEL сами выбирают зеркала через metalink (гео + свежесть) — настраивать нечего".into()
        }
    }
    fn probe_url(&self, m: &str) -> String {
        m.into()
    }
    fn valid_mirror(&self, _: &str) -> Result<(), String> {
        Err(self.mirror_note())
    }
    fn default_mirrors(&self) -> Vec<String> {
        vec![]
    }
    fn pinned(&self) -> Vec<String> {
        vec![]
    }
    fn list_mirrors(&self) -> Vec<String> {
        vec![]
    }
    fn discover(&self, _: usize, _: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
        Err(self.mirror_note())
    }
    fn apply_mirrors(&self, _: &[String], _: Option<&[String]>) -> Result<bool, String> {
        Ok(false)
    }
    fn remove_mirrors(&self) -> Result<(), String> {
        Ok(())
    }
    fn refresh(&self, quiet: bool) -> Result<(), String> {
        if self.zyp() {
            run(quiet, &[], "zypper", &["-n", "refresh"])
        } else {
            run(quiet, &[], self.bin, &["-q", "makecache"])
        }
    }
    fn updates(&self) -> Result<Vec<String>, String> {
        if self.zyp() {
            let mut args = vec!["-n", "-q", "list-updates"];
            if self.tumbleweed {
                args.push("--all");
            }
            return Ok(out("zypper", &args)
                .0
                .lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('|').map(str::trim).collect();
                    (f.len() >= 5 && f[0] == "v").then(|| format!("{} {} -> {}", f[2], f[3], f[4]))
                })
                .collect());
        }
        let (s, code) = out(self.bin, &["-q", "check-update"]);
        if code != 0 && code != 100 {
            return Err(format!("{} check-update: код {code}", self.bin));
        }
        Ok(s.lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f.len() == 3 && f[0].contains('.') && !l.ends_with(':')).then(|| format!("{} -> {}", f[0], f[1]))
            })
            .collect())
    }
    fn prefetch(&self, _: &[String], quiet: bool) -> Result<(), String> {
        if self.zyp() {
            run(quiet, &[], "zypper", &["-n", self.up_cmd(), "--download-only"])
        } else {
            run(quiet, &[], self.bin, &["-y", "-q", "upgrade", "--downloadonly"])
        }
    }
    fn upgrade(&self, _: bool) -> Result<(), String> {
        run(false, &[], self.bin, &[self.up_cmd()])
    }
    fn clean(&self) -> Result<(), String> {
        if self.zyp() {
            return run(false, &[], "zypper", &["clean", "--all"]);
        }
        let _ = run(false, &[], self.bin, &["clean", "packages"]);
        run(false, &[], self.bin, &["autoremove"])
    }
    fn orphans(&self) -> Vec<String> {
        if !self.zyp() {
            return vec![];
        }
        out("zypper", &["-q", "packages", "--unneeded"])
            .0
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('|').map(str::trim).collect();
                (f.len() > 3 && f[0].starts_with('i')).then(|| f[2].to_string())
            })
            .collect()
    }
    fn pending_configs(&self) -> Vec<String> {
        find_etc(&[".rpmnew"])
    }
    fn merge(&self) -> Result<(), String> {
        Err("слияние вручную: сравни файл с его .rpmnew".into())
    }
    fn cache_dirs(&self) -> Vec<&'static str> {
        if self.zyp() {
            vec!["/var/cache/zypp/packages"]
        } else {
            vec!["/var/cache/dnf", "/var/cache/libdnf5"]
        }
    }
    fn db_path(&self) -> &'static str {
        "/var/lib/rpm"
    }
    fn history(&self, n: usize) -> Vec<String> {
        let p = if self.zyp() { "/var/log/zypp/history" } else { "/var/log/dnf.rpm.log" };
        hist_from(p, n, |l| !l.starts_with('#'))
    }
}
