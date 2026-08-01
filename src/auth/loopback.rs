//! Loopback OAuth callback listener (feature = "oauth").
//! Binds `127.0.0.1:1455`, accepts one GET, returns `(code, state)`.

use super::pkce::parse_oauth_callback_query;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Default bind matching [`super::openai::OPENAI_LOOPBACK_REDIRECT`].
pub const LOOPBACK_ADDR: &str = "127.0.0.1:1455";
pub const LOOPBACK_PATH_PREFIX: &str = "/auth/callback";

/// Extract request-target (path + optional query) from a raw HTTP request head.
pub fn http_request_target(request_head: &str) -> Result<String, String> {
    let line = request_head.lines().next().unwrap_or("").trim();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if !method.eq_ignore_ascii_case("GET") {
        return Err(format!("expected GET, got {method}"));
    }
    if target.is_empty() {
        return Err("missing request target".into());
    }
    Ok(target.to_string())
}

/// Parse code/state from an HTTP request-target (`/path?query` or full URL).
pub fn code_state_from_request_target(target: &str) -> Result<(String, String), String> {
    if target.contains('?') || target.contains("code=") {
        parse_oauth_callback_query(target)
    } else {
        Err("callback request has no query".into())
    }
}

fn respond_html(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn read_request_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = [0u8; 8192];
    let mut acc = Vec::new();
    loop {
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        acc.extend_from_slice(&buf[..n]);
        if acc.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if acc.len() > 16_384 {
            return Err("HTTP request head too large".into());
        }
    }
    String::from_utf8(acc).map_err(|e| e.to_string())
}

/// Accept one OAuth redirect on `addr` within `timeout`.
/// Returns `(code, state)` after responding with a tiny success HTML page.
pub fn wait_for_oauth_callback(addr: &str, timeout: Duration) -> Result<(String, String), String> {
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener.set_nonblocking(false).map_err(|e| e.to_string())?;
    // accept timeout via set_read_timeout on accepted streams; for accept itself
    // use a short poll loop with nonblocking accept.
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    let stream = loop {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for OAuth callback on {addr} after {}s",
                timeout.as_secs()
            ));
        }
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let head = read_request_head(&mut stream)?;
    let target = http_request_target(&head)?;
    if !target.starts_with(LOOPBACK_PATH_PREFIX) && !target.contains("code=") {
        respond_html(
            &mut stream,
            "404 Not Found",
            "<html><body>plazir18: unexpected path</body></html>",
        );
        return Err(format!("unexpected path: {target}"));
    }
    match code_state_from_request_target(&target) {
        Ok((code, state)) => {
            respond_html(
                &mut stream,
                "200 OK",
                "<html><body><h1>plazir18</h1><p>OAuth code received. You can close this tab and return to the agent.</p></body></html>",
            );
            Ok((code, state))
        }
        Err(e) => {
            respond_html(
                &mut stream,
                "400 Bad Request",
                &format!("<html><body>plazir18 OAuth error: {e}</body></html>"),
            );
            Err(e)
        }
    }
}

/// Parse loopback `host:port` from a redirect URL like `http://127.0.0.1:1455/auth/callback`.
pub fn bind_addr_from_redirect(redirect_uri: &str) -> Result<String, String> {
    let rest = redirect_uri
        .strip_prefix("http://")
        .or_else(|| redirect_uri.strip_prefix("https://"))
        .ok_or_else(|| "redirect must be http(s)".to_string())?;
    let hostport = rest.split('/').next().unwrap_or("");
    if hostport.is_empty() {
        return Err("empty host in redirect".into());
    }
    // Ensure port present for TcpListener clarity.
    if hostport.contains(':') {
        Ok(hostport.to_string())
    } else {
        Ok(format!("{hostport}:80"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;
    use std::thread;

    #[test]
    fn http_request_target_parses_get() {
        let t =
            http_request_target("GET /auth/callback?code=a&state=b HTTP/1.1\r\nHost: x\r\n\r\n")
                .unwrap();
        assert_eq!(t, "/auth/callback?code=a&state=b");
    }

    #[test]
    fn bind_addr_from_redirect_ok() {
        assert_eq!(
            bind_addr_from_redirect("http://127.0.0.1:1455/auth/callback").unwrap(),
            "127.0.0.1:1455"
        );
    }

    #[test]
    fn wait_for_oauth_callback_accepts_one_get() {
        // Ephemeral port to avoid clashing with a live agent.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let addr_s = addr.to_string();
        let addr_client = addr;
        let handle =
            thread::spawn(move || wait_for_oauth_callback(&addr_s, Duration::from_secs(5)));
        thread::sleep(Duration::from_millis(80));
        let mut s = TcpStream::connect_timeout(&addr_client, Duration::from_secs(2)).unwrap();
        let req = "GET /auth/callback?code=testcode&state=teststate HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        let _ = s.read_to_string(&mut resp);
        assert!(resp.contains("200"), "{resp}");
        let (code, state) = handle.join().unwrap().unwrap();
        assert_eq!(code, "testcode");
        assert_eq!(state, "teststate");
    }

    #[test]
    fn wait_times_out_when_no_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        let err = wait_for_oauth_callback(&addr, Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }
}
