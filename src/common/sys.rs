//! Safe wrappers for libc calls that take no pointers and cannot fail.

/// Effective user ID of this process.
pub fn euid() -> u32 {
    // SAFETY: geteuid takes no arguments, touches no memory and always succeeds.
    unsafe { libc::geteuid() }
}

/// Real user ID of this process.
pub fn uid() -> u32 {
    // SAFETY: getuid takes no arguments, touches no memory and always succeeds.
    unsafe { libc::getuid() }
}

/// Effective group ID of this process.
pub fn egid() -> u32 {
    // SAFETY: getegid takes no arguments, touches no memory and always succeeds.
    unsafe { libc::getegid() }
}
