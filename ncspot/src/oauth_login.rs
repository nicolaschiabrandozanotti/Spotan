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

/// `Some(code)` only for `GET <path>?code=..&state=<expected_state>`.
fn parse_callback(request_line: &str, expected_state: &str) -> Option<String> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let url = Url::parse(&format!("http://localhost{}", parts.next()?)).ok()?;
    let (mut code, mut state_ok) = (None, false);
    for (k, v) in url.query_pairs() {
        match &*k {
            "code" => code = Some(v.into_owned()),
            "state" => state_ok = v == expected_state,
            _ => {}
        }
    }
    code.filter(|_| state_ok)
}

fn token_request(form: &[(&str, &str)]) -> Result<LoginTokens, String> {
    let resp = reqwest::blocking::Client::new()
        .post(TOKEN_URL)
        .form(form)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("token endpoint returned {}", resp.status()));
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
    scopes: &[&str],
    on_url: &dyn Fn(&str),
    timeout: Duration,
) -> Result<LoginTokens, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
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
        let code = parse_callback(&line, &state);
        let body = if code.is_some() {
            "Listo, podés volver a Spoti"
        } else {
            "Solicitud inválida"
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        if let Some(code) = code {
            break code;
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
        assert_eq!(p("GET /ncspot_login?code=XYZ&state=ST HTTP/1.1"), Some("XYZ".into()));
        assert_eq!(p("GET /ncspot_login?code=XYZ&state=OTRO HTTP/1.1"), None);
        assert_eq!(p("GET /ncspot_login?state=ST HTTP/1.1"), None);
        assert_eq!(p("GET /favicon.ico HTTP/1.1"), None);
    }

    #[test]
    fn oauth_listener_times_out() {
        let t = Instant::now();
        let r = oauth_login("id", "/login", &["x"], &|_| {}, Duration::from_millis(200));
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
