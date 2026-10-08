fn profile_path(sub: &Sub, kind: &str) -> String {
    format!("{}/profiles/{}.{}", home(), sub.id, if kind == "uri" { "txt" } else { "yaml" })
}

struct ProfileChange {
    target: PathBuf,
    previous_target: Option<PathBuf>,
    old_extension: Option<PathBuf>,
}

fn sync_profile_dir(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| t!("{}: нет каталога профилей", path.display()))?;
    fs::File::open(parent).and_then(|dir| dir.sync_all()).map_err(|e| format!("{}: {e}", parent.display()))
}

fn remove_profile_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => sync_profile_dir(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn stage_profile(path: &Path, body: &str, kind: &str, nodes: usize) -> Result<PathBuf, String> {
    let dir = path.parent().ok_or_else(|| t!("{}: нет каталога профилей", path.display()))?;
    for _ in 0..8 {
        let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".cm-profile-{}-{n}.tmp", std::process::id()));
        let mut file = match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let result = (|| -> Result<(), String> {
            use std::io::Write;
            file.write_all(body.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
            file.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
            drop(file);
            let staged = fs::read_to_string(&temp).map_err(|e| format!("{}: {e}", path.display()))?;
            let (staged_kind, staged_nodes) = classify_sub(&staged)?;
            if staged_kind != kind || staged_nodes != nodes {
                return Err(t!("временный профиль отличается от проверенного ответа подписки").into());
            }
            sync_profile_dir(path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(temp);
    }
    Err(t!("{}: не удалось создать временный профиль", path.display()))
}

fn install_profile(temp: &Path, target: &Path, old_extension: Option<PathBuf>) -> Result<ProfileChange, String> {
    let previous_target = match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let parent = target.parent().ok_or_else(|| t!("{}: нет каталога профилей", target.display()))?;
            let mut backup = None;
            // Hard link keeps the old bytes available for rollback without another data write.
            for _ in 0..8 {
                let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                let candidate = parent.join(format!(".cm-profile-backup-{}-{n}.tmp", std::process::id()));
                match fs::hard_link(target, &candidate) {
                    Ok(()) => {
                        if let Err(e) = sync_profile_dir(target) {
                            let _ = fs::remove_file(&candidate);
                            return Err(e);
                        }
                        backup = Some(candidate);
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(t!("{}: не удалось сохранить прежний профиль: {1}", target.display(), e)),
                }
            }
            Some(backup.ok_or_else(|| t!("{}: не удалось создать резервную ссылку профиля", target.display()))?)
        }
        Ok(_) => return Err(t!("{}: ожидался обычный файл профиля", target.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", target.display())),
    };
    if let Err(e) = fs::rename(temp, target) {
        if let Some(backup) = &previous_target {
            let _ = remove_profile_file(backup);
        }
        let _ = remove_profile_file(temp);
        return Err(format!("{}: {e}", target.display()));
    }
    if let Err(e) = sync_profile_dir(target) {
        let rollback = match &previous_target {
            Some(backup) => fs::rename(backup, target).map_err(|rollback| t!("{}: {2}; прежний профиль сохранён в {}", target.display(), backup.display(), rollback)),
            None => remove_profile_file(target),
        };
        return match rollback {
            Ok(()) => Err(e),
            Err(rollback) => Err(t!("{0}; не удалось вернуть прежний профиль: {1}", e, rollback)),
        };
    }
    Ok(ProfileChange { target: target.to_path_buf(), previous_target, old_extension })
}

fn rollback_profile_change(change: &ProfileChange) -> Result<(), String> {
    match &change.previous_target {
        Some(backup) => {
            fs::rename(backup, &change.target).map_err(|e| t!("{}: {2}; прежний профиль сохранён в {}", change.target.display(), backup.display(), e))?;
            sync_profile_dir(&change.target)
        }
        None => remove_profile_file(&change.target),
    }
}

fn finish_profile_change(change: ProfileChange, log: Log) {
    if let Some(old) = &change.old_extension
        && old != &change.target
        && let Err(e) = remove_profile_file(old) {
        log(&t!("VPN: старый профиль оставлен для очистки: {0}", e));
    }
    if let Some(backup) = &change.previous_target
        && let Err(e) = remove_profile_file(backup) {
        log(&t!("VPN: резервный профиль оставлен для очистки: {0}", e));
    }
}

fn classify_sub(body: &str) -> Result<(String, usize), String> {
    if let Ok(Value::Mapping(m)) = parse_profile(body) {
        check_subscription_placeholder(&m)?;
        let proxies = m.get("proxies").and_then(Value::as_sequence).map(|s| s.len()).unwrap_or(0);
        if proxies > 0 || m.contains_key("proxy-providers") {
            return Ok(("clash".into(), proxies));
        }
    }
    let count = |t: &str| t.lines().filter(|line| {
        line.trim().split_once("://").is_some_and(|(scheme, address)| {
            !address.is_empty() && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
    }).count();
    let n = count(body);
    if n > 0 {
        return Ok(("uri".into(), n));
    }
    if let Some(dec) = b64_decode(body.trim()) {
        let n = count(&String::from_utf8_lossy(&dec));
        if n > 0 {
            return Ok(("uri".into(), n));
        }
    }
    Err(t!("ответ не похож на подписку (нет ни конфига Clash, ни ссылок vless://, ss://…)").into())
}

pub fn add_sub(url: &str, name: &str, c: &Config, log: Log) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    subscription_url(url)?;
    let mut subs = load_subs()?;
    if subs.list.iter().any(|s| s.url == url) {
        return Err(t!("такая подписка уже есть").into());
    }
    let id = format!("{:x}", now()) + &sha1_smol::Sha1::from(url).digest().to_string()[..6];
    let mut sub = Sub { id: id.clone(), name: sanitize_profile_name(name), url: url.to_string(), interval_h: c.vpn_sub_update_h, ..Default::default() };
    log(&t!("скачиваю подписку {}...", mask_url(url)));
    let change = fetch_sub(&mut sub, c)?;
    log(&t!("«{}»: серверов {}, формат {}", sub.name, sub.nodes, sub.kind));
    subs.list.push(sub);
    if subs.active.is_empty() || !subs.list.iter().any(|s| s.id == subs.active) {
        subs.active = id;
    }
    if let Err(e) = save_subs(&subs) {
        return match rollback_profile_change(&change) {
            Ok(()) => Err(e),
            Err(rollback) => Err(t!("{0}; профиль не удалось вернуть: {1}", e, rollback)),
        };
    }
    finish_profile_change(change, log);
    Ok(())
}

/// Какая подписка: номер из `cm vpn subs` (для разовой команды) или неизменный id (из TUI).
/// Номер и id разрешаются в запись только под блокировкой подписок.
#[derive(Clone, Debug, PartialEq)]
pub enum SubRef {
    Index(usize),
    Id(String),
}

impl SubRef {
    /// «3» — третья по списку; «id:…» — запись с этим id.
    pub fn parse(s: &str) -> Option<SubRef> {
        match s.strip_prefix("id:") {
            Some(id) if !id.is_empty() => Some(SubRef::Id(id.to_string())),
            Some(_) => None,
            None => s.parse::<usize>().ok().filter(|n| *n > 0).map(|n| SubRef::Index(n - 1)),
        }
    }

    fn resolve(&self, subs: &Subs) -> Result<usize, String> {
        match self {
            SubRef::Index(i) if *i < subs.list.len() => Ok(*i),
            SubRef::Id(id) => subs.list.iter().position(|s| &s.id == id).ok_or_else(|| t!("подписки уже нет — список изменился").into()),
            _ => Err(t!("нет такой подписки").into()),
        }
    }
}

fn validate_user_agent(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 512 || !value.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(t!("неверный User-Agent: нужна строка ASCII до 512 байт без управляющих символов").into());
    }
    Ok(())
}

pub fn set_user_agent(reference: &SubRef, user_agent: &str) -> Result<(), String> {
    validate_user_agent(user_agent)?;
    let _vpn_files = vpn_config_lock(true)?;
    let _subs_lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let index = reference.resolve(&subs)?;
    subs.list[index].user_agent = user_agent.to_owned();
    save_subs(&subs)
}

pub fn delete_sub(r: &SubRef) -> Result<String, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let idx = r.resolve(&subs)?;
    let s = subs.list.remove(idx);
    if subs.active == s.id {
        subs.active = subs.list.first().map(|x| x.id.clone()).unwrap_or_default();
    }
    save_subs(&subs)?;
    for ext in ["yaml", "txt"] {
        let _ = remove_profile_file(Path::new(&format!("{}/profiles/{}.{ext}", home(), s.id)));
    }
    Ok(s.name)
}

/// Выбор фиксируется только после проверки и успешного применения конфига.
pub fn use_sub(r: &SubRef, c: &Config) -> Result<String, String> {
    let subs = load_subs()?;
    let selected = &subs.list[r.resolve(&subs)?];
    let invalid = fs::read_to_string(profile_path(selected, &selected.kind))
        .map(|body| classify_sub(&body).is_err()).unwrap_or(true);
    let reference = SubRef::Id(selected.id.clone());
    if invalid { repair_profile(&reference, c)?; }
    switch_subscription(&reference, c, service_active(), &mut || reload())
}

fn repair_profile(reference: &SubRef, c: &Config) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let index = reference.resolve(&subs)?;
    let change = fetch_sub(&mut subs.list[index], c)?;
    if let Err(error) = save_subs(&subs) {
        rollback_profile_change(&change)?;
        return Err(error);
    }
    finish_profile_change(change, &|_| {});
    Ok(())
}

fn switch_subscription(r: &SubRef, c: &Config, active: bool, reload_core: &mut impl FnMut() -> Result<(), String>) -> Result<String, String> {
    switch_subscription_with(r, c, active, reload_core, &mut save_subs)
}

fn switch_subscription_with(r: &SubRef, c: &Config, active: bool, reload_core: &mut impl FnMut() -> Result<(), String>, commit: &mut impl FnMut(&Subs) -> Result<(), String>) -> Result<String, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let previous_subs = subs.clone();
    let selected = subs.list[r.resolve(&subs)?].clone();
    let mut stage = "build";
    let result = (|| {
        subs.active = selected.id.clone();
        subs.store_source.clear();
        let candidate = build_config_for(&latest, &subs)?;
        stage = "validate";
        validate_candidate(&candidate)?;
        let previous = match fs::read(config_path()) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !active => None,
            Err(e) => return Err(format!("{}: {e}", config_path())),
        };
        stage = "write";
        let applied = write_private(&config_path(), candidate.as_bytes()).and_then(|_| {
            stage = "apply";
            if active { reload_core() } else { Ok(()) }
        });
        let saving = applied.is_ok();
        if saving { stage = "save"; }
        let committed = applied.and_then(|_| commit(&subs));
        if let Err(error) = committed {
            let mut rollback_errors = vec![];
            let restored = match previous {
                Some(bytes) => write_private(&config_path(), &bytes),
                None => match fs::remove_file(config_path()) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.to_string()),
                },
            };
            match restored {
                Ok(()) if active => if let Err(error) = reload_core() { rollback_errors.push(error); },
                Ok(()) => {},
                Err(error) => rollback_errors.push(error),
            }
            if saving {
                // Atomic rename can succeed before directory fsync fails.
                if let Err(error) = commit(&previous_subs) { rollback_errors.push(error); }
            }
            return Err(if rollback_errors.is_empty() {
                format!("{error}; previous VPN configuration restored")
            } else { format!("{error}; VPN rollback failed: {}", rollback_errors.join("; ")) });
        }
        resolve_failure_locked();
        Ok(selected.name.clone())
    })();
    if let Err(error) = &result { store_failure_locked(stage, Some(&selected), error); }
    result
}

