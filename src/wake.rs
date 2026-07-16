//! Named FIFO wake source. Poked by tmux lifecycle hooks and pipe-pane streams;
//! the poller blocks on the resulting channel instead of a fixed timer.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::SyncSender;

/// Path to the wake FIFO ($XDG_RUNTIME_DIR/agent-wall.wake, fallback /tmp).
pub fn fifo_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join("agent-wall.wake")
}

/// (Re)create the FIFO. A stale path from a prior run is removed first so we
/// never open a leftover regular file. Returns the path as a string for
/// embedding in the tmux writer commands.
pub fn ensure_fifo() -> String {
    let path = fifo_path();
    let _ = std::fs::remove_file(&path);
    let _ = Command::new("mkfifo").arg(&path).status();
    path.to_string_lossy().into_owned()
}

/// Spawn the reader thread. The FIFO is opened read **and** write so its read
/// end never reports EOF as tmux writers (`printf`, `pipe-pane` cat) come and
/// go; each burst of bytes forwards one coalesced poke to the poller.
pub fn spawn_reader(path: PathBuf, tx: SyncSender<()>) {
    std::thread::spawn(move || {
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("agent-wall wake FIFO open failed: {e}");
                return;
            }
        };
        let mut buf = [0u8; 4096];
        loop {
            match file.read(&mut buf) {
                // O_RDWR keeps a writer end open on our side, so a 0-byte read
                // is unexpected; bail rather than spin.
                Ok(0) => return,
                // try_send drops when a poke is already pending — that is the
                // coalescing we want under a burst of output.
                Ok(_) => {
                    let _ = tx.try_send(());
                }
                Err(_) => return,
            }
        }
    });
}
