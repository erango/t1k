//! Unix socket between `t1k hook` (inside a tab) and the app.
//!
//! Wire format: one JSON object per connection, `{"tab_id": "...", "payload": {hook JSON}}`.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Envelope {
    pub tab_id: String,
    pub payload: serde_json::Value,
}

/// Bind the socket and forward each message to `tx` from a background thread.
/// `notify` is called after each message so the UI can wake up.
pub fn serve(path: &Path, tx: Sender<Envelope>, notify: impl Fn() + Send + 'static) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path)?;
    std::thread::Builder::new().name("t1k-agent-socket".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = Vec::new();
            if conn.take(4 << 20).read_to_end(&mut buf).is_err() {
                continue;
            }
            match serde_json::from_slice::<Envelope>(&buf) {
                Ok(env) => {
                    if tx.send(env).is_err() {
                        break;
                    }
                    notify();
                }
                Err(e) => log::warn!("bad agent message: {e}"),
            }
        }
    })?;
    Ok(())
}

/// Send one message (used by `t1k hook`). Short timeouts: a hook must never stall Claude.
pub fn send(path: &Path, env: &Envelope) -> std::io::Result<()> {
    let mut s = UnixStream::connect(path)?;
    s.set_write_timeout(Some(Duration::from_millis(300)))?;
    s.write_all(&serde_json::to_vec(env)?)?;
    Ok(())
}

/// Remove sockets left by T1K processes that are gone (crashes, kills, older builds).
pub fn remove_stale(support_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(support_dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("t1k-"))
            .and_then(|n| n.strip_suffix(".sock"))
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() as i32 && !alive(pid) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn alive(pid: i32) -> bool {
    // Signal 0 checks existence; EPERM still means the process exists.
    unsafe { libc::kill(pid, 0) == 0 || *libc::__error() == libc::EPERM }
}

/// Per-process socket path, so several T1K instances never collide.
pub fn socket_path(support_dir: &Path) -> PathBuf {
    support_dir.join(format!("t1k-{}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let path = std::env::temp_dir().join(format!("t1k-test-{}.sock", std::process::id()));
        let (tx, rx) = std::sync::mpsc::channel();
        serve(&path, tx, || {}).unwrap();
        let env = Envelope { tab_id: "t1".into(), payload: serde_json::json!({"hook_event_name": "Stop"}) };
        send(&path, &env).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(got.tab_id, "t1");
        assert_eq!(got.payload["hook_event_name"], "Stop");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn removes_only_dead_sockets() {
        let dir = std::env::temp_dir().join(format!("t1k-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = socket_path(&dir);
        let dead = dir.join("t1k-999999.sock");
        let other = dir.join("notes.txt");
        for p in [&mine, &dead, &other] {
            std::fs::write(p, "").unwrap();
        }
        remove_stale(&dir);
        assert!(mine.exists() && other.exists() && !dead.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
