//! Desktop notifications from programs: OSC 9 (iTerm2, `ESC ] 9 ; body BEL`) and OSC 777
//! (`ESC ] 777 ; notify ; title ; body BEL`, Ghostty / rxvt). The terminal core ignores both,
//! so the pty reader is wrapped and the bytes are scanned on their way to the parser.

use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};

/// A notification a program asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Longest OSC payload kept; longer ones are dropped rather than buffered without end.
const MAX: usize = 4096;

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Esc,
    /// Inside `ESC ]`, collecting the payload.
    Osc,
    /// Inside the payload, after an ESC (maybe the `ESC \` terminator).
    OscEsc,
}

/// Byte-at-a-time scanner; keeps its state across reads, so sequences split between reads work.
#[derive(Default)]
pub struct Scanner {
    state: State,
    buf: Vec<u8>,
    overflow: bool,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Notice>) {
        for &b in bytes {
            self.state = match self.state {
                State::Ground => {
                    if b == 0x1b {
                        State::Esc
                    } else {
                        State::Ground
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.buf.clear();
                        self.overflow = false;
                        State::Osc
                    } else if b == 0x1b {
                        State::Esc
                    } else {
                        State::Ground
                    }
                }
                State::Osc => match b {
                    0x07 => {
                        self.finish(out);
                        State::Ground
                    }
                    0x1b => State::OscEsc,
                    // CAN / SUB abort a sequence.
                    0x18 | 0x1a => State::Ground,
                    _ => {
                        self.push(b);
                        State::Osc
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(out);
                        State::Ground
                    } else if b == b']' {
                        // An unterminated OSC followed by a new one.
                        self.buf.clear();
                        self.overflow = false;
                        State::Osc
                    } else {
                        State::Ground
                    }
                }
            };
        }
    }

    fn push(&mut self, b: u8) {
        if self.buf.len() < MAX {
            self.buf.push(b);
        } else {
            self.overflow = true;
        }
    }

    fn finish(&mut self, out: &mut Vec<Notice>) {
        if !self.overflow {
            if let Some(n) = parse(&self.buf) {
                out.push(n);
            }
        }
        self.buf.clear();
    }
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn parse(payload: &[u8]) -> Option<Notice> {
    let s = String::from_utf8_lossy(payload);
    if let Some(body) = s.strip_prefix("9;") {
        // ConEmu reuses OSC 9 with a number for other things (9;4 is progress); those aren't
        // messages.
        let first = body.split(';').next().unwrap_or("");
        if !first.is_empty() && first.len() <= 2 && first.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let body = clean(body);
        return (!body.is_empty()).then(|| Notice { title: String::new(), body });
    }
    if let Some(rest) = s.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
        let (title, body) = (clean(title), clean(body));
        return (!title.is_empty() || !body.is_empty()).then_some(Notice { title, body });
    }
    None
}

/// The pty, with its output scanned for notifications. Reads go through a second handle on
/// the same file, so polling and writes still use the original.
pub struct ScanPty {
    inner: Pty,
    reader: ScanReader,
}

pub struct ScanReader {
    file: std::fs::File,
    scan: Scanner,
    found: Vec<Notice>,
    sink: Arc<dyn Fn(Notice) + Send + Sync>,
}

impl Read for ScanReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        // Cheap check first: no ESC in the chunk and nothing pending means nothing to do.
        if n > 0 && (!matches!(self.scan.state, State::Ground) || buf[..n].contains(&0x1b)) {
            self.scan.feed(&buf[..n], &mut self.found);
            for notice in self.found.drain(..) {
                (self.sink)(notice);
            }
        }
        Ok(n)
    }
}

impl ScanPty {
    pub fn new(inner: Pty, sink: Arc<dyn Fn(Notice) + Send + Sync>) -> io::Result<Self> {
        let file = inner.file().try_clone()?;
        Ok(Self { inner, reader: ScanReader { file, scan: Scanner::default(), found: Vec::new(), sink } })
    }
}

impl EventedReadWrite for ScanPty {
    type Reader = ScanReader;
    type Writer = std::fs::File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        unsafe { self.inner.register(poll, interest, mode) }
    }
    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.inner.reregister(poll, interest, mode)
    }
    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }
    fn reader(&mut self) -> &mut ScanReader {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut std::fs::File {
        self.inner.writer()
    }
}

impl EventedPty for ScanPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for ScanPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.inner.on_resize(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<Notice> {
        let mut s = Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn osc9_and_777() {
        let n = scan(&[b"hi \x1b]9;Build done\x07 more"]);
        assert_eq!(n, [Notice { title: "".into(), body: "Build done".into() }]);
        let n = scan(&[b"\x1b]777;notify;Tests;42 passed\x1b\\"]);
        assert_eq!(n, [Notice { title: "Tests".into(), body: "42 passed".into() }]);
    }

    #[test]
    fn split_across_reads() {
        let n = scan(&[b"\x1b]9;Dep", b"loy fini", b"shed\x1b", b"\\"]);
        assert_eq!(n[0].body, "Deploy finished");
    }

    #[test]
    fn ignores_other_osc_and_conemu() {
        assert!(scan(&[b"\x1b]0;title\x07\x1b]9;4;1;50\x07\x1b]8;;http://x\x1b\\"]).is_empty());
        assert!(scan(&[b"\x1b]9;\x07"]).is_empty());
    }
}
