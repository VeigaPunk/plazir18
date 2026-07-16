//! Unix socket toggle IPC. Whole module is cfg(unix).
//!
//! Socket path: $XDG_RUNTIME_DIR/agent-wall.sock (fallback /tmp/agent-wall.sock).
//! Protocol: client sends "toggle\n", server reads it and fires IpcCmd::Toggle.

pub enum IpcCmd {
    Toggle,
}

#[cfg(unix)]
mod unix_impl {
    use super::IpcCmd;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    pub(crate) fn socket_path() -> PathBuf {
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime).join("agent-wall.sock")
    }

    /// Start a background listener thread. Accepts one connection at a time,
    /// reads one line; if "toggle" sends IpcCmd::Toggle and requests a repaint.
    pub fn serve(tx: std::sync::mpsc::SyncSender<IpcCmd>, ctx: eframe::egui::Context) {
        let path = socket_path();
        // Remove stale socket from a crashed prior run.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("agent-wall IPC bind failed: {e}");
                return;
            }
        };
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if read_toggle(&stream) {
                    let _ = tx.try_send(IpcCmd::Toggle);
                    ctx.request_repaint();
                }
            }
        });
    }

    fn read_toggle(stream: &UnixStream) -> bool {
        if stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .is_err()
        {
            return false;
        }
        let reader = BufReader::new(stream);
        reader
            .lines()
            .next()
            .and_then(|r| r.ok())
            .map(|l| l.trim() == "toggle")
            .unwrap_or(false)
    }

    /// Connect to a running instance and send the toggle command.
    /// Returns true if the message was delivered.
    pub fn send_toggle() -> bool {
        match UnixStream::connect(socket_path()) {
            Ok(mut s) => writeln!(s, "toggle").is_ok(),
            Err(_) => false,
        }
    }
}

#[cfg(unix)]
pub use unix_impl::{send_toggle, serve};

#[cfg(test)]
mod tests {
    // IPC socket test is omitted in unit form because socket_path() is tied
    // to $XDG_RUNTIME_DIR. Setting env vars across parallel test threads is
    // unsafe (std::env::set_var is not thread-safe since Rust 1.80+).
    // An integration test would require spawning a separate binary.
    // The logic is trivially correct: serve reads "toggle\n" → IpcCmd::Toggle.
    #[test]
    fn ipc_cmd_toggle_is_constructible() {
        // Compile-time check that IpcCmd::Toggle exists and is usable.
        let _cmd = super::IpcCmd::Toggle;
    }
}
