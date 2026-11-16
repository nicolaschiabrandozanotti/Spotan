use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use librespot_core::authentication::Credentials as RespotCredentials;
use librespot_core::cache::Cache;
use log::{error, info, warn};

use crate::oauth_login::{self, LoginTokens};

use crate::config;

pub const SPOTIFY_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

// This is a client ID issued specifically for ncspot tied to my personal Spotify account.
// Please do not use it without my permission.
pub const NCSPOT_CLIENT_ID: &str = "d420a117a32841c2b3474932e49fb54b";

static OAUTH_SCOPES: &[&str] = &[
    "playlist-modify",
    "playlist-modify-private",
    "playlist-modify-public",
    "playlist-read",
    "playlist-read-collaborative",
    "playlist-read-private",
    "streaming",
    "user-follow-modify",
    "user-follow-read",
    "user-library-modify",
    "user-library-read",
    "user-modify",
    "user-modify-playback-state",
    "user-modify-private",
    "user-personalized",
    "user-read-currently-playing",
    "user-read-email",
    "user-read-play-history",
    "user-read-playback-position",
    "user-read-playback-state",
    "user-read-private",
    "user-read-recently-played",
    "user-top-read",
];

static NCSPOT_OAUTH_SCOPES: &[&str] = &[
    "streaming",
    "user-read-email",
    "user-read-private",
    "user-library-read",
    "user-library-modify",
    "user-read-playback-state",
    "user-modify-playback-state",
    "playlist-read-private",
    "playlist-modify-public",
    "playlist-modify-private",
    "user-follow-read",
    "user-follow-modify",
    "user-top-read",
    "user-read-currently-playing",
    "user-read-recently-played",
];

/// Cached librespot credentials, or `None` when the start screen must ask for a login.
/// Never prints or opens a browser.
pub fn get_credentials() -> Result<Option<RespotCredentials>, String> {
    let cache = Cache::new(Some(config::cache_path("librespot")), None, None, None)
        .map_err(|e| format!("Could not create librespot cache: {e}"))?;
    let cached = cache.credentials();
    if cached.is_some() {
        info!("Using cached credentials");
    }
    Ok(cached)
}

/// Starts the OAuth2 login and reports the login URL through `on_url` (blocking).
pub fn create_credentials_with(on_url: &dyn Fn(&str)) -> Result<RespotCredentials, String> {
    crate::oauth_login::oauth_login(
        SPOTIFY_CLIENT_ID,
        "/login",
        OAUTH_SCOPES,
        on_url,
        crate::oauth_login::LOGIN_TIMEOUT,
    )
    .map(|t| RespotCredentials::with_access_token(t.access_token))
}

const TOKEN_FILE: &str = "rspotify_token.json";
const TOKEN_ID_FILE: &str = "rspotify_client_id";
const EXPIRED_MARK: &str = "rspotify_expired";

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ApiSession {
    None,
    Active,
    Expired,
}

/// Custom Web API client ID chosen by the user; `None` means the shared one.
pub fn api_client_id() -> Option<String> {
    let id = fs::read_to_string(config::config_path("api_client_id")).ok()?;
    Some(id.trim().to_string()).filter(|s| !s.is_empty())
}

/// Search results per page: a custom app is limited to 10, the shared one allows 50.
pub fn search_page_size() -> u32 {
    if api_client_id().is_some() { 10 } else { 50 }
}

pub fn effective_client_id() -> String {
    api_client_id().unwrap_or_else(|| NCSPOT_CLIENT_ID.to_string())
}

/// Client ID the stored token was issued for (absent file = shared, legacy tokens).
fn token_client_id() -> String {
    fs::read_to_string(config::cache_path(TOKEN_ID_FILE))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| NCSPOT_CLIENT_ID.to_string())
}

/// Bumped whenever the stored Web API token changes outside the normal refresh, so a
/// running app drops the token it holds in memory.
static API_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn api_generation() -> u64 {
    API_GENERATION.load(Ordering::SeqCst)
}

