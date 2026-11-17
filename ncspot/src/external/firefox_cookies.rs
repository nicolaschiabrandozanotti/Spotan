//! Lee la sesión de YouTube de Firefox (solo las cookies de sesión de youtube.com).
//! Nunca se muestran ni se loguean los valores de las cookies.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rusqlite::{Connection, OpenFlags};

/// Cookies de sesión que se envían (con todas YouTube responde HTTP 413).
/// Además valen las que empiezan con `__Secure-1PSID` o `__Secure-3PSID`.
pub const SESSION_COOKIES: &[&str] = &[
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "LOGIN_INFO",
    "SIDCC",
    "PREF",
    "VISITOR_INFO1_LIVE",
    "YSC",
];

pub fn is_session_cookie(name: &str) -> bool {
    SESSION_COOKIES.contains(&name)
        || name.starts_with("__Secure-1PSID")
        || name.starts_with("__Secure-3PSID")
}

#[derive(Debug, PartialEq)]
pub enum FfError {
    NoFirefox,
    NoSession,
    Locked,
    Other(String),
}

impl fmt::Display for FfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfError::NoFirefox | FfError::NoSession => f.write_str(
                "No encontré una sesión de YouTube en Firefox. Iniciá sesión en music.youtube.com con Firefox o usá Pegar.",
            ),
            FfError::Locked => f.write_str("No pude leer Firefox: cerralo y probá de nuevo, o usá Pegar"),
            FfError::Other(e) => f.write_str(e),
        }
    }
}

pub struct SessionCookies {
    pub header: String,
    pub last_access: i64,
}

impl fmt::Debug for SessionCookies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionCookies")
            .field("cookies", &self.header.split("; ").filter(|c| !c.is_empty()).count())
            .field("last_access", &self.last_access)
            .finish()
    }
}

pub fn profile_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(|a| vec![PathBuf::from(a).join("Mozilla").join("Firefox")])
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
        vec![
            home.join(".mozilla/firefox"),
            config.join("mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/config/mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ]
    }
}

/// Perfiles listados en `profiles.ini` (`Path=` relativo salvo `IsRelative=0`).
pub fn profiles(root: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut section: Vec<(&str, &str)> = Vec::new();
    let mut flush = |section: &mut Vec<(&str, &str)>| {
        let get = |k: &str| section.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        if let Some(path) = get("Path") {
            out.push(if get("IsRelative") == Some("0") {
                PathBuf::from(path)
            } else {
                root.join(path)
            });
        }
        section.clear();
    };
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            flush(&mut section);
        } else if let Some((k, v)) = line.split_once('=') {
            section.push((k.trim(), v.trim()));
        }
    }
    flush(&mut section);
    out
}

fn locked(e: &std::io::Error) -> bool {
    // 32 = ERROR_SHARING_VIOLATION en Windows
    e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(32)
}

/// Directorio temporal propio (modo 0700, siempre nuevo); se borra al soltarlo.
struct TempDir(PathBuf);

impl TempDir {
    fn new(base: &Path) -> std::io::Result<Self> {
        sweep_stale(base);
        static N: AtomicU32 = AtomicU32::new(0);
        let mut last = None;
        for _ in 0..8 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let name = format!("spotan-ff-{}-{nanos:x}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
            match Self::create_at(base.join(name)) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                r => return r,
            }
        }
        Err(last.unwrap())
    }

    /// Falla si la ruta ya existe (no reutiliza directorios ni symlinks ajenos).
    fn create_at(dir: PathBuf) -> std::io::Result<Self> {
        let mut b = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&dir)?;
        Ok(Self(dir))
    }
}

/// Borra (sin fallar) copias `spotan-ff-*` propias de más de una hora que quedaron de otra ejecución.
fn sweep_stale(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else { return };
    let limit = std::time::Duration::from_secs(3600);
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with("spotan-ff-") {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        #[cfg(unix)]
        if std::os::unix::fs::MetadataExt::uid(&m) != unsafe { libc::geteuid() } {
            continue;
        }
        let old = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
        if m.is_dir() && old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("No se pudo borrar la copia temporal de cookies de Firefox ({:?})", e.kind());
            }
        }
    }
}

pub fn read_profile(profile: &Path) -> Result<Option<SessionCookies>, FfError> {
    read_profile_in(profile, &std::env::temp_dir())
}

