//! The session daemon: a background process (`t1k daemon <dir>`) that owns every tab's shell,
//! so a crash, a freeze, an update or Restart T1K doesn't end what's running in them.
//!
//! For each tab it is the only reader of the pty. Output goes into its own copy of the
//! terminal (an alacritty `Term`) and, while the app is attached, on to the app over a unix
//! socket. When an app attaches to a tab that was already running, the daemon first sends
//! `replay::replay` of its copy, so the app's fresh grid matches, then the live output.
//!
//! Requests are one JSON line on a new connection to `<dir>/t1k-daemon.sock`, answered with
//! one JSON line. `open` also passes the pty's master fd (SCM_RIGHTS), so the app can ask the
//! kernel about the foreground process and terminal modes as before, and the connection then
//! carries the tab's bytes: output from the daemon, typed input from the app.
//!
//! Never logs terminal contents.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::term::{test::TermSize, Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use serde::{Deserialize, Serialize};

/// Bumped when requests or replies change in a way an older daemon or app can't read.
pub const PROTOCOL: u32 = 1;

/// History lines kept per tab, as in the app.
const HISTORY: usize = 10_000;

/// Output queued for an app that isn't reading before the daemon lets go of it.
const QUEUE_CHUNKS: usize = 8192;

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("t1k-daemon.sock")
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Spawn {
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub command: Option<Vec<String>>,
    pub start_at_bottom: bool,
    /// Scrollback saved from the last run, replayed into the daemon's copy too.
    pub restore: Option<String>,
    pub kitty_keyboard: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Size {
    fn window(self) -> WindowSize {
        WindowSize { num_cols: self.cols, num_lines: self.rows, cell_width: self.cell_width, cell_height: self.cell_height }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello,
    /// Attach to the tab, starting its shell first if it isn't running.
    Open { tab: String, size: Size, spawn: Spawn },
    Resize { tab: String, size: Size },
    /// End the tab's shell (SIGHUP, as closing a terminal window does).
    Close { tab: String },
    Status { tab: String },
    List,
    /// End every tab whose id isn't listed (tabs closed while the app was gone).
    Keep { tabs: Vec<String> },
    /// End every tab and exit.
    Quit,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub protocol: u32,
    #[serde(default)]
    pub version: String,
    /// Open: the shell's pid, whether it was started now, and the size its screen has.
    #[serde(default)]
    pub pid: i32,
    #[serde(default)]
    pub new: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Status: the raw wait status once the shell has exited; `alive` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(default)]
    pub alive: bool,
    /// List: running tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<String>,
}

impl Reply {
    fn ok() -> Self {
        Reply { ok: true, protocol: PROTOCOL, version: env!("CARGO_PKG_VERSION").into(), ..Default::default() }
    }
    fn err(e: impl ToString) -> Self {
        Reply { error: Some(e.to_string()), protocol: PROTOCOL, ..Default::default() }
    }
}

// ---- fd passing --------------------------------------------------------------------------

/// Send `data` with `fd` attached.
pub fn send_with_fd(sock: &UnixStream, data: &[u8], fd: RawFd) -> io::Result<()> {
    unsafe {
        let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut _, iov_len: data.len() };
        let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr() as *mut _;
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg) as *mut RawFd, fd);
        if libc::sendmsg(sock.as_raw_fd(), &msg, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Receive up to `buf.len()` bytes and the fd sent with them, if any.
pub fn recv_with_fd(sock: &UnixStream, buf: &mut [u8]) -> io::Result<(usize, Option<OwnedFd>)> {
    unsafe {
        let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut _, iov_len: buf.len() };
        let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr() as *mut _;
        msg.msg_controllen = space as _;
        let n = libc::recvmsg(sock.as_raw_fd(), &mut msg, 0);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut fd = None;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if !cmsg.is_null() && (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
            let raw = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg) as *const RawFd);
            libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC);
            fd = Some(OwnedFd::from_raw_fd(raw));
        }
        Ok((n as usize, fd))
    }
}

// ---- the daemon --------------------------------------------------------------------------

/// What the daemon's copy of a terminal needs from outside its lock: where its answers to a
/// program's queries go while no app is attached, and the title.
struct Shared {
    /// The pty's master, for answers to queries (cursor position, device attributes) while no
    /// app is attached to answer them itself.
    pty: Option<std::fs::File>,
    attached: bool,
    title: Option<String>,
}

#[derive(Clone)]
struct Mirror(Arc<Mutex<Shared>>);

