//! Запуск приложения внутри netns своего туннеля и только при живом ядре.

use crate::app::spec;
use crate::common::test_mode;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::core_ops::Workers;
use super::drop::{self, run_as_for};
use super::harden::ChildLimits;
use super::net_ops::NetCtx;
use super::owner::Owned;
use super::peer::PeerIdentity;
use super::protocol::{ControlError, Reply, ReplyData};
use super::registry::Generations;

const MAX_ENV_BYTES: usize = 4096;

#[allow(clippy::too_many_arguments)]
pub fn launch(
    owned: &Owned,
    id: &str,
    generation: u64,
    program: &str,
    args: &[String],
    session_env: &std::collections::BTreeMap<String, String>,
    session: Option<&str>,
    peer: &PeerIdentity,
    workers: &Workers,
    nets: &NetCtx,
    base: &Path,
) -> Result<Reply, ControlError> {
    let Some(index) = super::net_ops::current_index(nets, owned)? else {
        return Err(ControlError::NotRunning);
    };
    let generations = Generations::load(&owned.generations_path())?;
    if generations
        .current(&format!("net:{}", owned.instance.as_str()))
        .is_none()
    {
        return Err(ControlError::NotRunning);
    }
    // Поколение запроса — поколение worker-а, которое клиент прочитал из `WorkerStatus`.
    // Ядро обязано отвечать: запись в карте остаётся и после гибели процесса.
    let status = workers.status(owned, "status");
    let Some(ReplyData::Status {
        running: true,
        generation: Some(current),
        api,
        ..
    }) = status.data
    else {
        return Err(ControlError::NotRunning);
    };
    if api != "api_ready" {
        return Err(ControlError::NotRunning);
    }
    if current != generation {
        return Err(ControlError::GenerationMismatch);
    }
    if spec::program_rejected(program, args) {
        return Err(ControlError::BadArgument);
    }
    let run_as = run_as_for(peer.uid, peer.gid)?;
    let session_file = read_session(owned, session)?;
    let mut env = env_of(owned, peer.uid, session_env, session_file.as_ref())?;
    apply_private(&mut env, peer.uid, session, session_file.as_ref())?;
    let localtime = zone_file(&env)?;
    let netns_path = netns_root(base).join(format!("cm-{index}"));
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&netns_path)
        .map_err(|_| ControlError::NotRunning)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in &env {
        command.env(key, value);
    }
    // Без файла DNS приложение не запускается: оно читало бы resolv.conf хоста.
    let resolv = nets
        .etc_root
        .join(format!("cm-{index}"))
        .join("resolv.conf");
    if !resolv.is_file() {
        return Err(ControlError::NotRunning);
    }
    if let Some(home) = env.get("HOME").filter(|home| Path::new(home).is_dir()) {
        command.current_dir(home);
    } else {
        command.current_dir("/");
    }
    drop::drop_into_netns_region_pre_exec(
        &mut command,
        file.as_raw_fd(),
        Some(&resolv),
        localtime.as_deref(),
        run_as,
        &[],
        ChildLimits::default(),
    )?;
    // Дескриптор netns нужен только до exec потомка; у контроллера он закрывается здесь.
    let child = command.spawn().map_err(|_| ControlError::Failed)?;
    drop(file);
    let pid = child.id();
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    workers.note_launch(owned.uid, owned.instance.as_str());
    Ok(Reply::ok(id, Some(ReplyData::Launched { pid })))
}

pub fn list(
    owned: &Owned,
    id: &str,
    peer: &PeerIdentity,
    nets: &NetCtx,
    base: &Path,
) -> Result<Reply, ControlError> {
    if peer.uid != owned.uid {
        return Err(ControlError::NotRunning);
    }
    let Some(index) = super::net_ops::current_index(nets, owned)? else {
        return Err(ControlError::NotRunning);
    };
    let netns = format!("cm-{index}");
    let inode = crate::app::instance::netns_of_path(&netns_root(base).join(&netns));
    let mut pids = match inode {
        Some(inode) => {
            let procs = crate::app::instance::scan(Path::new("/proc"));
            crate::app::instance::holders(&procs, inode, peer.uid)
        }
        None => Vec::new(),
    };
    pids.truncate(256);
    Ok(Reply::ok(id, Some(ReplyData::Apps { netns, pids })))
}

pub fn env_of(
    owned: &Owned,
    uid: u32,
    session_env: &std::collections::BTreeMap<String, String>,
    session: Option<&crate::app::env::SessionFile>,
) -> Result<std::collections::BTreeMap<String, String>, ControlError> {
    // Имена уже проверил декодер; регион ниже перекрывает всё, что пришло от клиента.
    let mut env = session_env.clone();
    env.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    if let Some(home) = home_dir(uid) {
        env.insert("HOME".to_owned(), home);
    }
    env.insert("XDG_RUNTIME_DIR".to_owned(), format!("/run/user/{uid}"));
    if let Some(file) = session {
        env.insert("TZ".to_owned(), file.timezone.clone());
        env.insert("LANG".to_owned(), file.locale.clone());
    } else if let Some((timezone, locale)) = read_env_preset(owned)? {
        env.insert("TZ".to_owned(), timezone);
        env.insert("LANG".to_owned(), locale);
    }
    Ok(env)
}