fn bump_generation() {
    API_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Whether the in-memory token must be reloaded from disk.
pub fn needs_reload(loaded_generation: u64, current_generation: u64, secs_left: i64) -> bool {
    loaded_generation != current_generation || secs_left <= 60 * 5
}

fn clear_token() {
    let _ = fs::remove_file(config::cache_path(TOKEN_FILE));
    let _ = fs::remove_file(config::cache_path(TOKEN_ID_FILE));
    bump_generation();
}

/// Set (or clear with `None`) the custom client ID. A different ID invalidates the token.
pub fn set_api_client_id(id: Option<&str>) -> Result<(), String> {
    let path = config::config_path("api_client_id");
    match id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => fs::write(&path, id).map_err(|e| e.to_string())?,
        None => match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        },
    }
    if token_client_id() != effective_client_id() {
        clear_token();
        let _ = fs::remove_file(config::cache_path(EXPIRED_MARK));
    }
    Ok(())
}

pub fn api_session() -> ApiSession {
    let has_token = fs::read_to_string(config::cache_path(TOKEN_FILE))
        .ok()
        .and_then(|j| serde_json::from_str::<rspotify::Token>(&j).ok())
        .is_some_and(|t| !t.is_expired() || t.refresh_token.as_deref().is_some_and(|r| !r.is_empty()));
    if has_token {
        ApiSession::Active
    } else if config::cache_path(EXPIRED_MARK).exists() {
        ApiSession::Expired
    } else {
        ApiSession::None
    }
}

/// The only interactive path to the Web API token. Blocking.
pub fn api_login(on_url: &dyn Fn(&str)) -> Result<(), String> {
    let id = effective_client_id();
    let tokens = oauth_login::oauth_login(
        &id,
        "/ncspot_login",
        NCSPOT_OAUTH_SCOPES,
        on_url,
        oauth_login::LOGIN_TIMEOUT,
    )?;
    fs::write(config::cache_path(TOKEN_ID_FILE), &id).map_err(|e| e.to_string())?;
    write_token(&config::cache_path(TOKEN_FILE), &to_rspotify(tokens, None))?;
    let _ = fs::remove_file(config::cache_path(EXPIRED_MARK));
    bump_generation();
    Ok(())
}

pub fn api_logout() {
    clear_token(); // also bumps the generation
    let _ = fs::remove_file(config::cache_path(EXPIRED_MARK));
}

/// Stored Web API token, refreshed if needed. Never starts a login.
pub fn get_rspotify_token() -> Result<rspotify::Token, String> {
    get_rspotify_token_with(&oauth_login::refresh)
}

fn get_rspotify_token_with(
    refresh: &dyn Fn(&str, &str) -> Result<LoginTokens, String>,
) -> Result<rspotify::Token, String> {
    let path = config::cache_path(TOKEN_FILE);
    let token = fs::read_to_string(&path)
        .ok()
        .and_then(|j| serde_json::from_str::<rspotify::Token>(&j).ok())
        .ok_or_else(|| "No Web API session; log in from settings".to_string())?;

    if !token.is_expired() {
        return Ok(token);
    }

    let id = token_client_id();
    if id != effective_client_id() {
        clear_token();
        return Err("Web API client ID changed; log in again".to_string());
    }

    // Spotify's refresh responses may omit the refresh token, in which case it must be
    // reused, so an empty/missing value here means we have nothing to refresh with.
    let old = token.refresh_token.as_deref().filter(|s| !s.is_empty());
    let result = match old {
        Some(old) => {
            info!("Access token expired, attempting to refresh..");
            refresh(&id, old).map(|t| to_rspotify(t, Some(old)))
        }
        None => Err("no refresh token".to_string()),
    };
    match result {
        Ok(new) => {
            write_token(&path, &new)?;
            Ok(new)
        }
        Err(e) if e.starts_with("token endpoint returned 4") || old.is_none() => {
            error!("Failed to refresh token: {e}");
            clear_token();
            let _ = fs::write(config::cache_path(EXPIRED_MARK), "");
            Err(format!("Web API session expired: {e}"))
        }
        // Network errors and 5xx are transient: keep the token and retry later.
        Err(e) => {
            error!("Failed to refresh token (will retry): {e}");
            Err(format!("Could not refresh Web API token: {e}"))
        }
    }
}