impl EventListener for Mirror {
    fn send_event(&self, event: AlacEvent) {
        let Ok(mut s) = self.0.lock() else { return };
        match event {
            AlacEvent::PtyWrite(text) if !s.attached => {
                if let Some(pty) = s.pty.as_mut() {
                    let _ = pty.write_all(text.as_bytes());
                }
            }
            AlacEvent::Title(t) => s.title = Some(t),
            AlacEvent::ResetTitle => s.title = None,
            _ => {}
        }
    }
}

struct Client {
    gen: u64,
    out: SyncSender<Vec<u8>>,
    stream: UnixStream,
}

struct State {
    term: Term<Mirror>,
    parser: Processor<StdSyncHandler>,
    /// Owns the child: dropping it hangs the shell up.
    pty: Option<tty::Pty>,
    size: Size,
    client: Option<Client>,
    gen: u64,
    exit: Option<i32>,
}

struct Tab {
    pid: i32,
    master: std::fs::File,
    shared: Mirror,
    state: Mutex<State>,
}

impl Tab {
    fn alive(&self) -> bool {
        self.state.lock().map(|s| s.exit.is_none()).unwrap_or(false)
    }
}

struct Daemon {
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    last_client: Mutex<Instant>,
}

/// Run the daemon for `dir` until told to quit, or until nothing has run and no app has been
/// attached for a while. Returns when another daemon already serves `dir`.
pub fn run(dir: &Path) -> io::Result<()> {
    let path = socket_path(dir);
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let daemon = Arc::new(Daemon { tabs: Mutex::new(HashMap::new()), last_client: Mutex::new(Instant::now()) });
    {
        let daemon = daemon.clone();
        let path = path.clone();
        std::thread::Builder::new().name("idle".into()).spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            if daemon.idle() {
                log::info!("nothing running and no app for a while; exiting");
                let _ = std::fs::remove_file(&path);
                std::process::exit(0);
            }
        })?;
    }
    log::info!("serving {} (protocol {PROTOCOL})", path.display());
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let daemon = daemon.clone();
        let path = path.clone();
        let _ = std::thread::Builder::new().name("request".into()).spawn(move || {
            if let Err(e) = daemon.handle(conn, &path) {
                log::warn!("request failed: {e}");
            }
        });
    }
    Ok(())
}

impl Daemon {
    fn idle(&self) -> bool {
        let tabs = self.tabs.lock().unwrap();
        let running = tabs.values().any(|t| t.alive());
        let attached = tabs.values().any(|t| t.state.lock().map(|s| s.client.is_some()).unwrap_or(false));
        if running || attached {
            *self.last_client.lock().unwrap() = Instant::now();
            return false;
        }
        self.last_client.lock().unwrap().elapsed() > Duration::from_secs(60)
    }

    fn tab(&self, id: &str) -> Option<Arc<Tab>> {
        self.tabs.lock().unwrap().get(id).cloned()
    }

    fn handle(self: &Arc<Self>, conn: UnixStream, path: &Path) -> io::Result<()> {
        let mut line = String::new();
        BufReader::new(conn.try_clone()?).take(1 << 20).read_line(&mut line)?;
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => return reply(&conn, &Reply::err(format!("bad request: {e}"))),
        };
        match req {
            Request::Hello => reply(&conn, &Reply::ok()),
            Request::Open { tab, size, spawn } => self.open(conn, tab, size, spawn),
            Request::Resize { tab, size } => {
                if let Some(t) = self.tab(&tab) {
                    t.resize(size);
                }
                reply(&conn, &Reply::ok())
            }
            Request::Close { tab } => {
                let t = self.tabs.lock().unwrap().remove(&tab);
                if let Some(t) = t {
                    t.end();
                }
                reply(&conn, &Reply::ok())
            }
            Request::Status { tab } => {
                let mut r = Reply::ok();
                if let Some(t) = self.tab(&tab) {
                    let s = t.state.lock().unwrap();
                    r.exit = s.exit;
                    r.alive = s.exit.is_none();
                    r.pid = t.pid;
                }
                reply(&conn, &r)
            }
            Request::List => {
                let mut r = Reply::ok();
                r.tabs = self.tabs.lock().unwrap().iter().filter(|(_, t)| t.alive()).map(|(id, _)| id.clone()).collect();
                reply(&conn, &r)
            }
            Request::Keep { tabs } => {
                let gone: Vec<Arc<Tab>> = {
                    let mut all = self.tabs.lock().unwrap();
                    let ids: Vec<String> = all.keys().filter(|id| !tabs.contains(id)).cloned().collect();
                    ids.iter().filter_map(|id| all.remove(id)).collect()
                };
                for t in gone {
                    t.end();
                }
                reply(&conn, &Reply::ok())
            }
            Request::Quit => {
                let all: Vec<Arc<Tab>> = self.tabs.lock().unwrap().drain().map(|(_, t)| t).collect();
                for t in all {
                    t.end();
                }
                reply(&conn, &Reply::ok())?;
                let _ = std::fs::remove_file(path);
                log::info!("told to quit");
                std::process::exit(0);
            }
        }
    }

    fn open(self: &Arc<Self>, conn: UnixStream, id: String, size: Size, spawn: Spawn) -> io::Result<()> {
        let existing = self.tab(&id).filter(|t| t.alive());
        let (tab, new) = match existing {
            Some(t) => (t, false),
            None => match Tab::spawn(&id, size, spawn) {
                Ok(t) => {
                    self.tabs.lock().unwrap().insert(id.clone(), t.clone());
                    (t, true)
                }
                Err(e) => return reply(&conn, &Reply::err(format!("couldn't start the shell: {e}"))),
            },
        };
        tab.attach(conn, new)
    }
}

