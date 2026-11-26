//! The app's side of the session daemon (`daemon.rs`): starting it, asking it things, and a
//! pty for alacritty's event loop that is really the daemon's socket for one tab.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};

// alacritty's event loop tells its two pty sources apart by these keys (crate-private there).
const PTY_READ_WRITE_TOKEN: usize = 0;
const PTY_CHILD_EVENT_TOKEN: usize = 1;
use polling::{Event, PollMode, Poller};

use crate::daemon::{recv_with_fd, Reply, Request, Size, Spawn, PROTOCOL};

static DETACHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// This app is going away but its tabs aren't (Restart T1K, an update): from now on no
/// terminal ends its shell when it goes.
pub fn detach_all() {
    DETACHING.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn detaching() -> bool {
    DETACHING.load(std::sync::atomic::Ordering::SeqCst)
}

/// How to reach the daemon for one support folder.
#[derive(Clone, Debug)]
pub struct Daemon {
    sock: PathBuf,
}

impl Daemon {
    /// The daemon for `dir`, started with `exe daemon <dir>` if none answers.
    pub fn connect(dir: &Path, exe: &Path) -> io::Result<Daemon> {
        let d = Daemon { sock: crate::daemon::socket_path(dir) };
        match d.request(&Request::Hello) {
            Ok(r) if r.protocol == PROTOCOL => return Ok(d),
            Ok(r) => return Err(io::Error::other(format!("session daemon speaks protocol {}, not {PROTOCOL}", r.protocol))),
            Err(_) => {}
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("daemon").arg(dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        // Its log, next to the app's.
        let log = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/T1K"));
        match log.and_then(|l| std::fs::create_dir_all(&l).ok().and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(l.join("daemon.log")).ok())) {
            Some(f) => cmd.stderr(f),
            None => cmd.stderr(std::process::Stdio::null()),
        };
        // Its own session, so whatever ends the app (a force quit, a crash) doesn't reach it.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if d.request(&Request::Hello).is_ok() {
                // It lives on its own; reap it if it ever exits while the app runs.
                std::thread::spawn(move || child.wait());
                return Ok(d);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "the session daemon didn't start"))
    }

    fn send(&self, req: &Request) -> io::Result<UnixStream> {
        let conn = UnixStream::connect(&self.sock)?;
        conn.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut line = serde_json::to_string(req).map_err(io::Error::other)?;
        line.push('\n');
        (&conn).write_all(line.as_bytes())?;
        Ok(conn)
    }

    pub fn request(&self, req: &Request) -> io::Result<Reply> {
        let conn = self.send(req)?;
        let mut line = String::new();
        BufReader::new(conn).read_line(&mut line)?;
        let r: Reply = serde_json::from_str(&line).map_err(io::Error::other)?;
        match r.error {
            Some(e) => Err(io::Error::other(e)),
            None => Ok(r),
        }
    }

    /// Attach to a tab (starting it if needed). Returns the reply, the pty's master and the
    /// connection that now carries the tab's bytes.
    pub fn open(&self, tab: &str, size: Size, spawn: Spawn) -> io::Result<(Reply, OwnedFd, UnixStream)> {
        let conn = self.send(&Request::Open { tab: tab.into(), size, spawn })?;
        // The reply line comes with the fd; read it a byte at a time so nothing after it (the
        // tab's first output) is taken from the socket.
        let mut line = Vec::new();
        let mut fd = None;
        loop {
            let mut b = [0u8; 1];
            let (n, f) = recv_with_fd(&conn, &mut b)?;
            if f.is_some() {
                fd = f;
            }
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if b[0] == b'\n' {
                break;
            }
            line.push(b[0]);
        }
        let r: Reply = serde_json::from_slice(&line).map_err(io::Error::other)?;
        if let Some(e) = r.error {
            return Err(io::Error::other(e));
        }
        let fd = fd.ok_or_else(|| io::Error::other("no pty in the daemon's reply"))?;
        conn.set_read_timeout(None)?;
        Ok((r, fd, conn))
    }

    pub fn resize(&self, tab: &str, size: Size) {
        if let Err(e) = self.request(&Request::Resize { tab: tab.into(), size }) {
            log::warn!("session daemon: resize failed: {e}");
        }
    }

    pub fn close(&self, tab: &str) {
        let _ = self.request(&Request::Close { tab: tab.into() });
    }

    /// End every tab not in `tabs`.
    pub fn keep(&self, tabs: Vec<String>) {
        let _ = self.request(&Request::Keep { tabs });
    }

    /// End every tab and stop the daemon.
    pub fn quit(&self) {
        let _ = self.request(&Request::Quit);
    }
}

/// One tab's connection, as alacritty's event loop sees a pty. Output and typing go over the
/// socket; resizes and the exit status are requests.
pub struct RemotePty {
    daemon: Arc<Daemon>,
    tab: String,
    stream: UnixStream,
    /// Made readable when the connection ends, so the loop asks `next_child_event`.
    ended: (UnixStream, UnixStream),
    /// Set on detach (Restart T1K, updates): dropping doesn't end the tab.
    pub keep: Arc<std::sync::atomic::AtomicBool>,
    /// Resizes go to the daemon from a thread of their own, latest first.
    resizes: std::sync::mpsc::Sender<Size>,
}

