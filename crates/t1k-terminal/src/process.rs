//! Inspect the processes running in a pty (macOS): foreground process, its cwd and name.

use std::ffi::CStr;
use std::os::fd::RawFd;
use std::path::PathBuf;

/// Process group in the foreground of the pty (the program the user is interacting with).
pub fn foreground_pid(pty_fd: RawFd) -> Option<i32> {
    let pid = unsafe { libc::tcgetpgrp(pty_fd) };
    (pid > 0).then_some(pid)
}

/// Current working directory of a process.
#[cfg(target_os = "macos")]
pub fn cwd(pid: i32) -> Option<PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let n = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut info as *mut _ as *mut libc::c_void, size)
    };
    if n != size {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr() as *const libc::c_char) };
    let s = path.to_str().ok()?;
    (!s.is_empty()).then(|| PathBuf::from(s))
}

#[cfg(not(target_os = "macos"))]
pub fn cwd(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// Short process name (e.g. "zsh", "claude", "nvim").
#[cfg(target_os = "macos")]
pub fn name(pid: i32) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(not(target_os = "macos"))]
pub fn name(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_process() {
        let pid = std::process::id() as i32;
        assert_eq!(super::cwd(pid), std::env::current_dir().ok());
        assert!(super::name(pid).is_some());
    }
}