fn reply(conn: &UnixStream, r: &Reply) -> io::Result<()> {
    let mut line = serde_json::to_string(r).map_err(io::Error::other)?;
    line.push('\n');
    (&*conn).write_all(line.as_bytes())
}

/// The user's shell as a login session (see `terminal::login_shell`).
fn shell(command: Option<Vec<String>>) -> Option<tty::Shell> {
    match command {
        Some(c) if !c.is_empty() => Some(tty::Shell::new(c[0].clone(), c[1..].to_vec())),
        _ => crate::terminal::login_shell(),
    }
}

impl Tab {
    fn spawn(id: &str, size: Size, spawn: Spawn) -> io::Result<Arc<Tab>> {
        let opts = tty::Options { shell: shell(spawn.command), working_directory: spawn.cwd, drain_on_exit: true, env: spawn.env };
        let pty = tty::new(&opts, size.window(), 0)?;
        let master = pty.file().try_clone()?;
        let pid = pty.child().id() as i32;
        let shared = Mirror(Arc::new(Mutex::new(Shared { pty: Some(master.try_clone()?), attached: false, title: None })));
        let config = Config { scrolling_history: HISTORY, kitty_keyboard: spawn.kitty_keyboard, ..Config::default() };
        let mut term = Term::new(config, &TermSize::new(size.cols as usize, size.rows as usize), shared.clone());
        let mut parser: Processor<StdSyncHandler> = Default::default();
        // The same start as the app's own copy (`Terminal::spawn`), so both grids agree.
        if spawn.start_at_bottom {
            use alacritty_terminal::vte::ansi::Handler;
            for _ in 0..400 {
                term.linefeed();
            }
        }
        if let Some(r) = spawn.restore.as_deref().filter(|r| !r.is_empty()) {
            parser.advance(&mut term, r.as_bytes());
        }
        let tab = Arc::new(Tab { pid, master, shared, state: Mutex::new(State { term, parser, pty: Some(pty), size, client: None, gen: 0, exit: None }) });
        let reader = tab.clone();
        let name = format!("pty {}", &id[..id.len().min(8)]);
        std::thread::Builder::new().name(name).spawn(move || reader.read_loop())?;
        log::info!("started a shell (pid {pid})");
        Ok(tab)
    }