fn read_profile_in(profile: &Path, tmp_base: &Path) -> Result<Option<SessionCookies>, FfError> {
    let src = profile.join("cookies.sqlite");
    if !src.exists() {
        return Ok(None);
    }
    let tmp = TempDir::new(tmp_base).map_err(|e| if locked(&e) { FfError::Locked } else { FfError::Other(e.to_string()) })?;
    for suffix in ["", "-wal"] {
        let from = profile.join(format!("cookies.sqlite{suffix}"));
        if suffix.is_empty() || from.exists() {
            fs::copy(&from, tmp.0.join(format!("cookies.sqlite{suffix}")))
                .map_err(|e| if locked(&e) { FfError::Locked } else { FfError::Other(e.to_string()) })?;
        }
    }
    let other = |e: rusqlite::Error| FfError::Other(e.to_string());
    let conn = Connection::open_with_flags(tmp.0.join("cookies.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(other)?;
    // Filtro por nombre en SQL (literales fijos): una fila ajena con value raro no debe abortar la lectura.
    let names = SESSION_COOKIES.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", ");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name, value, lastAccessed FROM moz_cookies \
             WHERE (host = 'youtube.com' OR host LIKE '%.youtube.com') AND originAttributes = '' \
             AND (name IN ({names}) OR name LIKE '\\_\\_Secure-1PSID%' ESCAPE '\\' \
                  OR name LIKE '\\_\\_Secure-3PSID%' ESCAPE '\\') \
             ORDER BY name"
        ))
        .map_err(other)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1).ok().flatten(), r.get::<_, i64>(2)?)))
        .map_err(other)?;
    let mut parts = Vec::new();
    let mut last_access = None;
    for row in rows {
        let (name, value, accessed) = row.map_err(other)?;
        let Some(value) = value.filter(|_| is_session_cookie(&name)) else {
            continue;
        };
        if name == "SAPISID" {
            last_access = Some(accessed);
        }
        parts.push(format!("{name}={value}"));
    }
    Ok(last_access.map(|last_access| SessionCookies { header: parts.join("; "), last_access }))
}

pub fn find_session() -> Result<(PathBuf, SessionCookies), FfError> {
    find_session_in(&profile_roots())
}