struct CandidateConfig(PathBuf);
impl Drop for CandidateConfig {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}
fn validate_candidate(config: &str) -> Result<(), String> {
    crate::core::legacy_host::dispatch_validate(config)
}

pub(crate) fn validate_candidate_body(config: &str) -> Result<(), String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    private_dir(&home())?;
    let path = Path::new(&home()).join(format!(".candidate-{}-{}.yaml", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let candidate = CandidateConfig(path);
    write_private(candidate.0.to_str().ok_or("invalid VPN path")?, config.as_bytes())?;
    let mut policy = CapturePolicy::background(Some(64 << 10));
    policy.stderr_max = 64 << 10;
    let output = capture_with_policy(std::process::Command::new(core_bin()).args(["-t", "-d", &home(), "-f", candidate.0.to_str().ok_or("invalid VPN path")?]).env("LC_ALL", "C"), policy).map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut reason = core_validation_reason(&stderr, &stdout).to_owned();
        if let Ok(profile) = parse_profile(config) { redact_credentials(&profile, &mut reason); }
        scrub_stored_error(&mut reason);
        return Err(t!("mihomo не принял конфиг: {}", reason));
    }
    Ok(())
}

fn core_validation_reason<'a>(stderr: &'a str, stdout: &'a str) -> &'a str {
    let meaningful = |line: &&str| {
        let lower = line.to_ascii_lowercase();
        !line.trim().is_empty() && !(lower.contains("configuration file") && lower.contains("test failed"))
    };
    let error = |line: &&str| meaningful(line) && {
        let lower = line.to_ascii_lowercase();
        lower.contains("error") || lower.contains("fatal")
    };
    stderr.lines().rev().find(error).or_else(|| stdout.lines().rev().find(error))
        .or_else(|| stderr.lines().rev().find(meaningful)).or_else(|| stdout.lines().rev().find(meaningful))
        .map(str::trim).unwrap_or("validation failed")
}