    /// Read the pty until the shell is gone, keeping the copy current and passing output on.
    fn read_loop(self: Arc<Self>) {
        let fd = self.master.as_raw_fd();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut p, 1, -1) } < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n > 0 {
                let bytes = &buf[..n as usize];
                let mut s = self.state.lock().unwrap();
                let State { term, parser, .. } = &mut *s;
                parser.advance(term, bytes);
                let dropped = match &s.client {
                    Some(c) => match c.out.try_send(bytes.to_vec()) {
                        Ok(()) => false,
                        Err(TrySendError::Full(_)) => {
                            log::warn!("app stopped reading a tab; letting it go");
                            true
                        }
                        Err(TrySendError::Disconnected(_)) => true,
                    },
                    None => false,
                };
                if dropped {
                    self.detach(&mut s);
                }
                continue;
            }
            if n < 0 && matches!(io::Error::last_os_error().kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) {
                continue;
            }
            break;
        }
        // The shell is gone (EOF, or EIO once the last slave fd closed).
        let mut status = 0;
        let got = unsafe { libc::waitpid(self.pid, &mut status, 0) };
        let mut s = self.state.lock().unwrap();
        s.exit = Some(if got == self.pid { status } else { 0 });
        log::info!("shell exited (pid {})", self.pid);
        // Closing the connection is how the app learns; it asks for the status next.
        self.detach(&mut s);
        if let Some(pty) = s.pty.take() {
            drop_pty(pty);
        }
    }

    fn detach(&self, s: &mut State) {
        if let Some(c) = s.client.take() {
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Ok(mut sh) = self.shared.0.lock() {
            sh.attached = false;
        }
    }

    fn attach(self: &Arc<Self>, conn: UnixStream, new: bool) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        // A newer attach wins (the app restarted while the old one hung on).
        self.detach(&mut s);
        let mut r = Reply::ok();
        r.pid = self.pid;
        r.new = new;
        r.size = Some(s.size);
        let mut line = serde_json::to_string(&r).map_err(io::Error::other)?;
        line.push('\n');
        send_with_fd(&conn, line.as_bytes(), self.master.as_raw_fd())?;
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_CHUNKS);
        if !new {
            // Start the app's grid over, then rebuild it.
            let title = self.shared.0.lock().ok().and_then(|sh| sh.title.clone());
            let mut first = b"\x1bc\x1b[3J".to_vec();
            first.extend(crate::replay::replay(&s.term, title.as_deref()));
            let _ = tx.try_send(first);
        }
        s.gen += 1;
        let gen = s.gen;
        let writer = conn.try_clone()?;
        std::thread::Builder::new().name("to app".into()).spawn(move || {
            let mut w = writer;
            for chunk in rx {
                if w.write_all(&chunk).is_err() {
                    break;
                }
            }
        })?;
        let input = conn.try_clone()?;
        s.client = Some(Client { gen, out: tx, stream: conn });
        if let Ok(mut sh) = self.shared.0.lock() {
            sh.attached = true;
        }
        drop(s);
        let tab = self.clone();
        std::thread::Builder::new().name("from app".into()).spawn(move || tab.input_loop(input, gen))?;
        Ok(())
    }

    /// Pass what the app sends (typing, pastes, answers to queries) to the pty.
    fn input_loop(&self, mut conn: UnixStream, gen: u64) {
        let mut buf = vec![0u8; 16 * 1024];
        let mut master = match self.master.try_clone() {
            Ok(m) => m,
            Err(_) => return,
        };
        loop {
            match conn.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if write_all_blocking(&mut master, &buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let mut s = self.state.lock().unwrap();
        if s.client.as_ref().is_some_and(|c| c.gen == gen) {
            self.detach(&mut s);
        }
    }

    fn resize(&self, size: Size) {
        let mut s = self.state.lock().unwrap();
        if size.cols == 0 || size.rows == 0 {
            return;
        }
        let changed = s.size != size;
        s.size = size;
        s.term.resize(TermSize::new(size.cols as usize, size.rows as usize));
        if let Some(pty) = s.pty.as_mut() {
            pty.on_resize(size.window());
        }
        // An app that just attached asks for its size; when that's the size the program
        // already has, it gets no SIGWINCH and wouldn't redraw what the copy can't carry
        // (the scroll region, the main screen under an alternate one).
        if !changed {
            let pgid = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
            if pgid > 0 {
                unsafe { libc::killpg(pgid, libc::SIGWINCH) };
            }
        }
    }

    /// Hang the shell up and forget the tab.
    fn end(&self) {
        let mut s = self.state.lock().unwrap();
        self.detach(&mut s);
        if let Ok(mut sh) = self.shared.0.lock() {
            sh.pty = None;
        }
        if let Some(pty) = s.pty.take() {
            drop_pty(pty);
        }
    }
}

/// Dropping a `Pty` sends SIGHUP and then waits for the child, which a program that ignores
/// SIGHUP never ends: do it off the thread that asked.
fn drop_pty(pty: tty::Pty) {
    let _ = std::thread::Builder::new().name("hang up".into()).spawn(move || drop(pty));
}

/// The master is non-blocking (alacritty sets it so); wait for room instead of failing.
fn write_all_blocking(f: &mut std::fs::File, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        match f.write(data) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                unsafe { libc::poll(&mut p, 1, 1000) };
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