pub fn find_session_in(roots: &[PathBuf]) -> Result<(PathBuf, SessionCookies), FfError> {
    let mut best: Option<(PathBuf, SessionCookies)> = None;
    let mut any_root = false;
    let mut err = None;
    for root in roots.iter().filter(|r| r.join("profiles.ini").exists()) {
        any_root = true;
        for profile in profiles(root) {
            match read_profile(&profile) {
                Ok(Some(s)) if best.as_ref().is_none_or(|(_, b)| s.last_access > b.last_access) => {
                    best = Some((profile, s));
                }
                Err(e) => err = Some(e),
                _ => {}
            }
        }
    }
    match (best, any_root, err) {
        (Some(b), _, _) => Ok(b),
        (None, false, _) => Err(FfError::NoFirefox),
        (None, true, Some(e)) => Err(e),
        (None, true, None) => Err(FfError::NoSession),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crea `<root>/<name>` con cookies.sqlite: filas (host, name, value, originAttributes, lastAccessed).
    fn fake_profile(root: &Path, name: &str, rows: &[(&str, &str, &str, &str, i64)]) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("cookies.sqlite")).unwrap();
        conn.execute(
            "CREATE TABLE moz_cookies(name TEXT, value TEXT, host TEXT, originAttributes TEXT, lastAccessed INTEGER)",
            [],
        )
        .unwrap();
        for (host, n, v, oa, la) in rows {
            conn.execute("INSERT INTO moz_cookies VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![n, v, host, oa, la])
                .unwrap();
        }
        let ini = format!("[Profile0]\nName=x\nIsRelative=1\nPath={name}\n");
        let prev = fs::read_to_string(root.join("profiles.ini")).unwrap_or_default();
        fs::write(root.join("profiles.ini"), prev + &ini).unwrap();
        dir
    }

    #[test]
    fn reads_only_session_cookies_of_youtube() {
        let root = tempfile::tempdir().unwrap();
        let p = fake_profile(
            root.path(),
            "a.default",
            &[
                (".youtube.com", "SID", "v1", "", 1),
                (".youtube.com", "SAPISID", "v2", "", 7),
                (".youtube.com", "ST-abc", "v3", "", 1),
                (".youtube.com", "__Secure-3PSIDTS", "v4", "", 1),
                (".google.com", "HSID", "v5", "", 1),
                ("notyoutube.com", "SAPISID", "v8", "", 9),
                ("youtube.com.evil.com", "SID", "v9", "", 1),
                (".youtube.com", "SSID", "v6", "^userContextId=1", 1),
            ],
        );
        let got = read_profile(&p).unwrap().unwrap();
        assert_eq!(got.last_access, 7);
        let names: Vec<&str> = got.header.split("; ").map(|c| c.split('=').next().unwrap()).collect();
        assert_eq!(names, ["SAPISID", "SID", "__Secure-3PSIDTS"]);
    }

    #[cfg(unix)]
    #[test]
    fn profile_roots_include_xdg_and_flatpak_config() {
        let roots = profile_roots();
        let ends = |s: &str| roots.iter().any(|r| r.ends_with(s));
        assert!(ends("mozilla/firefox") && ends(".var/app/org.mozilla.firefox/config/mozilla/firefox"));
        assert!(roots.iter().any(|r| r.ends_with(".config/mozilla/firefox")) || std::env::var_os("XDG_CONFIG_HOME").is_some());
    }

    #[test]
    fn stale_temp_dirs_are_swept() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("spotan-ff-1-a-0");
        let other = base.path().join("otra-cosa");
        for d in [&old, &other] {
            fs::create_dir(d).unwrap();
        }
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        fs::File::open(&old).unwrap().set_modified(two_hours_ago).unwrap();
        fs::File::open(&other).unwrap().set_modified(two_hours_ago).unwrap();
        let _t = TempDir::new(base.path()).unwrap();
        assert!(!old.exists() && other.exists());
    }

    #[test]
    fn no_sapisid_is_none() {
        let root = tempfile::tempdir().unwrap();
        let p = fake_profile(root.path(), "a", &[(".youtube.com", "SID", "v", "", 1)]);
        assert!(read_profile(&p).unwrap().is_none());
    }

    #[test]
    fn picks_most_recent_profile() {
        let root = tempfile::tempdir().unwrap();
        fake_profile(root.path(), "old", &[(".youtube.com", "SAPISID", "v", "", 5)]);
        let new = fake_profile(root.path(), "new", &[(".youtube.com", "SAPISID", "v", "", 9)]);
        let (path, s) = find_session_in(&[root.path().to_path_buf()]).unwrap();
        assert_eq!((path, s.last_access), (new, 9));
    }

    #[test]
    fn missing_root_is_no_firefox() {
        let root = tempfile::tempdir().unwrap();
        let r = find_session_in(&[root.path().join("nada")]);
        assert_eq!(r.unwrap_err(), FfError::NoFirefox);
    }

    #[test]
    fn root_without_session_is_no_session() {
        let root = tempfile::tempdir().unwrap();
        fake_profile(root.path(), "a", &[]);
        assert_eq!(find_session_in(&[root.path().to_path_buf()]).unwrap_err(), FfError::NoSession);
    }

    #[test]
    fn temp_copy_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let p = fake_profile(root.path(), "a", &[(".youtube.com", "SAPISID", "v", "", 1)]);
        assert!(read_profile_in(&p, tmp.path()).unwrap().is_some());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn debug_hides_cookie_values() {
        let c = SessionCookies { header: "SAPISID=secreto1; SID=secreto2".into(), last_access: 3 };
        let s = format!("{c:?}");
        assert!(!s.contains("secreto") && s.contains('2'));
    }

    #[test]
    fn bad_value_in_non_session_row_does_not_abort() {
        let root = tempfile::tempdir().unwrap();
        let p = fake_profile(root.path(), "a", &[(".youtube.com", "SAPISID", "v", "", 4)]);
        let conn = Connection::open(p.join("cookies.sqlite")).unwrap();
        conn.execute("INSERT INTO moz_cookies VALUES ('otra', NULL, '.youtube.com', '', 1)", []).unwrap();
        conn.execute("INSERT INTO moz_cookies VALUES ('blob', x'00ff', '.youtube.com', '', 1)", []).unwrap();
        drop(conn);
        assert_eq!(read_profile(&p).unwrap().unwrap().last_access, 4);
    }

    #[test]
    fn reads_session_that_lives_only_in_wal() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a");
        fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("cookies.sqlite")).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute(
            "CREATE TABLE moz_cookies(name TEXT, value TEXT, host TEXT, originAttributes TEXT, lastAccessed INTEGER)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO moz_cookies VALUES ('SAPISID', 'v', '.youtube.com', '', 6)", []).unwrap();
        assert!(dir.join("cookies.sqlite-wal").metadata().unwrap().len() > 0);
        assert_eq!(read_profile(&dir).unwrap().unwrap().last_access, 6);
    }

    #[cfg(unix)]
    #[test]
    fn temp_dir_is_private_and_never_reused() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let t = TempDir::new(base.path()).unwrap();
        assert_eq!(fs::metadata(&t.0).unwrap().permissions().mode() & 0o777, 0o700);
        let e = TempDir::create_at(t.0.clone()).err().unwrap();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(t.0.exists(), "el existente no se toca");
    }
}