pub struct RemoteReader {
    stream: UnixStream,
    ended: UnixStream,
    done: bool,
}

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        match self.stream.read(buf) {
            Ok(0) => {
                self.done = true;
                let _ = (&self.ended).write_all(&[1]);
                Err(io::ErrorKind::WouldBlock.into())
            }
            r => r,
        }
    }
}

impl RemotePty {
    /// Returns the pty and the reader the event loop parses from.
    pub fn new(daemon: Arc<Daemon>, tab: String, stream: UnixStream) -> io::Result<(Self, RemoteReader)> {
        stream.set_nonblocking(true)?;
        let ended = UnixStream::pair()?;
        ended.0.set_nonblocking(true)?;
        ended.1.set_nonblocking(true)?;
        let reader = RemoteReader { stream: stream.try_clone()?, ended: ended.1.try_clone()?, done: false };
        let (resizes, rx) = std::sync::mpsc::channel::<Size>();
        {
            let (daemon, tab) = (daemon.clone(), tab.clone());
            std::thread::Builder::new().name("resize".into()).spawn(move || {
                while let Ok(mut size) = rx.recv() {
                    while let Ok(later) = rx.try_recv() {
                        size = later;
                    }
                    daemon.resize(&tab, size);
                }
            })?;
        }
        Ok((RemotePty { daemon, tab, stream, ended, keep: Default::default(), resizes }, reader))
    }
}

impl Drop for RemotePty {
    fn drop(&mut self) {
        if !self.keep.load(std::sync::atomic::Ordering::Relaxed) && !detaching() {
            self.daemon.close(&self.tab);
        }
    }
}

impl EventedReadWrite for RemotePty {
    type Reader = UnixStream;
    type Writer = UnixStream;

    unsafe fn register(&mut self, poll: &Arc<Poller>, mut interest: Event, mode: PollMode) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        unsafe {
            poll.add_with_mode(&self.stream, interest, mode)?;
            poll.add_with_mode(&self.ended.0, Event::readable(PTY_CHILD_EVENT_TOKEN), PollMode::Level)
        }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, mut interest: Event, mode: PollMode) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        poll.modify_with_mode(self.stream.as_fd(), interest, mode)?;
        poll.modify_with_mode(self.ended.0.as_fd(), Event::readable(PTY_CHILD_EVENT_TOKEN), PollMode::Level)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(self.stream.as_fd())?;
        poll.delete(self.ended.0.as_fd())
    }

    fn reader(&mut self) -> &mut UnixStream {
        // Unused: `ScanPty` reads through `RemoteReader`.
        &mut self.stream
    }

    fn writer(&mut self) -> &mut UnixStream {
        &mut self.stream
    }
}

impl EventedPty for RemotePty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut b = [0u8; 1];
        if (&self.ended.0).read(&mut b).is_err() {
            return None;
        }
        // The connection ended: the shell exited (or the daemon is gone).
        let status = self.daemon.request(&Request::Status { tab: self.tab.clone() }).ok().and_then(|r| r.exit);
        Some(ChildEvent::Exited(status.map(std::process::ExitStatus::from_raw)))
    }
}

impl OnResize for RemotePty {
    fn on_resize(&mut self, size: WindowSize) {
        let size = Size { cols: size.num_cols, rows: size.num_lines, cell_width: size.cell_width, cell_height: size.cell_height };
        let _ = self.resizes.send(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_until(stream: &mut UnixStream, needle: &str) -> String {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&got).contains(needle) {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        String::from_utf8_lossy(&got).into_owned()
    }

    #[test]
    fn tabs_outlive_the_connection_and_come_back_with_their_screen() {
        let dir = std::env::temp_dir().join(format!("t1kd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let dir = dir.clone();
            std::thread::spawn(move || crate::daemon::run(&dir));
        }
        let d = Daemon { sock: crate::daemon::socket_path(&dir) };
        let start = Instant::now();
        while d.request(&Request::Hello).is_err() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let size = Size { cols: 40, rows: 10, cell_width: 8, cell_height: 16 };
        let spawn = Spawn { command: Some(vec!["/bin/cat".into()]), ..Default::default() };

        let (r, _fd, mut s) = d.open("t", size, spawn.clone()).unwrap();
        assert!(r.new && r.pid > 0);
        s.write_all(b"first words\n").unwrap();
        assert!(read_until(&mut s, "first words").contains("first words"));
        drop(s);

        // Attaching again finds it running, and the screen arrives before anything new.
        let (r2, _fd, mut s2) = d.open("t", size, spawn).unwrap();
        assert!(!r2.new);
        assert_eq!(r2.pid, r.pid);
        assert!(read_until(&mut s2, "first words").contains("first words"));
        assert_eq!(d.request(&Request::List).unwrap().tabs, vec!["t".to_string()]);

        d.close("t");
        let start = Instant::now();
        while unsafe { libc::kill(r.pid, 0) } == 0 && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(d.request(&Request::List).unwrap().tabs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
