#[derive(Clone, Copy, Debug)]
struct Peer {
    pid: i32,
    uid: u32,
}

impl Peer {
    fn user_context(self) -> Result<Option<UserContext>, String> {
        if self.uid == 0 { Ok(None) } else { UserContext::from_uid(self.uid).map(Some) }
    }
}

fn peer_of(s: &UnixStream) -> Option<Peer> {
    // SAFETY: plain C struct; the all-zero bit pattern is a valid value.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len describe a live ucred-sized buffer.
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0 && cred.pid > 0).then_some(Peer { pid: cred.pid, uid: cred.uid })
}

/// Время старта процесса (поле 22 /proc/PID/stat): вместе с pid однозначно называет процесс для polkit.
fn start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

pub fn user_name(uid: u32) -> Option<String> {
    // SAFETY: plain C struct; the all-zero bit pattern is a valid value.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: passwd, buffer and result are valid for the call; buffer outlives every pointer stored in passwd.
    let r = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    if r != 0 || res.is_null() {
        return None;
    }
    // SAFETY: getpw*_r succeeded, so pw_name points to a NUL-terminated string inside the still-live buffer.
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

enum Auth {
    Yes,
    No(String),
}

/// Проверка через polkit. root разрешено всё; в тестовом режиме (CM_STATE_DIR) можно разрешить явно.
fn authorize(p: Peer, action: &str) -> Auth {
    if p.uid == 0 || (test_mode() && std::env::var("CM_HELPER_ALLOW").as_deref() == Ok("1")) {
        return Auth::Yes;
    }
    let Some(start) = start_time(p.pid) else { return Auth::No(t!("процесс запроса уже завершился").into()) };
    match crate::controller::actions::pkcheck(p.pid, start, p.uid, action) {
        crate::controller::actions::PkResult::Allowed => Auth::Yes,
        crate::controller::actions::PkResult::Dismissed => Auth::No(t!("окно подтверждения закрыто").into()),
        crate::controller::actions::PkResult::NoAgent => Auth::No(t!("нет агента polkit для ввода пароля").into()),
        crate::controller::actions::PkResult::Denied => Auth::No(t!("нет прав (polkit: {0})", action)),
        crate::controller::actions::PkResult::Failed => Auth::No(t!("не удалось проверить права: {0}", "pkcheck")),
    }
}

// ======================= операция =======================