#[derive(Debug, Serialize)]
pub struct ProfileCheck {
    pub name: String,
    pub active: bool,
    pub error: Option<String>,
}

/// Проверка всех сохранённых профилей без изменения выбора, конфига и работающего ядра.
pub fn doctor(c: &Config, reference: Option<&SubRef>) -> Result<Vec<ProfileCheck>, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _subs_lock = subscriptions_lock(true)?;
    let subs = load_subs()?;
    let selected = reference.map(|r| r.resolve(&subs)).transpose()?;
    let mut checks = vec![];
    for (i, sub) in subs.list.iter().enumerate() {
        if selected.is_some_and(|n| n != i) { continue; }
        let mut candidate = subs.clone();
        candidate.active = sub.id.clone();
        let result = build_config_for(c, &candidate).and_then(|config| validate_candidate(&config));
        checks.push(ProfileCheck { name: sanitize_profile_name(&sub.name), active: subs.active == sub.id, error: result.err().map(|e| safe_diagnostic(&e, Some(sub))) });
    }
    Ok(checks)
}

/// Обновить подписки: все (force) или те, у которых подошёл срок. true — активная изменилась.
pub fn update_subs(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    update_subs_locked(c, log, force)
}

fn update_subs_locked(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let mut subs = load_subs()?;
    let mut active_changed = false;
    let mut profile_changes = Vec::new();
    for s in subs.list.iter_mut() {
        let interval = if s.interval_h > 0 { s.interval_h } else { c.vpn_sub_update_h };
        if !force && !elapsed_at_least(s.updated, hours_secs(interval)) {
            continue;
        }
        let previous = s.clone();
        match fetch_sub(s, c) {
            Ok(change) => {
                profile_changes.push(change);
                log(&t!("подписка «{}»: обновлена, серверов {}", s.name, s.nodes));
                active_changed |= s.id == subs.active;
            }
            Err(e) => {
                *s = previous;
                s.error = e.clone();
                log(&t!("подписка «{}»: {1}", s.name, e));
            }
        }
    }
    if let Err(e) = save_subs(&subs) {
        let mut rollback_errors = Vec::new();
        for change in profile_changes.iter().rev() {
            if let Err(error) = rollback_profile_change(change) {
                rollback_errors.push(error);
            }
        }
        if rollback_errors.is_empty() {
            return Err(e);
        }
        return Err(t!("{1}; не удалось вернуть прежние профили: {}", rollback_errors.join("; "), e));
    }
    for change in profile_changes {
        finish_profile_change(change, log);
    }
    Ok(active_changed)
}

// ======================= публичное состояние (без секретов) =======================