/// Write `token` to `path`, logging (rather than silently discarding) any failure, and
/// restricting file permissions since the file holds a long-lived credential.
fn write_token(path: &Path, token: &rspotify::Token) -> Result<(), String> {
    let json = match serde_json::to_string_pretty(token) {
        Ok(json) => json,
        Err(e) => {
            error!("Failed to serialize rspotify token: {e}");
            return Err(e.to_string());
        }
    };

    if let Err(e) = fs::write(path, json) {
        error!("Failed to write rspotify token cache: {e}");
        return Err(e.to_string());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            warn!("Failed to restrict permissions on rspotify token cache: {e}");
        }
    }
    Ok(())
}

/// Convert login tokens into the `rspotify::Token` used for Web API calls. When the
/// response omits the refresh token, `fallback_refresh_token` (still valid) is kept.
fn to_rspotify(tokens: LoginTokens, fallback_refresh_token: Option<&str>) -> rspotify::Token {
    let expires_in =
        chrono::Duration::from_std(tokens.expires_in).unwrap_or(chrono::Duration::seconds(0));
    rspotify::Token {
        access_token: tokens.access_token,
        expires_in,
        scopes: tokens.scopes.into_iter().collect(),
        expires_at: Some(chrono::Utc::now() + expires_in),
        refresh_token: tokens
            .refresh_token
            .filter(|s| !s.is_empty())
            .or_else(|| fallback_refresh_token.map(str::to_string)),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::oauth_login::LoginTokens;
    use std::sync::Mutex;
    use std::time::Duration;

    // The configuration base path is global, so tests touching it run one at a time.
    static LOCK: Mutex<()> = Mutex::new(());

    fn sandbox() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        config::set_configuration_base_path(Some(dir.path().to_path_buf()));
        (guard, dir)
    }

    fn tokens(refresh_token: &str) -> LoginTokens {
        LoginTokens {
            access_token: "access".to_string(),
            refresh_token: Some(refresh_token.to_string()).filter(|s| !s.is_empty()),
            expires_in: Duration::from_secs(3600),
            scopes: Vec::new(),
        }
    }

    fn store_token(expires_in_secs: i64, refresh: Option<&str>) {
        let token = rspotify::Token {
            access_token: "stored".to_string(),
            expires_in: chrono::Duration::seconds(expires_in_secs),
            scopes: Default::default(),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(expires_in_secs)),
            refresh_token: refresh.map(str::to_string),
        };
        write_token(&config::cache_path("rspotify_token.json"), &token).unwrap();
    }

    fn no_refresh(_: &str, _: &str) -> Result<LoginTokens, String> {
        panic!("refresh must not be called");
    }

    #[test]
    fn missing_credentials_means_start_screen() {
        let (_g, _dir) = sandbox();
        assert!(get_credentials().unwrap().is_none());
    }

    #[test]
    fn search_page_size_depends_on_own_app() {
        let (_g, _d) = sandbox();
        assert_eq!(search_page_size(), 50);
        fs::write(config::config_path("api_client_id"), "abc").unwrap();
        assert_eq!(search_page_size(), 10);
    }

    #[test]
    fn client_id_is_trimmed_and_empty_means_shared() {
        let (_g, _d) = sandbox();
        assert_eq!(effective_client_id(), NCSPOT_CLIENT_ID);
        fs::write(config::config_path("api_client_id"), "  abc123\n").unwrap();
        assert_eq!(api_client_id(), Some("abc123".to_string()));
        assert_eq!(effective_client_id(), "abc123");
        fs::write(config::config_path("api_client_id"), "   \n").unwrap();
        assert_eq!(api_client_id(), None);
    }

    #[test]
    fn legacy_token_without_id_file_counts_as_shared() {
        let (_g, _d) = sandbox();
        store_token(3600, Some("r"));
        assert!(get_rspotify_token_with(&no_refresh).is_ok());
        assert_eq!(api_session(), ApiSession::Active);
    }

    #[test]
    fn changing_client_id_discards_token() {
        let (_g, _d) = sandbox();
        store_token(3600, Some("r"));
        fs::write(config::cache_path("rspotify_client_id"), "A").unwrap();
        set_api_client_id(Some("B")).unwrap();
        assert_eq!(api_session(), ApiSession::None);
        assert!(!config::cache_path("rspotify_client_id").exists());
    }

    #[test]
    fn network_error_keeps_token() {
        let (_g, _d) = sandbox();
        store_token(-10, Some("r"));
        let failing = |_: &str, _: &str| Err("error sending request".to_string());
        assert!(get_rspotify_token_with(&failing).is_err());
        assert!(config::cache_path("rspotify_token.json").exists());
        assert_eq!(api_session(), ApiSession::Active);
        let server = |_: &str, _: &str| Err("token endpoint returned 503: x".to_string());
        assert!(get_rspotify_token_with(&server).is_err());
        assert_eq!(api_session(), ApiSession::Active);
    }

    #[test]
    fn reload_decision_uses_generation() {
        assert!(!needs_reload(3, 3, 3000));
        assert!(needs_reload(3, 4, 3000));
        assert!(needs_reload(3, 3, 100));
    }

    #[test]
    fn generation_bumps_on_logout_and_id_change() {
        let (_g, _d) = sandbox();
        let g = api_generation();
        api_logout();
        assert!(api_generation() > g);
    }

    #[test]
    fn expired_token_without_refresh_is_not_active() {
        let (_g, _d) = sandbox();
        store_token(-10, None);
        assert_eq!(api_session(), ApiSession::None);
    }

    #[test]
    fn failed_refresh_clears_token_and_marks_expired() {
        let (_g, _d) = sandbox();
        store_token(-10, Some("r"));
        let failing = |_: &str, _: &str| Err("token endpoint returned 400: boom".to_string());
        assert!(get_rspotify_token_with(&failing).is_err());
        assert!(!config::cache_path("rspotify_token.json").exists());
        assert_eq!(api_session(), ApiSession::Expired);
        api_logout();
        assert_eq!(api_session(), ApiSession::None);
    }

    #[test]
    fn successful_refresh_keeps_old_refresh_token_and_rewrites() {
        let (_g, _d) = sandbox();
        store_token(-10, Some("old"));
        let ok = |_: &str, rt: &str| {
            assert_eq!(rt, "old");
            Ok(tokens(""))
        };
        let t = get_rspotify_token_with(&ok).unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("old"));
        assert_eq!(api_session(), ApiSession::Active);
    }

    #[test]
    fn expired_token_with_other_client_id_is_discarded() {
        let (_g, _d) = sandbox();
        store_token(-10, Some("r"));
        fs::write(config::cache_path("rspotify_client_id"), "A").unwrap();
        assert!(get_rspotify_token_with(&no_refresh).is_err());
        assert_eq!(api_session(), ApiSession::None);
    }

    #[test]
    fn no_token_never_logs_in() {
        let (_g, _d) = sandbox();
        assert!(get_rspotify_token_with(&no_refresh).is_err());
        assert_eq!(api_session(), ApiSession::None);
    }

    #[test]
    fn to_rspotify_keeps_new_refresh_token_when_present() {
        let mapped = to_rspotify(tokens("new"), Some("old"));
        assert_eq!(mapped.refresh_token, Some("new".to_string()));
    }

    #[test]
    fn to_rspotify_falls_back_to_previous_refresh_token_when_omitted() {
        let mapped = to_rspotify(tokens(""), Some("old"));
        assert_eq!(mapped.refresh_token, Some("old".to_string()));
        assert_eq!(to_rspotify(tokens(""), None).refresh_token, None);
    }
}
