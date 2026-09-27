//! Всё, что зависит от пакетного менеджера: pacman, apt, dnf/yum, zypper.

use crate::common::*;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub enum ProbeKind {
    Generic,
    PacmanDb,
    AptInRelease,
}

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
    fn probe_kind(&self) -> ProbeKind {
        ProbeKind::Generic
    }
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
    /// Спрашивает ли установщик подтверждение сам; если нет — спрашивает upd
    fn upgrade_asks(&self) -> bool {
        true
    }
    /// Доступен ли AUR (Arch и производные)
    fn aur(&self) -> bool {
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
        Err(t!("{}: пакетный менеджер не поддерживается (есть: pacman, apt, dnf, zypper)", osr["PRETTY_NAME"]))
    }
}

struct PrivateTempDir(PathBuf);

impl PrivateTempDir {
    fn new() -> std::io::Result<Self> {
        for _ in 0..8 {
            let mut random = [0u8; 16];
            fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
            let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let path = PathBuf::from(format!("/tmp/upd-mirrors-{suffix}"));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o700)) {
                        let _ = fs::remove_dir_all(&path);
                        return Err(e);
                    }
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, t!("не удалось подобрать имя временного каталога")))
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hist_from(path: &str, n: usize, keep: impl Fn(&str) -> bool) -> Vec<String> {
    tail_file(path, 512 << 10).into_iter().rev().filter(|l| !l.trim().is_empty() && keep(l)).take(n).collect()
}

// ======================= pacman (Arch, Garuda, EndeavourOS, CachyOS…) =======================

pub(crate) const PIN_BEGIN: &str = "## >>> upd: pinned mirrors (managed automatically, see upd) >>>";
/// Начало блока до 0.2.5 — узнаём, при следующей записи блок получит новую метку
const PIN_BEGIN_OLD: &str = "## >>> upd: закреплённые зеркала (управляется автоматически, см. upd) >>>";
pub(crate) const PIN_END: &str = "## <<< upd <<<";

pub struct Pacman {
    mirrorlist: String,
    distro: String,
    manjaro: bool,
    /// Почему upd не трогает mirrorlist; None — зеркала Arch, управляем
    unmanaged: Option<String>,
}

impl Pacman {
    fn new(osr: &BTreeMap<String, String>) -> Self {
        let mirrorlist = env_or("UPD_MIRRORLIST", "/etc/pacman.d/mirrorlist");
        let field = |k: &str| osr.get(k).map(String::as_str).unwrap_or("");
        // Manjaro и его редакции/форки (BigLinux и т.п.): свои зеркала с $branch и pacman-mirrors
        let manjaro = field("ID").starts_with("manjaro") || field("ID_LIKE").split_whitespace().any(|s| s == "manjaro") || have("pacman-mirrors");
        let conf = fs::read_to_string(env_or("UPD_PACMAN_CONF", "/etc/pacman.conf")).unwrap_or_default();
        let unmanaged = if manjaro {
            Some(t!("в Manjaro зеркалами управляет pacman-mirrors (sudo pacman-mirrors --fasttrack)").to_string())
        } else if !conf.is_empty() && !repo_uses(&conf, "core", &mirrorlist) {
            // Artix и подобные: в mirrorlist зеркала своих репозиториев, а не Arch — подменять нельзя
            Some(t!("{0} не подключён к репозиторию Arch [core] — зеркалами управляет дистрибутив", mirrorlist))
        } else {
            None
        };
        Pacman { mirrorlist, distro: field("PRETTY_NAME").to_string(), manjaro, unmanaged }
    }

    fn read_list(&self) -> Vec<String> {
        fs::read_to_string(&self.mirrorlist).map(|s| s.trim_end_matches('\n').lines().map(String::from).collect()).unwrap_or_default()
    }

    fn sync_db() -> String {
        format!("{}/syncdb", state_dir())
    }
}

/// Берёт ли репозиторий [repo] из pacman.conf зеркала из файла mirrorlist.
fn repo_uses(conf: &str, repo: &str, mirrorlist: &str) -> bool {
    let mut inside = false;
    for l in conf.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == format!("[{repo}]");
        } else if inside {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == "Include" && v.trim() == mirrorlist {
                    return true;
                }
            }
        }
    }
    false
}

/// Загружена ли система из снапшота btrfs (snapper, timeshift): изменения пропадут при перезагрузке.
fn booted_from_snapshot() -> bool {
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    cmdline.split_whitespace().any(|a| (a.starts_with("rootflags=") || a.starts_with("subvol=")) && (a.contains("/.snapshots/") || a.contains("timeshift-btrfs/snapshots")))
}

fn server_url(l: &str) -> Option<String> {
    let (k, v) = l.trim().split_once('=')?;
    (k.trim() == "Server").then(|| v.trim().to_string())
}

