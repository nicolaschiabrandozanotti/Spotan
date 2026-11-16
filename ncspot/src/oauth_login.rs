//! OAuth2 PKCE login that reports the authorization URL through a callback instead of
//! printing it (librespot-oauth writes to stdout, which corrupts the TUI).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use oauth2::{CsrfToken, PkceCodeChallenge};
use serde::Deserialize;
use url::Url;

pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

pub struct LoginTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Duration,
    pub scopes: Vec<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

fn auth_url(client_id: &str, redirect: &str, scopes: &[&str], challenge: &str, state: &str) -> String {
    let mut url = Url::parse("https://accounts.spotify.com/authorize").expect("static url");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge)
        .append_pair("state", state)
        .append_pair("scope", &scopes.join(" "));
    // form-urlencoding writes spaces as '+'; Spotify expects %20.
    url.as_str().replace('+', "%20")
}

#[derive(Debug, PartialEq)]
enum Callback {
    Code(String),
    Denied,
    Ignore,
}

/// `Code` only for `GET <path>?code=..&state=<expected_state>`; `Denied` for the same
/// state with `error=` (user cancelled).
fn parse_callback(request_line: &str, expected_state: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Ignore;
    };
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return Callback::Ignore;
    };
    let (mut code, mut error, mut state_ok) = (None, false, false);
    for (k, v) in url.query_pairs() {
        match &*k {
            "code" => code = Some(v.into_owned()),
            "error" => error = true,
            "state" => state_ok = v == expected_state,
            _ => {}
        }
    }
    match (state_ok, code, error) {
        (true, Some(c), _) => Callback::Code(c),
        (true, None, true) => Callback::Denied,
        _ => Callback::Ignore,
    }
}

fn token_request(form: &[(&str, &str)]) -> Result<LoginTokens, String> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(TOKEN_URL)
        .form(form)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("token endpoint returned {status}: {body}"));
    }
    let t: TokenResponse = resp.json().map_err(|e| e.to_string())?;
    Ok(LoginTokens {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        expires_in: Duration::from_secs(t.expires_in),
        scopes: t
            .scope
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
    })
}

pub fn refresh(client_id: &str, refresh_token: &str) -> Result<LoginTokens, String> {
    token_request(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])
}

/// Blocking. Waits for the browser callback until `timeout`.
pub fn oauth_login(
    client_id: &str,
    path: &str,
    port: u16,
    scopes: &[&str],
    on_url: &dyn Fn(&str),
    timeout: Duration,
) -> Result<LoginTokens, String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("no se pudo abrir el puerto {port}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}{path}");

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let state = CsrfToken::new_random().secret().to_string();
    let url = auth_url(client_id, &redirect, scopes, challenge.as_str(), &state);
    on_url(&url);
    #[cfg(not(test))]
    open::that_in_background(url);

    let deadline = Instant::now() + timeout;
    let code = loop {
        if Instant::now() >= deadline {
            return Err("login timed out".to_string());
        }
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        // Accepted sockets may inherit non-blocking mode; read with a short timeout instead.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let cb = parse_callback(&line, &state);
        let body = match cb {
            Callback::Code(_) => "Listo, podés volver a Spoti",
            Callback::Denied => "Inicio de sesión cancelado, podés volver a Spoti",
            Callback::Ignore => "Solicitud inválida",
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        match cb {
            Callback::Code(code) => break code,
            Callback::Denied => return Err("Inicio de sesión cancelado".to_string()),
            Callback::Ignore => {}
        }
    };

    token_request(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("client_id", client_id),
        ("code_verifier", verifier.secret()),
    ])
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn auth_url_has_pkce_and_redirect() {
        let u = auth_url(
            "abc",
            "http://127.0.0.1:4381/ncspot_login",
            &["a", "b"],
            "CH",
            "ST",
        );
        for s in [
            "client_id=abc",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A4381%2Fncspot_login",
            "code_challenge=CH",
            "code_challenge_method=S256",
            "state=ST",
            "response_type=code",
            "scope=a%20b",
        ] {
            assert!(u.contains(s), "{u} missing {s}");
        }
    }

    #[test]
    fn parse_callback_rejects_wrong_state_and_missing_code() {
        let p = |l| parse_callback(l, "ST");
        assert_eq!(p("GET /ncspot_login?code=XYZ&state=ST HTTP/1.1"), Callback::Code("XYZ".into()));
        assert_eq!(p("GET /ncspot_login?code=XYZ&state=OTRO HTTP/1.1"), Callback::Ignore);
        assert_eq!(p("GET /ncspot_login?state=ST HTTP/1.1"), Callback::Ignore);
        assert_eq!(p("GET /favicon.ico HTTP/1.1"), Callback::Ignore);
        assert_eq!(p("GET /ncspot_login?error=access_denied&state=ST HTTP/1.1"), Callback::Denied);
        assert_eq!(p("GET /ncspot_login?error=access_denied&state=OTRO HTTP/1.1"), Callback::Ignore);
    }

    #[test]
    fn oauth_listener_times_out() {
        let t = Instant::now();
        let r = oauth_login("id", "/login", 0, &["x"], &|_| {}, Duration::from_millis(200));
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
