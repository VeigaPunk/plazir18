//! Named FIFO wake source. Poked by tmux lifecycle hooks and pipe-pane streams;
//! the poller blocks on the resulting channel instead of a fixed timer.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::SyncSender;

/// Path to the wake FIFO ($XDG_RUNTIME_DIR/agent-wall.wake, fallback /tmp).
pub fn fifo_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join("agent-wall.wake")
}

/// (Re)create the FIFO. A stale path from a prior run is removed first so we
/// never open a leftover regular file, and so any backlog a crashed run left in
/// the pipe buffer is discarded. Returns the path as a string for embedding in
/// the tmux writer commands.
pub fn ensure_fifo() -> String {
    let path = fifo_path();
    remove_fifo_at(&path);
    let _ = Command::new("mkfifo").arg(&path).status();
    path.to_string_lossy().into_owned()
}

/// Delete the FIFO so the tmux hooks — which are GLOBAL and outlive this
/// process — fail their `[ -p ]` guard and become no-ops. Call on exit
/// alongside `tmux::remove_hooks`.
#[allow(dead_code, reason = "wired by the main.rs exit path")]
pub fn remove_fifo() {
    remove_fifo_at(&fifo_path());
}

fn remove_fifo_at(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Shell command a global tmux hook runs to poke the wake FIFO.
///
/// GNU `dd`'s `oflag=nonblock` makes a reader-less FIFO fail with ENXIO rather
/// than parking a hook shell in the kernel. The `[ -p ]` guard disarms a global
/// hook that outlived its GUI process once cleanup removes the FIFO.
pub fn poke_command(fifo: &str) -> String {
    let quoted = shell_quote(fifo);
    format!("[ -p {quoted} ] && printf . | dd of={quoted} oflag=nonblock status=none 2>/dev/null")
}

/// Shell command `pipe-pane` runs to stream a pane into the wake FIFO. Same
/// non-blocking open and stale-hook guard as [`poke_command`].
pub fn stream_command(fifo: &str) -> String {
    let quoted = shell_quote(fifo);
    format!("[ -p {quoted} ] && dd of={quoted} oflag=nonblock status=none 2>/dev/null")
}

/// Deliver one wake byte in-process, never blocking.
///
/// A non-blocking write-open of a FIFO fails with ENXIO when no reader is
/// attached and the write returns EAGAIN rather than parking when the pipe
/// buffer is full — both are the intended silent skip. Returns whether a byte
/// actually landed.
#[allow(dead_code, reason = "wired by the main.rs --poke arm")]
#[cfg(unix)]
pub fn poke(path: &Path) -> bool {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    const O_NONBLOCK: i32 = if cfg!(any(target_os = "linux", target_os = "android")) {
        0o4000
    } else {
        0x0004
    };

    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .is_ok_and(|mut fifo| fifo.write_all(b".").is_ok())
}

#[allow(dead_code, reason = "wired by the main.rs --poke arm")]
#[cfg(not(unix))]
pub fn poke(_path: &Path) -> bool {
    false
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("agent-wall-wake-tests");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn mkfifo_at(path: &PathBuf) {
        let _ = Command::new("mkfifo").arg(path).status();
        assert!(path.exists(), "mkfifo failed for {path:?}");
    }

    /// THE regression lock for the 66-stuck-`sh` leak: the generated hook must
    /// never contain a plain append redirect, because `open(fifo, O_WRONLY)`
    /// blocks forever when no reader exists.
    #[test]
    fn poke_command_never_uses_a_blocking_append_redirect() {
        let cmd = poke_command("/run/user/1000/agent-wall.wake");

        assert!(
            !cmd.contains(">>"),
            "append redirect blocks in open(2) with no reader: {cmd}"
        );
    }

    /// The hook writer must request O_NONBLOCK: O_RDWR avoids the open-time
    /// block, but can still leave bytes in a dead FIFO until its buffer fills.
    #[test]
    fn poke_command_uses_a_nonblocking_writer() {
        let cmd = poke_command("/run/user/1000/agent-wall.wake");

        assert!(cmd.contains("dd"), "no FIFO writer in {cmd}");
        assert!(
            cmd.contains("oflag=nonblock"),
            "writer can block after the FIFO buffer fills: {cmd}"
        );
    }

    /// A global tmux hook outlives the process, so it must no-op once the FIFO
    /// is gone rather than resurrect a stale path as a regular file.
    #[test]
    fn poke_command_is_guarded_by_fifo_existence() {
        let cmd = poke_command("/run/user/1000/agent-wall.wake");

        assert!(
            cmd.contains("[ -p "),
            "hook is not guarded on FIFO existence: {cmd}"
        );
    }

    #[test]
    fn poke_command_shell_quotes_the_path() {
        let cmd = poke_command("/tmp/$(touch /tmp/pwned)' wall");

        assert!(
            cmd.contains("'/tmp/$(touch /tmp/pwned)'\\'' wall'"),
            "path not single-quote escaped: {cmd}"
        );
        assert!(
            !cmd.contains("$(touch /tmp/pwned) "),
            "injection window: {cmd}"
        );
    }

    #[test]
    fn stream_command_never_uses_a_blocking_append_redirect() {
        let cmd = stream_command("/run/user/1000/agent-wall.wake");

        assert!(!cmd.contains(">>"), "append redirect blocks: {cmd}");
        assert!(
            cmd.contains("oflag=nonblock"),
            "stream writer can block after the FIFO buffer fills: {cmd}"
        );
        assert!(cmd.contains("[ -p "), "unguarded: {cmd}");
    }

    /// The in-process primitive: a reader-less FIFO must fail instantly
    /// (ENXIO from the non-blocking open), never park the caller.
    #[test]
    #[cfg(unix)]
    fn poke_returns_immediately_on_a_readerless_fifo() {
        let path = scratch("readerless");
        mkfifo_at(&path);

        let start = Instant::now();
        let delivered = poke(&path);
        let elapsed = start.elapsed();

        assert!(!delivered, "claimed delivery with no reader attached");
        assert!(
            elapsed < Duration::from_millis(250),
            "poke parked for {elapsed:?} — still blocking in open(2)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn poke_delivers_a_byte_when_a_reader_is_attached() {
        let path = scratch("with-reader");
        mkfifo_at(&path);

        let mut reader = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open reader end");

        assert!(poke(&path), "poke reported failure with a reader attached");

        let mut buf = [0u8; 16];
        let n = reader.read(&mut buf).expect("read poke byte");
        assert_eq!(n, 1, "expected exactly one poke byte, got {n}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn poke_on_a_missing_path_creates_nothing() {
        let path = scratch("absent");

        assert!(!poke(&path), "poke claimed success on a missing FIFO");
        assert!(!path.exists(), "poke created {path:?} as a regular file");
    }

    #[test]
    fn remove_fifo_disarms_a_stale_hook() {
        let path = scratch("disarm");
        mkfifo_at(&path);

        remove_fifo_at(&path);

        assert!(!path.exists(), "FIFO survived cleanup: {path:?}");
    }
}