/// Делит mirrorlist на закреплённые зеркала и остальной файл.
fn split_pin(ls: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut pinned, mut rest, mut inside) = (vec![], vec![], false);
    for l in ls {
        if l == PIN_BEGIN || l == PIN_BEGIN_OLD {
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
        self.unmanaged.is_none()
    }
    fn mirror_note(&self) -> String {
        self.unmanaged.clone().unwrap_or_default()
    }
    fn watch_path(&self) -> Option<String> {
        Some(self.mirrorlist.clone())
    }
    fn probe_kind(&self) -> ProbeKind {
        ProbeKind::PacmanDb
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
            Err(t!("нужен http(s)://…/$repo/os/$arch").into())
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
        let mut file: Vec<String> = vec![];
        let rate_mirrors = have("rate-mirrors");
        let reflector = !rate_mirrors && have("reflector");
        if rate_mirrors || reflector {
            log(if rate_mirrors { t!("  rate-mirrors: зеркала рядом с тобой...") } else { t!("  reflector: свежие зеркала...") });
            match PrivateTempDir::new() {
                Ok(tmp_dir) => {
                    let output = tmp_dir.0.join("mirrors");
                    let output_arg = output.to_string_lossy().into_owned();
                    let result = if rate_mirrors {
                        let save_arg = format!("--save={output_arg}");
                        run(true, &[], "rate-mirrors", &["--allow-root", &save_arg, "arch", "--max-delay=21600"])
                    } else {
                        run(true, &[], "reflector", &["--latest", "40", "--protocol", "https", "--sort", "score", "--save", &output_arg])
                    };
                    match result {
                        Ok(()) => match fs::read_to_string(&output) {
                            Ok(body) => file = body.lines().map(String::from).collect(),
                            Err(e) => log(&t!("  не удалось прочитать результат автопоиска: {0}", e)),
                        },
                        Err(e) => log(&t!("  не сработал: {0}", e)),
                    }
                }
                Err(e) => log(&t!("  не удалось создать закрытый временный каталог: {0}", e)),
            }
        }
        let mut servers: Vec<String> = file.iter().filter_map(|l| server_url(l)).collect();
        if servers.len() < 5 {
            log(t!("  список зеркал с archlinux.org..."));
            servers = arch_status_mirrors()?;
            file = vec![format!("# upd: list from archlinux.org/mirrors/status, {}", fmt_time(now()))];
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
            return Err(t!("временная база ещё не скачана").into());
        }
        // обрезанный stdout — ошибка, а не неполный список обновлений
        let output = capture(Command::new("pacman").args(["-Qu", "--dbpath", &Self::sync_db()]).env("LC_ALL", "C"), Some(OUT_MAX))
            .map_err(|e| t!("pacman -Qu: не удалось запустить: {0}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = last_line(&stderr).map(|line| format!(": {line}")).unwrap_or_default();
        let Some(code) = output.status.code() else {
            return Err(t!("pacman -Qu: процесс завершился без кода выхода{0}", detail));
        };
        let updates: Vec<String> = lines(&stdout).into_iter().filter(|line| !line.contains("[ignored]")).collect();
        if code > 1 || (code == 1 && (!stderr.trim().is_empty() || !updates.is_empty())) {
            return Err(t!("pacman -Qu: код {0}{1}", code, detail));
        }
        Ok(updates)
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
            // подтверждение уже спросил upd (ответ pacman «н» в русской раскладке отменял установку);
            // конфликты пакетов auto-pacman из garuda-update разбирает сам
            return run(false, &env, "garuda-update", &["--noconfirm"]);
        }
        if booted_from_snapshot() {
            return Err(t!("система загружена из снапшота — изменения пропадут при перезагрузке. Сначала восстанови снапшот (snapper rollback / btrfs-assistant) и перезагрузись").into());
        }
        // Как в garuda-update и eos-update: сначала ключи (archlinux-, manjaro-, endeavouros-, cachyos-, chaotic-keyring…),
        // иначе после смены ключа сборщика всё обновление падает на проверке подписей.
        // -Sy с последующим -S ключей и -Su — рекомендованное Arch исключение из запрета частичных обновлений.
        run(false, &[], "pacman", &["-Sy"])?;
        let (outdated, _) = out("pacman", &["-Qqu"]);
        let keyrings: Vec<&str> = outdated.lines().map(str::trim).filter(|p| p.ends_with("-keyring")).collect();
        if !keyrings.is_empty() {
            println!("{}", t!("→ сначала ключи: {}", keyrings.join(", ")));
            let mut args = vec!["-S", "--needed", "--noconfirm"];
            args.extend(&keyrings);
            run(false, &[], "pacman", &args)?;
        }
        // подтверждение уже спросил upd; на конфликтах --noconfirm выбирает безопасное «нет» и прерывает установку
        run(false, &[], "pacman", &["-Su", "--noconfirm"]).map_err(|e| t!("{0} — если pacman спрашивал про конфликт пакетов, запусти вручную: sudo pacman -Syu", e))
    }
    fn upgrade_handles_aur(&self) -> bool {
        have("garuda-update")
    }
    fn upgrade_asks(&self) -> bool {
        false
    }
    fn aur(&self) -> bool {
        true
    }

    fn clean(&self) -> Result<(), String> {
        if have("paccache") {
            println!("{}", t!("→ кэш: оставляю 2 последние версии установленных пакетов"));
            let _ = run(false, &[], "paccache", &["-rk2"]);
            println!("{}", t!("→ кэш: убираю версии удалённых пакетов"));
            let _ = run(false, &[], "paccache", &["-ruk0"]);
        } else {
            let _ = run(false, &[], "pacman", &["-Sc"]);
        }
        let o = self.orphans();
        if o.is_empty() {
            println!("{}", t!("→ ненужных пакетов нет"));
            return Ok(());
        }
        println!("{}", t!("→ ненужные пакеты (сироты): {}", o.len()));
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
            return Err(t!("нет pacdiff (пакет pacman-contrib)").into());
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
    let v: serde_json::Value = serde_json::from_str(&read_text(resp, 16 << 20)?).map_err(|e| e.to_string())?;
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
const APT_BACKUP: &str = "apt-backup.json";
const APT_ORIGINAL_URI: &str = "apt-original-uri";

fn apt_backup_dir() -> PathBuf {
    PathBuf::from(state_dir()).join("private")
}

fn apt_backup_path() -> PathBuf {
    apt_backup_dir().join(APT_BACKUP)
}

fn legacy_apt_backup_path() -> PathBuf {
    PathBuf::from(state_dir()).join(APT_BACKUP)
}

fn apt_original_uri_path() -> PathBuf {
    apt_backup_dir().join(APT_ORIGINAL_URI)
}

fn legacy_apt_original_uri_path() -> PathBuf {
    PathBuf::from(state_dir()).join(APT_ORIGINAL_URI)
}

fn load_apt_original_uri() -> Result<String, String> {
    let private = apt_original_uri_path();
    let legacy = legacy_apt_original_uri_path();
    ensure_private_dir(&apt_backup_dir())?;
    let has_private = secure_backup_file(&private)?;
    let has_legacy = secure_backup_file(&legacy)?;
    if has_private {
        let uri = fs::read_to_string(&private).map_err(|e| format!("{}: {e}", private.display()))?;
        if has_legacy {
            remove_file_if_exists(&legacy)?;
        }
        return Ok(uri);
    }
    if has_legacy {
        let uri = fs::read_to_string(&legacy).map_err(|e| format!("{}: {e}", legacy.display()))?;
        atomic_write(&private, uri.as_bytes(), 0o600).map_err(|e| e.to_string())?;
        sync_parent_dir(&private)?;
        remove_file_if_exists(&legacy)?;
        return Ok(uri);
    }
    Ok(String::new())
}

fn peek_apt_original_uri() -> Result<String, String> {
    for path in [apt_original_uri_path(), legacy_apt_original_uri_path()] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => return fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display())),
            Ok(_) => return Err(t!("{}: ожидался обычный файл", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(String::new())
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent_dir(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AptSourceVersion {
    content: String,
    mode: u32,
    uid: u32,
    gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AptSourceBackup {
    content: String,
    mode: u32,
    uid: u32,
    gid: u32,
    #[serde(default)]
    applied: Option<AptSourceVersion>,
}

const APT_TRANSACTION: &str = "apt-transaction.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct AptTransaction {
    files: BTreeMap<String, Option<AptSourceBackup>>,
    prepared_file: Option<String>,
}

static APT_SOURCE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn apt_transaction_path() -> PathBuf {
    apt_backup_dir().join(APT_TRANSACTION)
}

fn sync_parent_dir(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| t!("{}: нет родительского каталога", path.display()))?;
    fs::File::open(parent).and_then(|dir| dir.sync_all()).map_err(|e| format!("{}: {e}", parent.display()))
}

fn apt_is_source_path(path: &Path) -> bool {
    path == Path::new("/etc/apt/sources.list")
        || (path.parent() == Some(Path::new("/etc/apt/sources.list.d")) && matches!(path.extension().and_then(|ext| ext.to_str()), Some("list" | "sources")))
}

fn apt_transaction_path_allowed(path: &Path) -> bool {
    path == Path::new(APT_LIST)
        || path == apt_backup_path().as_path()
        || path == legacy_apt_backup_path().as_path()
        || path == apt_original_uri_path().as_path()
        || path == legacy_apt_original_uri_path().as_path()
        || apt_is_source_path(path)
}

fn apt_temp_path_allowed(path: &Path) -> bool {
    path.parent() == Path::new(APT_LIST).parent()
        && path.file_name().and_then(|name| name.to_str()).map(|name| name.starts_with(".upd-source-") && name.ends_with(".tmp")).unwrap_or(false)
}

fn snapshot_apt_file(path: &Path) -> Result<Option<AptSourceBackup>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let content = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Some(AptSourceBackup {
                content,
                mode: metadata.permissions().mode() & 0o7777,
                uid: metadata.uid(),
                gid: metadata.gid(),
                applied: None,
            }))
        }
        Ok(_) => Err(t!("{}: ожидался обычный файл", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn write_apt_transaction(transaction: &AptTransaction) -> Result<(), String> {
    let path = apt_transaction_path();
    ensure_private_dir(&apt_backup_dir())?;
    secure_backup_file(&path)?;
    let data = serde_json::to_vec_pretty(transaction).map_err(|e| e.to_string())?;
    atomic_write(&path, &data, 0o600).map_err(|e| e.to_string())?;
    sync_parent_dir(&path)
}

fn rollback_apt_transaction(transaction: &AptTransaction) -> Result<(), String> {
    let mut paths: Vec<_> = transaction.files.iter().collect();
    paths.sort_by_key(|(path, _)| apt_is_source_path(Path::new(path)));
    let mut errors = Vec::new();
    for (path, previous) in paths {
        let path = Path::new(path);
        if !apt_transaction_path_allowed(path) {
            errors.push(t!("{}: путь отката APT не разрешён", path.display()));
            continue;
        }
        let result = match snapshot_apt_file(path) {
            Ok(current) if &current == previous => continue,
            Err(e) => Err(e),
            Ok(_) => match previous {
                Some(file) => atomic_write_apt_source(path, file.content.as_bytes(), file.mode, file.uid, file.gid),
                None => remove_file_if_exists(path),
            },
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    if let Some(prepared) = &transaction.prepared_file {
        let path = Path::new(prepared);
        if !apt_temp_path_allowed(path) {
            errors.push(t!("{}: временный путь APT не разрешён", path.display()));
        } else if let Err(e) = remove_file_if_exists(path) {
            errors.push(e);
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    remove_file_if_exists(&apt_transaction_path())
}

fn recover_apt_transaction() -> Result<(), String> {
    ensure_private_dir(&apt_backup_dir())?;
    let path = apt_transaction_path();
    if !secure_backup_file(&path)? {
        return Ok(());
    }
    let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let transaction: AptTransaction = serde_json::from_slice(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    if transaction.files.keys().any(|p| !apt_transaction_path_allowed(Path::new(p)))
        || transaction.prepared_file.as_deref().map(|p| !apt_temp_path_allowed(Path::new(p))).unwrap_or(false)
    {
        return Err(t!("{}: журнал APT содержит недопустимый путь", path.display()));
    }
    rollback_apt_transaction(&transaction)
}

fn run_apt_transaction<T>(paths: &[PathBuf], prepared_file: Option<&Path>, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    recover_apt_transaction()?;
    if let Some(path) = prepared_file {
        if !apt_temp_path_allowed(path) {
            return Err(t!("{}: временный путь транзакции APT не разрешён", path.display()));
        }
    }
    let mut files = BTreeMap::new();
    for path in paths {
        if !apt_transaction_path_allowed(path) {
            return Err(t!("{}: путь транзакции APT не разрешён", path.display()));
        }
        files.insert(path.to_string_lossy().into_owned(), snapshot_apt_file(path)?);
    }
    let transaction = AptTransaction { files, prepared_file: prepared_file.map(|path| path.to_string_lossy().into_owned()) };
    if let Err(e) = write_apt_transaction(&transaction) {
        if let Some(path) = prepared_file {
            let _ = remove_file_if_exists(path);
        }
        return Err(e);
    }
    match action() {
        Err(error) => match rollback_apt_transaction(&transaction) {
            Ok(()) => Err(error),
            Err(rollback) => Err(t!("{0}; откат APT не завершён: {1}", error, rollback)),
        },
        Ok(value) => match remove_file_if_exists(&apt_transaction_path()) {
            Ok(()) => Ok(value),
            Err(error) => {
                let journal_error = write_apt_transaction(&transaction).err();
                let rollback = rollback_apt_transaction(&transaction);
                let details = journal_error.map(|e| t!("; журнал не восстановлен: {0}", e)).unwrap_or_default();
                match rollback {
                    Ok(()) => Err(t!("не удалось зафиксировать транзакцию APT: {0}{1}", error, details)),
                    Err(e) => Err(t!("не удалось зафиксировать транзакцию APT: {0}{1}; откат: {2}", error, details, e)),
                }
            }
        },
    }
}

/// Подготовить APT-файл закрытым временным файлом и синхронизировать его до rename.
fn prepare_apt_file(path: &Path, data: &[u8], mode: u32, uid: u32, gid: u32) -> Result<PathBuf, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut created = None;
    for _ in 0..8 {
        let n = APT_SOURCE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".upd-source-{}-{n}.tmp", std::process::id()));
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp) {
            Ok(file) => {
                created = Some((tmp, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    let Some((tmp, mut file)) = created else {
        return Err(t!("{}: не удалось создать временный файл", path.display()));
    };
    let result = (|| -> std::io::Result<()> {
        file.write_all(data)?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        if metadata.uid() != uid || metadata.gid() != gid {
            chown(&tmp, Some(uid), Some(gid))?;
        }
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    if let Err(e) = sync_parent_dir(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

fn install_prepared_apt_file(prepared: &Path, path: &Path) -> Result<(), String> {
    fs::rename(prepared, path).map_err(|e| format!("{}: {e}", path.display()))?;
    sync_parent_dir(path)
}

/// Записать APT source атомарно: временный файл остаётся закрытым до установки owner и mode.
fn atomic_write_apt_source(path: &Path, data: &[u8], mode: u32, uid: u32, gid: u32) -> Result<(), String> {
    let tmp = prepare_apt_file(path, data, mode, uid, gid)?;
    if let Err(e) = install_prepared_apt_file(&tmp, path) {
        let _ = remove_file_if_exists(&tmp);
        return Err(e);
    }
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or(t!("нет родительского каталога для APT backup"))?;
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(t!("{}: ожидался каталог", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.file_type().is_dir() {
        return Err(t!("{}: ожидался каталог", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", path.display()))?;
    sync_parent_dir(path)
}

/// Проверить обычный файл и закрыть его до чтения или перезаписи.
fn secure_backup_file(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return Err(t!("{}: ожидался обычный файл", path.display()));
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn read_apt_backup(path: &Path) -> Result<BTreeMap<String, AptSourceBackup>, String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let raw: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    raw.into_iter()
        .map(|(source, entry)| {
            let backup = if let Some(content) = entry.as_str() {
                // Старый формат хранил только текст; его прежнее поведение было 0644 root:root.
                AptSourceBackup { content: content.to_string(), mode: 0o644, uid: 0, gid: 0, applied: None }
            } else {
                serde_json::from_value(entry).map_err(|e| format!("{}: {e}", path.display()))?
            };
            Ok((source, backup))
        })
        .collect()
}

fn merge_apt_source_after_apply(path: &str, original: &AptSourceBackup, current: AptSourceBackup, original_uri: &str) -> Result<AptSourceBackup, String> {
    let reference = format!("mirror+file:{APT_LIST}");
    if current.content == original.content {
        return Ok(current);
    }
    let legacy_applied = if original.applied.is_none() && !original_uri.is_empty() {
        Some(AptSourceVersion {
            content: original.content.replace(original_uri, &reference),
            mode: original.mode,
            uid: original.uid,
            gid: original.gid,
        })
    } else {
        None
    };
    let applied = original.applied.as_ref().or(legacy_applied.as_ref());
    if let Some(applied) = applied {
        if current.content == applied.content {
            let metadata_unchanged = current.mode == applied.mode && current.uid == applied.uid && current.gid == applied.gid;
            return Ok(AptSourceBackup {
                content: original.content.clone(),
                mode: if metadata_unchanged { original.mode } else { current.mode },
                uid: if metadata_unchanged { original.uid } else { current.uid },
                gid: if metadata_unchanged { original.gid } else { current.gid },
                applied: None,
            });
        }
    }
    if current.content.contains(&reference) && original_uri.is_empty() {
        return Err(t!("{0}: нет исходного URI для восстановления; backup сохранён", path));
    }
    let restored = if let Some(applied) = applied {
        let applied_lines: Vec<&str> = applied.content.split_inclusive('\n').collect();
        let original_lines: Vec<&str> = original.content.split_inclusive('\n').collect();
        let mut next_applied = 0;
        let mut merged = String::new();
        for line in current.content.split_inclusive('\n') {
            let found = (next_applied..applied_lines.len()).find(|index| applied_lines[*index] == line);
            if let Some(index) = found {
                next_applied = index + 1;
                if line.contains(&reference) {
                    let Some(original_line) = original_lines.get(index) else {
                        return Err(t!("{0}: applied snapshot не совпадает с backup; backup сохранён для ручного merge", path));
                    };
                    if original_line.contains(&reference) {
                        return Err(t!("{0}: исходный source уже ссылался на список upd; backup сохранён для ручного merge", path));
                    }
                    merged.push_str(original_line);
                } else {
                    merged.push_str(line);
                }
            } else {
                if line.contains(&reference) {
                    return Err(t!("{0}: source изменён после применения upd и всё ещё ссылается на его список зеркал; backup сохранён", path));
                }
                merged.push_str(line);
            }
        }
        merged
    } else if current.content.contains(&reference) {
        if original.content.contains(&reference) {
            return Err(t!("{0}: нельзя отличить исходную ссылку от подстановки upd; backup сохранён для ручного merge", path));
        }
        current.content.replace(&reference, original_uri)
    } else {
        current.content.clone()
    };
    if restored.contains(&reference) {
        return Err(t!("{0}: после восстановления остаётся ссылка на список зеркал upd; backup сохранён", path));
    }
    Ok(AptSourceBackup { content: restored, mode: current.mode, uid: current.uid, gid: current.gid, applied: None })
}

fn restore_apt_source(path: &str, original: &AptSourceBackup, original_uri: &str) -> Result<(), String> {
    let path_ref = Path::new(path);
    let Some(current) = snapshot_apt_file(path_ref)? else {
        return Ok(()); // Администратор удалил source-файл после установки upd.
    };
    let restored = merge_apt_source_after_apply(path, original, current.clone(), original_uri)?;
    if restored == current {
        return Ok(());
    }
    atomic_write_apt_source(path_ref, restored.content.as_bytes(), restored.mode, restored.uid, restored.gid)
}

fn save_apt_backup(backup: &BTreeMap<String, AptSourceBackup>) -> Result<(), String> {
    let dir = apt_backup_dir();
    ensure_private_dir(&dir)?;
    let path = apt_backup_path();
    secure_backup_file(&path)?;
    let data = serde_json::to_vec_pretty(backup).map_err(|e| e.to_string())?;
    atomic_write(&path, &data, 0o600).map_err(|e| e.to_string())?;
    sync_parent_dir(&path)
}

fn load_apt_backup() -> Result<BTreeMap<String, AptSourceBackup>, String> {
    let dir = apt_backup_dir();
    ensure_private_dir(&dir)?;
    let path = apt_backup_path();
    let legacy = legacy_apt_backup_path();
    let has_private = secure_backup_file(&path)?;
    let has_legacy = secure_backup_file(&legacy)?;
    if has_private {
        let backup = read_apt_backup(&path)?;
        if has_legacy {
            remove_file_if_exists(&legacy)?;
        }
        return Ok(backup);
    }
    if has_legacy {
        let backup = read_apt_backup(&legacy)?;
        save_apt_backup(&backup)?;
        remove_file_if_exists(&legacy)?;
        return Ok(backup);
    }
    Ok(BTreeMap::new())
}

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
        t!("для этой системы зеркала apt не настраиваются автоматически").into()
    }
    fn probe_kind(&self) -> ProbeKind {
        ProbeKind::AptInRelease
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
            Err(t!("нужен http(s)://…/debian/ или …/ubuntu/").into())
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
            return Err(t!("для Debian автопоиска нет, используется CDN deb.debian.org").into());
        }
        log(t!("  mirrors.ubuntu.com: зеркала твоей страны..."));
        let resp = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .get("http://mirrors.ubuntu.com/mirrors.txt")
            .call()
            .map_err(|e| e.to_string())?;
        let body = read_text(resp, 1 << 20)?;
        Ok((body.lines().map(str::trim).filter(|l| l.starts_with("http")).take(n).map(String::from).collect(), None))
    }

    fn apply_mirrors(&self, best: &[String], _: Option<&[String]>) -> Result<bool, String> {
        recover_apt_transaction()?;
        let prim = self.primary_uri().ok_or(t!("не нашёл основной источник apt"))?;
        let switching_to_mirror_file = !prim.starts_with("mirror+file:");
        let mut original_uri = if switching_to_mirror_file { prim.clone() } else { peek_apt_original_uri()? };
        if original_uri.is_empty() && switching_to_mirror_file {
            original_uri = prim.clone();
        }
        let mut list = best.to_vec();
        let o = original_uri.trim();
        if !o.is_empty() && !contains(&list, o) {
            list.push(o.to_string()); // исходное зеркало — последним запасным
        }
        let body = format!("# upd: apt mirrors by descending speed (managed automatically)\n{}\n", list.join("\n"));
        let list_changed = !fs::read_to_string(APT_LIST).map(|current| current == body).unwrap_or(false);
        if !switching_to_mirror_file && !list_changed {
            return Ok(false);
        }
        let mut sources = Vec::new();
        if switching_to_mirror_file {
            for file in Self::source_files() {
                let Ok(content) = fs::read_to_string(&file) else { continue };
                if !content.contains(&prim) {
                    continue;
                }
                let metadata = fs::metadata(&file).map_err(|e| format!("{file}: {e}"))?;
                sources.push((
                    file,
                    content,
                    AptSourceBackup { content: String::new(), mode: metadata.permissions().mode() & 0o7777, uid: metadata.uid(), gid: metadata.gid(), applied: None },
                ));
            }
        }
        let prepared = prepare_apt_file(Path::new(APT_LIST), body.as_bytes(), 0o644, 0, 0)?;
        let mut paths = vec![
            PathBuf::from(APT_LIST),
            apt_backup_path(),
            legacy_apt_backup_path(),
            apt_original_uri_path(),
            legacy_apt_original_uri_path(),
        ];
        paths.extend(sources.iter().map(|(file, _, _)| PathBuf::from(file)));
        let result = run_apt_transaction(&paths, Some(&prepared), || {
            let stored_original_uri = load_apt_original_uri()?;
            if !switching_to_mirror_file && stored_original_uri != original_uri {
                return Err(t!("исходный URI APT изменился во время подготовки зеркал").into());
            }
            let mut backup = load_apt_backup()?;
            if backup.keys().any(|path| !apt_is_source_path(Path::new(path))) {
                return Err(t!("backup APT содержит недопустимый путь source-файла").into());
            }
            if switching_to_mirror_file {
                let reference = format!("mirror+file:{APT_LIST}");
                let mut replacements = Vec::new();
                for (file, content, metadata) in &sources {
                    let current_text = fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
                    let current_meta = fs::metadata(file).map_err(|e| format!("{file}: {e}"))?;
                    if current_text.as_str() != content.as_str()
                        || current_meta.permissions().mode() & 0o7777 != metadata.mode
                        || current_meta.uid() != metadata.uid
                        || current_meta.gid() != metadata.gid
                    {
                        return Err(t!("{0}: source изменился во время подготовки зеркал", file));
                    }
                    let current = AptSourceBackup { content: content.clone(), mode: metadata.mode, uid: metadata.uid, gid: metadata.gid, applied: None };
                    let baseline = match backup.get(file) {
                        Some(previous) => merge_apt_source_after_apply(file, previous, current, &stored_original_uri)?,
                        None => current,
                    };
                    let original = backup.entry(file.clone()).or_insert_with(|| baseline.clone());
                    // A direct or edited source is the new baseline before this application.
                    original.content = baseline.content;
                    original.mode = baseline.mode;
                    original.uid = baseline.uid;
                    original.gid = baseline.gid;
                    let replaced = content.replace(&prim, &reference);
                    original.applied = Some(AptSourceVersion { content: replaced.clone(), mode: original.mode, uid: original.uid, gid: original.gid });
                    replacements.push((file.clone(), replaced, original.mode, original.uid, original.gid));
                }
                if !sources.is_empty() {
                    save_apt_backup(&backup)?;
                }
                ensure_private_dir(&apt_backup_dir())?;
                atomic_write(&apt_original_uri_path(), prim.as_bytes(), 0o600).map_err(|e| e.to_string())?;
                sync_parent_dir(&apt_original_uri_path())?;
                install_prepared_apt_file(&prepared, Path::new(APT_LIST))?;
                for (file, replaced, mode, uid, gid) in replacements {
                    atomic_write_apt_source(Path::new(&file), replaced.as_bytes(), mode, uid, gid)?;
                }
            } else {
                install_prepared_apt_file(&prepared, Path::new(APT_LIST))?;
            }
            Ok(true)
        });
        if result.is_err() {
            let _ = remove_file_if_exists(&prepared);
        }
        result
    }

    fn remove_mirrors(&self) -> Result<(), String> {
        recover_apt_transaction()?;
        let backup = load_apt_backup()?;
        let mut paths = vec![
            PathBuf::from(APT_LIST),
            apt_backup_path(),
            legacy_apt_backup_path(),
            apt_original_uri_path(),
            legacy_apt_original_uri_path(),
        ];
        for dst in backup.keys() {
            let path = PathBuf::from(dst);
            if !apt_is_source_path(&path) {
                return Err(t!("{0}: недопустимый путь APT source в backup", dst));
            }
            paths.push(path);
        }
        run_apt_transaction(&paths, None, || {
            let original_uri = load_apt_original_uri()?;
            for (dst, original) in &backup {
                restore_apt_source(dst, original, &original_uri)?;
            }
            let reference = format!("mirror+file:{APT_LIST}");
            for file in Self::source_files() {
                match fs::read_to_string(&file) {
                    Ok(content) if content.contains(&reference) => return Err(t!("{0}: источник всё ещё ссылается на список зеркал upd", file)),
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(t!("{0}: не удалось проверить источник APT: {1}", file, e)),
                }
            }
            remove_file_if_exists(&apt_backup_path())?;
            remove_file_if_exists(&legacy_apt_backup_path())?;
            remove_file_if_exists(&apt_original_uri_path())?;
            remove_file_if_exists(&legacy_apt_original_uri_path())?;
            remove_file_if_exists(Path::new(APT_LIST))?;
            Ok(())
        })
    }

    fn refresh(&self, quiet: bool) -> Result<(), String> {
        run(quiet, &[], "apt-get", &["update", "-q"])
    }
    fn updates(&self) -> Result<Vec<String>, String> {
        let (s, code) = out_limited("apt-get", &["-s", "-q", "full-upgrade"], OUT_MAX)?;
        if code != 0 {
            return Err(t!("apt-get -s: код {0}", code));
        }
        Ok(s.lines()
            .filter(|l| l.starts_with("Inst "))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                let old = f.get(2).filter(|x| x.starts_with('[')).map(|x| x.trim_matches(|c| c == '[' || c == ']')).unwrap_or(t!("новый"));
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
        println!("{}", t!("→ кэш: убираю устаревшие пакеты"));
        let _ = run(false, &[], "apt-get", &["autoclean"]);
        println!("{}", t!("→ ненужные пакеты"));
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
        Err(t!("слияние вручную: сравни файл с его .dpkg-dist").into())
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
            t!("openSUSE сама выбирает ближайшее зеркало (MirrorCache) — настраивать нечего").into()
        } else {
            t!("Fedora/RHEL сами выбирают зеркала через metalink (гео + свежесть) — настраивать нечего").into()
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
            let (s, code) = out_limited("zypper", &args, OUT_MAX)?;
            if code != 0 {
                return Err(t!("zypper list-updates: код {0}", code));
            }
            return Ok(s
                .lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('|').map(str::trim).collect();
                    (f.len() >= 5 && f[0] == "v").then(|| format!("{} {} -> {}", f[2], f[3], f[4]))
                })
                .collect());
        }
        let (s, code) = out_limited(self.bin, &["-q", "check-update"], OUT_MAX)?;
        if code != 0 && code != 100 {
            return Err(t!("{} check-update: код {1}", self.bin, code));
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
        Err(t!("слияние вручную: сравни файл с его .rpmnew").into())
    }
    fn cache_dirs(&self) -> Vec<&'static str> {
        if self.zyp() {
            vec!["/var/cache/zypp/packages"]
        } else if self.bin == "dnf5" {
            vec!["/var/cache/libdnf5"]
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

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn pacman_mirrorlist_managed_only_for_arch_core() {
        let ml = "/etc/pacman.d/mirrorlist";
        let arch = "[options]\nHoldPkg = pacman\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n\n[extra]\nInclude = /etc/pacman.d/mirrorlist\n";
        assert!(repo_uses(arch, "core", ml));
        // Artix: mirrorlist — зеркала Artix, репозитории Arch идут через mirrorlist-arch
        let artix = "[system]\nInclude = /etc/pacman.d/mirrorlist\n\n[extra]\nInclude = /etc/pacman.d/mirrorlist-arch\n";
        assert!(!repo_uses(artix, "core", ml));
        // закомментированный [core] не считается
        assert!(!repo_uses("#[core]\n#Include = /etc/pacman.d/mirrorlist\n", "core", ml));
    }
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    struct EnvGuard {
        _iso: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
        cleanup: Vec<PathBuf>,
    }

    impl EnvGuard {
        fn state_dir(path: &Path) -> Self {
            let _iso = crate::common::contract_fixtures::isolation_lock();
            let mut g = Self { _iso, saved: vec![], cleanup: vec![] };
            g.set("UPD_STATE_DIR", path.to_str().unwrap());
            g.cleanup.push(path.to_path_buf());
            g
        }

        fn set(&mut self, key: &str, val: &str) {
            self.saved.push((key.to_string(), std::env::var(key).ok()));
            unsafe {
                std::env::set_var(key, val);
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(val) => unsafe {
                        std::env::set_var(k, val);
                    },
                    None => unsafe {
                        std::env::remove_var(k);
                    },
                }
            }
            for p in &self.cleanup {
                let _ = fs::remove_dir_all(p);
            }
        }
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn zypper_osr() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("PRETTY_NAME".into(), "Test".into());
        m.insert("ID".into(), "opensuse-tumbleweed".into());
        m
    }

    // --- FS-01 ---
    #[test]
    fn fs01_private_temp_dir_mode_and_cleanup() {
        let dir = PrivateTempDir::new().unwrap();
        let mode = fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let path = dir.0.clone();
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn fs01_does_not_use_predictable_tmp_path() {
        let pid = std::process::id();
        let bait = PathBuf::from(format!("/tmp/upd-mirrors.{pid}"));
        fs::write(&bait, b"attacker").unwrap();
        let dir = PrivateTempDir::new().unwrap();
        assert_ne!(dir.0, bait);
        assert_eq!(fs::read_to_string(&bait).unwrap(), "attacker");
        drop(dir);
        let _ = fs::remove_file(&bait);
    }

    // --- SEC-04A ---
    #[test]
    fn sec04a_apt_backup_is_private() {
        let base = std::env::temp_dir().join(format!("upd-apt-backup-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let _g = EnvGuard::state_dir(&base);
        let mut backup = BTreeMap::new();
        backup.insert(
            "/etc/apt/sources.list".into(),
            AptSourceBackup { content: "deb http://secret:token@mirror.example/ubuntu jammy main".into(), mode: 0o600, uid: 0, gid: 0, applied: None },
        );
        save_apt_backup(&backup).unwrap();
        let file_mode = fs::metadata(apt_backup_path()).unwrap().permissions().mode() & 0o777;
        let dir_mode = fs::metadata(apt_backup_dir()).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        let loaded = load_apt_backup().unwrap();
        assert_eq!(loaded.get("/etc/apt/sources.list").map(|b| b.content.as_str()), Some("deb http://secret:token@mirror.example/ubuntu jammy main"));
    }

    // --- UPD-05 ---
    #[test]
    fn upd05_zypper_failure_is_not_empty_list() {
        let bin = std::env::temp_dir().join(format!("upd-zypper-bin-{}", std::process::id()));
        fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("zypper"), "#!/bin/sh\nexit 9\n");
        let err = crate::common::contract_fixtures::with_prepend_path(&bin, || Rpm::zypper(&zypper_osr()).updates()).unwrap_err();
        assert!(err.contains("9"), "ожидали код ошибки в сообщении: {err}");
    }

    #[test]
    fn upd05_zypper_empty_stdout_is_ok() {
        let bin = std::env::temp_dir().join(format!("upd-zypper-empty-{}", std::process::id()));
        fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("zypper"), "#!/bin/sh\nexit 0\n");
        assert!(crate::common::contract_fixtures::with_prepend_path(&bin, || Rpm::zypper(&zypper_osr()).updates()).unwrap().is_empty());
    }

    #[test]
    fn upd05_zypper_parses_update_lines() {
        let bin = std::env::temp_dir().join(format!("upd-zypper-parse-{}", std::process::id()));
        fs::create_dir_all(&bin).unwrap();
        write_executable(
            &bin.join("zypper"),
            "#!/bin/sh\necho 'v | i | nano | 7.2-1.1 | 7.2-1.2 | repo'\nexit 0\n",
        );
        let ups = crate::common::contract_fixtures::with_prepend_path(&bin, || Rpm::zypper(&zypper_osr()).updates()).unwrap();
        assert_eq!(ups, vec!["nano 7.2-1.1 -> 7.2-1.2"]);
    }

    // --- SEC-04B ---
    #[test]
    fn sec04b_apt_original_uri_file_is_private() {
        let base = std::env::temp_dir().join(format!("upd-apt-uri-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let _g = EnvGuard::state_dir(&base);
        let uri = "http://secret:token@mirror.example/debian";
        save_apt_backup(&BTreeMap::new()).unwrap();
        atomic_write(&apt_original_uri_path(), uri.as_bytes(), 0o600).unwrap();
        let mode = fs::metadata(apt_original_uri_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(load_apt_original_uri().unwrap(), uri);
    }

    // --- UPD-07 ---
    #[test]
    fn upd07_pacman_missing_exit_code_is_error() {
        let base = std::env::temp_dir().join(format!("upd-pacman-miss-{}", std::process::id()));
        let bin = base.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(base.join("syncdb").join("sync")).unwrap();
        write_executable(&bin.join("pacman"), "#!/bin/sh\nexit 127\n");
        let osr = BTreeMap::from([("PRETTY_NAME".to_string(), "Test".to_string())]);
        let _g = EnvGuard::state_dir(&base);
        let _path = crate::common::contract_fixtures::prepend_path(&bin);
        let p = Pacman::new(&osr);
        let err = p.updates().unwrap_err();
        assert!(err.contains("127"), "{err}");
    }

    // --- SEC-04C ---
    #[test]
    fn sec04c_atomic_write_preserves_private_mode() {
        let base = std::env::temp_dir().join(format!("upd-apt-mode-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let path = base.join("sources.list");
        fs::write(&path, "deb http://example/debian stable main\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let uid = fs::metadata(&path).unwrap().uid();
        let gid = fs::metadata(&path).unwrap().gid();
        atomic_write_apt_source(&path, b"deb http://mirror/debian stable main\n", 0o600, uid, gid).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&base);
    }

    // --- APT-01 ---
    #[test]
    fn apt01_merge_keeps_admin_line_after_mirror_apply() {
        let uri = "http://mirror.example/debian";
        let reference = format!("mirror+file:{APT_LIST}");
        let original = AptSourceBackup {
            content: format!("deb {uri} stable main\n"),
            mode: 0o644,
            uid: 0,
            gid: 0,
            applied: None,
        };
        let applied_content = original.content.replace(uri, &reference);
        let current = AptSourceBackup {
            content: format!("{applied_content}deb http://admin-added/local extra\n"),
            mode: 0o644,
            uid: 0,
            gid: 0,
            applied: None,
        };
        let merged = merge_apt_source_after_apply("/etc/apt/sources.list", &original, current, uri).unwrap();
        assert!(merged.content.contains("admin-added"));
        assert!(!merged.content.contains(&reference));
    }

    // --- APT-02 ---
    #[test]
    fn apt02_merge_rejects_orphan_mirror_list_reference() {
        let reference = format!("mirror+file:{APT_LIST}");
        let original = AptSourceBackup {
            content: "deb http://mirror.example/debian stable main\n".into(),
            mode: 0o644,
            uid: 0,
            gid: 0,
            applied: None,
        };
        let current = AptSourceBackup {
            content: format!("deb {reference} stable main\n"),
            mode: 0o644,
            uid: 0,
            gid: 0,
            applied: None,
        };
        assert!(merge_apt_source_after_apply("/etc/apt/sources.list", &original, current, "").is_err());
    }
}
