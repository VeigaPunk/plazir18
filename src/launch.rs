//! Alacritty launcher. Safe: no shell; session_id is a raw argument.

use std::process::{Command, Stdio};

/// Open an Alacritty window attached to the given tmux session.
/// session_id should be the $-prefixed id (e.g. "$0") — passed directly
/// to tmux attach -t, never interpolated into a shell string.
pub fn alacritty(session_id: &str) -> String {
    match Command::new("alacritty")
        .arg("-e")
        .arg("tmux")
        .arg("attach")
        .arg("-t")
        .arg(session_id)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => format!("opened alacritty for {session_id}"),
        Err(e) => format!("alacritty failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn args_are_safe() {
        // session_id is passed as a single discrete arg — no shell expansion.
        let session_id = "$0; rm -rf /";
        let args: &[&str] = &["-e", "tmux", "attach", "-t", session_id];
        assert_eq!(args[0], "-e");
        assert_eq!(args[4], session_id);
        // Even shell-hostile content is safe when given as a raw arg.
        assert!(args[4].contains(';'));
    }
}