fn read_session(
    owned: &Owned,
    session: Option<&str>,
) -> Result<Option<crate::app::env::SessionFile>, ControlError> {
    let Some(session) = session else {
        return Ok(None);
    };
    let path = owned
        .root
        .join("instances")
        .join(owned.instance.as_str())
        .join("config")
        .join(format!("session-{session}.json"));
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ControlError::InvalidConfig);
        }
        Err(_) => return Err(ControlError::InvalidConfig),
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ControlError::InvalidConfig);
            }
            if meta.len() > crate::app::env::MAX_SESSION_BYTES as u64 {
                return Err(ControlError::TooLarge);
            }
        }
    }
    let bytes = super::owner::read_owned_file(owned, &path, crate::app::env::MAX_SESSION_BYTES)?;
    crate::app::env::parse_session(&bytes)
        .map(Some)
        .map_err(|_| ControlError::InvalidConfig)
}

fn apply_private(
    env: &mut std::collections::BTreeMap<String, String>,
    uid: u32,
    session: Option<&str>,
    file: Option<&crate::app::env::SessionFile>,
) -> Result<(), ControlError> {
    for key in ["SSH_AUTH_SOCK", "GNOME_KEYRING_CONTROL", "GPG_AGENT_INFO"] {
        env.remove(key);
    }
    let (Some(session), Some(file)) = (session, file) else {
        return Ok(());
    };
    if let Some(data) = &file.data
        && data.state != crate::env::private::DataState::Existing
    {
        let home = env.get("HOME").ok_or(ControlError::InvalidConfig)?;
        let dirs =
            crate::env::private::data_dirs(Path::new(home), &data.profile, data.state, session)
                .ok_or(ControlError::InvalidConfig)?;
        for path in [
            &dirs.root,
            &dirs.config,
            &dirs.data,
            &dirs.cache,
            &dirs.state,
        ] {
            if !crate::env::private::check_dir(path, uid) {
                return Err(ControlError::InvalidConfig);
            }
        }
        env.extend(crate::env::private::env_overrides(&dirs));
    }
    if file.ipc == crate::env::private::IpcPolicy::Private {
        let runtime = PathBuf::from(format!("/run/user/{uid}"));
        let dir = crate::env::private::runtime_dir(&runtime, session);
        if !crate::env::private::check_dir(&dir, uid) {
            return Err(ControlError::InvalidConfig);
        }
        env.insert(
            "XDG_RUNTIME_DIR".to_owned(),
            dir.to_string_lossy().into_owned(),
        );
        env.remove("DBUS_SESSION_BUS_ADDRESS");
        if let Some(display) = env.get("WAYLAND_DISPLAY").cloned()
            && !display.starts_with('/')
        {
            env.insert(
                "WAYLAND_DISPLAY".to_owned(),
                format!("{}/{display}", runtime.display()),
            );
        }
        if !env.contains_key("PULSE_SERVER") {
            env.insert(
                "PULSE_SERVER".to_owned(),
                format!("unix:{}/pulse/native", runtime.display()),
            );
        }
    }
    Ok(())
}

fn zone_file(
    env: &std::collections::BTreeMap<String, String>,
) -> Result<Option<PathBuf>, ControlError> {
    let Some(zone) = env.get("TZ") else {
        return Ok(None);
    };
    if zone.contains('\0') {
        return Err(ControlError::BadArgument);
    }
    let path = Path::new("/usr/share/zoneinfo").join(zone);
    if !path.is_file() {
        return Err(ControlError::InvalidConfig);
    }
    Ok(Some(path))
}

fn netns_root(base: &Path) -> PathBuf {
    if test_mode() {
        base.join("netns")
    } else {
        PathBuf::from("/run/netns")
    }
}

fn read_env_preset(owned: &Owned) -> Result<Option<(String, String)>, ControlError> {
    let path = owned
        .root
        .join("instances")
        .join(owned.instance.as_str())
        .join("config")
        .join("env.json");
    if std::fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    let bytes = super::owner::read_owned_file(owned, &path, MAX_ENV_BYTES)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| ControlError::InvalidConfig)?;
    let timezone = value
        .get("timezone")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_owned();
    let locale = value
        .get("locale")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_owned();
    if !crate::identity::tzdata::valid_zone_name(&timezone)
        || locale.is_empty()
        || locale.len() > 64
        || !locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'@'))
    {
        return Err(ControlError::InvalidConfig);
    }
    Ok(Some((timezone, locale)))
}

fn home_dir(uid: u32) -> Option<String> {
    // SAFETY: passwd is a plain C struct; the all-zero bit pattern is valid.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16 * 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: passwd and the buffer outlive the call; on success pw_dir points into `buf`.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_dir is a NUL-terminated string inside `buf`.
    let dir = unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) };
    Some(dir.to_string_lossy().into_owned())
}
