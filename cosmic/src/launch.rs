//! One bounded registry and one reaper for externally launched applications.
use std::collections::VecDeque;
use std::io;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
const MAX_CHILDREN: usize = 256;
const MAX_ERRORS: usize = 32;
#[derive(Default)]
struct Registry {
    children: Vec<(String, Child)>,
    errors: VecDeque<String>,
    closing: bool,
}
impl Registry {
    fn error(&mut self, error: String) {
        if self.errors.len() == MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(error);
    }
}
pub struct Reaper {
    state: Arc<(Mutex<Registry>, Condvar)>,
}
impl Reaper {
    pub fn new() -> io::Result<Self> {
        let state = Arc::new((Mutex::new(Registry::default()), Condvar::new()));
        let worker = state.clone();
        std::thread::Builder::new()
            .name("upd-launch-reaper".into())
            .spawn(move || {
                let (mutex, wake) = &*worker;
                let mut registry = mutex.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    let mut n = 0;
                    while n < registry.children.len() {
                        match registry.children[n].1.try_wait() {
                            Ok(Some(status)) => {
                                let (name, _) = registry.children.swap_remove(n);
                                if !status.success() {
                                    registry.error(format!("{name}: process exited with {status}"));
                                }
                            }
                            Ok(None) => n += 1,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => n += 1,
                            Err(e) => {
                                let (name, _) = registry.children.swap_remove(n);
                                registry.error(format!("{name}: reaping failed: {e}"));
                            }
                        }
                    }
                    if registry.closing && registry.children.is_empty() {
                        return;
                    }
                    registry = if registry.children.is_empty() {
                        wake.wait(registry).unwrap_or_else(|e| e.into_inner())
                    } else {
                        wake.wait_timeout(registry, Duration::from_millis(20))
                            .unwrap_or_else(|e| e.into_inner())
                            .0
                    };
                }
            })?;
        Ok(Self { state })
    }
    pub fn spawn(&self, command: &mut Command) -> io::Result<u32> {
        let name = command.get_program().to_string_lossy().into_owned();
        let (mutex, wake) = &*self.state;
        let mut registry = mutex.lock().unwrap_or_else(|e| e.into_inner());
        if registry.children.len() >= MAX_CHILDREN {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "too many external applications",
            ));
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let pid = child.id();
        registry.children.push((name, child));
        wake.notify_one();
        Ok(pid)
    }
    pub fn errors(&self) -> Vec<String> {
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .errors
            .drain(..)
            .collect()
    }
    pub fn record_error(&self, error: String) {
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error(error);
    }
}
impl Drop for Reaper {
    fn drop(&mut self) {
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closing = true;
        self.state.1.notify_one();
    }
}
fn reaper() -> io::Result<&'static Reaper> {
    static REAPER: OnceLock<Result<Reaper, String>> = OnceLock::new();
    REAPER
        .get_or_init(|| Reaper::new().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| io::Error::other(e.clone()))
}
pub fn spawn(command: &mut Command) -> io::Result<u32> {
    reaper()?.spawn(command)
}
pub fn errors() -> Vec<String> {
    reaper().map(Reaper::errors).unwrap_or_default()
}
pub fn record_error(error: String) {
    if let Ok(reaper) = reaper() {
        reaper.record_error(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    #[test]
    fn hundred_children_are_reaped_without_a_thread_per_child() {
        let reaper = Reaper::new().unwrap();
        let mut pids = vec![];
        for _ in 0..100 {
            pids.push(
                reaper
                    .spawn(Command::new("/bin/sh").args(["-c", "exit 0"]))
                    .unwrap(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !reaper.state.0.lock().unwrap().children.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        for pid in pids {
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        }
        assert!(reaper.errors().is_empty());
    }
    #[test]
    fn long_child_does_not_block_launch_and_missing_launcher_is_error() {
        let reaper = Reaper::new().unwrap();
        let start = Instant::now();
        let pid = reaper
            .spawn(Command::new("/bin/sh").args(["-c", "exec /bin/sleep 30"]))
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(
            reaper
                .spawn(&mut Command::new("/nonexistent/upd-fixture"))
                .is_err()
        );
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !reaper.state.0.lock().unwrap().children.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!reaper.errors().is_empty());
    }
}
