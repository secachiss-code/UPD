//! Общие операции процесса ядра: приватный файл, ожидание порта, остановка группы.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::TcpStream;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use super::adapter::CoreError;
use super::instance::InstanceOwner;

/// Приватный файл, который читает ядро под другим uid. Каталог обязан принадлежать
/// менеджеру: иначе владелец подменил бы запись до открытия.
pub fn write_for(path: &Path, bytes: &[u8], owner: Option<InstanceOwner>) -> Result<(), CoreError> {
    use std::os::fd::AsRawFd;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| CoreError::Failed)?;
    if let Some(owner) = owner {
        // SAFETY: fchown on a descriptor this function owns; no pointer is passed.
        if unsafe { libc::fchown(file.as_raw_fd(), owner.uid, owner.gid) } != 0 {
            return Err(CoreError::Failed);
        }
    }
    file.write_all(bytes).map_err(|_| CoreError::Failed)?;
    file.sync_all().map_err(|_| CoreError::Failed)?;
    Ok(())
}

pub fn remove_file(path: &Path) -> Result<(), CoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CoreError::Failed),
    }
}

pub fn signal_group(pgid: i32, signal: i32) -> bool {
    // SAFETY: a negative pid signals the process group. Signal 0 only probes.
    unsafe { libc::kill(-pgid, signal) == 0 }
}

/// SIGTERM, до двух секунд, затем SIGKILL. Завершившийся лидер остаётся зомби и отвечает
/// на сигнал 0, пока его не забрали, поэтому ожидание прекращается по `try_wait`.
pub fn terminate_child(child: &mut Child) {
    let pgid = child.id() as i32;
    terminate_group_until(pgid, &mut || {
        matches!(child.try_wait(), Ok(Some(_)) | Err(_))
    });
}

fn terminate_group_until(pgid: i32, leader_gone: &mut dyn FnMut() -> bool) {
    signal_group(pgid, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && signal_group(pgid, 0) {
        if leader_gone() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if signal_group(pgid, 0) {
        signal_group(pgid, libc::SIGKILL);
    }
}

pub fn wait_port(port: u16, timeout: Duration) -> Result<(), CoreError> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + timeout;
    loop {
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(CoreError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
