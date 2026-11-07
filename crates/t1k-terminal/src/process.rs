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

/// The command line a process was started with ("npm run dev"), from KERN_PROCARGS2.
#[cfg(target_os = "macos")]
pub fn command(pid: i32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) } != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    buf.truncate(size);
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    // After argc: the executable path, NUL padding, then argc NUL-terminated arguments.
    let rest = &buf[4..];
    let exe_end = rest.iter().position(|&b| b == 0)?;
    let mut i = exe_end;
    while rest.get(i) == Some(&0) {
        i += 1;
    }
    let args: Vec<String> = rest[i..].split(|&b| b == 0).take(argc).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    if args.is_empty() {
        return None;
    }
    // Show the program by its short name: "/usr/local/bin/node /x/npm-cli.js run dev" stays,
    // but a leading path is trimmed.
    let mut words = args;
    if let Some(first) = words.first_mut() {
        *first = first.rsplit('/').next().unwrap_or(first).trim_start_matches('-').to_string();
    }
    Some(words.join(" "))
}

#[cfg(not(target_os = "macos"))]
pub fn command(pid: i32) -> Option<String> {
    let s = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(s.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>().join(" "))
}

/// When a process started.
#[cfg(target_os = "macos")]
pub fn started(pid: i32) -> Option<std::time::SystemTime> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
    (n == size).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(info.pbi_start_tvsec))
}

#[cfg(not(target_os = "macos"))]
pub fn started(_pid: i32) -> Option<std::time::SystemTime> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_process() {
        let pid = std::process::id() as i32;
        assert_eq!(super::cwd(pid), std::env::current_dir().ok());
        assert!(super::name(pid).is_some());
        assert!(super::command(pid).is_some());
        assert!(super::started(pid).is_some());
    }
}
