//! Best-effort open URL in the user browser (feature = "oauth").

use std::process::Command;

/// Try `xdg-open` / `open` / `cmd start`. Returns Ok if a launcher was spawned.
/// Never panics; failure is soft (caller still prints the URL).
pub fn open_url_best_effort(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("empty url".into());
    }
    // Prefer platform-native openers without shell interpolation of the URL.
    let attempts: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("open", &[])]
    } else if cfg!(target_os = "windows") {
        &[("cmd", &["/C", "start", ""])]
    } else {
        &[("xdg-open", &[]), ("gio", &["open"])]
    };
    let mut last = String::from("no opener found");
    for (bin, prefix) in attempts {
        let mut cmd = Command::new(bin);
        for p in *prefix {
            cmd.arg(p);
        }
        cmd.arg(url);
        match cmd.spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last = format!("{bin}: {e}"),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_errs() {
        assert!(open_url_best_effort("").is_err());
    }
}
