//! Action notifications may wait for a user; keep their workers and children bounded.
use std::io;
use std::process::Command;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const MAX_PENDING: usize = 4;
#[derive(Default)]
struct Workers(Arc<AtomicUsize>);
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Workers {
    fn spawn(
        &self,
        work: impl FnOnce() + Send + 'static,
    ) -> io::Result<std::thread::JoinHandle<()>> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_PENDING).then_some(count + 1)
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "too many pending notification actions",
                )
            })?;
        let permit = Permit(self.0.clone());
        std::thread::Builder::new()
            .name("upd-notification".into())
            .spawn(move || {
                let _permit = permit;
                work();
            })
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    News,
    Log,
}
fn capture(command: &mut Command, timeout: Duration) -> Result<Option<Action>, String> {
    let mut policy = upd::common::CapturePolicy::background(Some(256));
    policy.stderr_max = 8192;
    policy.deadline = Instant::now() + timeout;
    let output = upd::common::capture_with_policy(command, policy).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("notify-send exited with {}", output.status));
    }
    Ok(match String::from_utf8_lossy(&output.stdout).trim() {
        "install" => Some(Action::Install),
        "news" => Some(Action::News),
        "log" => Some(Action::Log),
        _ => None,
    })
}
pub fn send(mut command: Command, action: impl FnOnce(Action) + Send + 'static) -> io::Result<()> {
    static WORKERS: OnceLock<Workers> = OnceLock::new();
    WORKERS
        .get_or_init(Workers::default)
        .spawn(
            move || match capture(&mut command, Duration::from_secs(300)) {
                Ok(Some(choice)) => action(choice),
                Ok(None) => {}
                Err(error) => crate::launch::record_error(format!("notification: {error}")),
            },
        )
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_workers_have_a_quota_and_release_it_after_completion() {
        let workers = Workers::default();
        let mut release = Vec::new();
        let mut handles = Vec::new();
        for _ in 0..MAX_PENDING {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            release.push(tx);
            handles.push(
                workers
                    .spawn(move || {
                        let _ = rx.recv_timeout(Duration::from_secs(2));
                    })
                    .unwrap(),
            );
        }
        assert_eq!(
            workers.spawn(|| {}).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        for tx in release {
            tx.send(()).unwrap();
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(workers.0.load(Ordering::Acquire), 0);
        workers.spawn(|| {}).unwrap().join().unwrap();
    }
    #[test]
    fn stalled_or_noisy_notification_is_bounded_and_action_is_whitelisted() {
        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "exec /bin/sleep 30"]);
        let start = Instant::now();
        assert!(
            capture(&mut slow, Duration::from_millis(80))
                .unwrap_err()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut noisy = Command::new("/bin/sh");
        noisy.args(["-c", "head -c 1024 /dev/zero"]);
        assert!(capture(&mut noisy, Duration::from_secs(1)).is_err());
        let mut known = Command::new("/bin/sh");
        known.args(["-c", "printf install"]);
        assert_eq!(
            capture(&mut known, Duration::from_secs(1)).unwrap(),
            Some(Action::Install)
        );
        let mut unknown = Command::new("/bin/sh");
        unknown.args(["-c", "printf arbitrary-command"]);
        assert_eq!(capture(&mut unknown, Duration::from_secs(1)).unwrap(), None);
    }
}
