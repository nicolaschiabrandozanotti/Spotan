// usado por las tareas siguientes (buscar/reproducir YouTube y SoundCloud)
#![allow(dead_code)]

//! Servicios externos (YouTube, SoundCloud): lista de los prendidos y utilidades comunes.

use std::fs;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use crate::config;
use crate::model::track::{Source, Track};

pub mod hls;
pub mod player;
pub mod sc_library;
pub mod soundcloud;
pub mod stream;
pub mod yt_library;
pub mod youtube;

/// Orden fijo de los servicios externos.
const ORDER: [Source; 2] = [Source::YouTube, Source::SoundCloud];

/// Una página de resultados y, si hay más, el token para pedir la siguiente.
pub struct Page {
    pub tracks: Vec<Track>,
    pub next: Option<String>,
}

pub fn parse_services(text: &str) -> Vec<Source> {
    let found: Vec<Source> = text.lines().filter_map(|l| Source::from_key(l.trim())).collect();
    ORDER.into_iter().filter(|s| found.contains(s)).collect()
}

pub fn format_services(list: &[Source]) -> String {
    ORDER
        .iter()
        .filter(|s| list.contains(s))
        .map(|s| format!("{}\n", s.key()))
        .collect()
}

static ENABLED: OnceLock<RwLock<Vec<Source>>> = OnceLock::new();

fn cache() -> &'static RwLock<Vec<Source>> {
    ENABLED.get_or_init(|| {
        let text = fs::read_to_string(config::config_path("services")).unwrap_or_default();
        RwLock::new(parse_services(&text))
    })
}

/// Servicios externos prendidos (sin Spotify). Se lee del archivo una sola vez.
pub fn enabled() -> Vec<Source> {
    cache().read().unwrap().clone()
}

pub fn set_enabled(source: Source, on: bool) -> Result<(), String> {
    let mut list = cache().write().unwrap();
    let mut new = list.clone();
    new.retain(|s| *s != source);
    if on {
        new.push(source);
    }
    let new = parse_services(&format_services(&new));
    let path = config::config_path("services");
    if new.is_empty() {
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    } else {
        fs::write(&path, format_services(&new)).map_err(|e| e.to_string())?;
    }
    *list = new;
    Ok(())
}

pub fn http() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            // youtube.com manda ~19 KB de encabezados; hyper corta en 16 KB por defecto con HTTP/2
            .http2_max_header_list_size(256 * 1024)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP")
    })
}

/// "6:00" -> 360000, "1:02:03" -> 3723000 (milisegundos).
pub fn parse_duration(text: &str) -> Option<u32> {
    let mut secs: u32 = 0;
    let mut parts = 0;
    for p in text.trim().split(':') {
        secs = secs.checked_mul(60)?.checked_add(p.parse().ok()?)?;
        parts += 1;
    }
    if parts > 3 {
        return None;
    }
    secs.checked_mul(1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_services_ignores_junk_and_duplicates() {
        let got = parse_services("youtube\n\nfoo\nyoutube\nsoundcloud\nspotify");
        assert_eq!(got, vec![Source::YouTube, Source::SoundCloud]);
    }

    #[test]
    fn format_parse_roundtrip() {
        let l = vec![Source::YouTube, Source::SoundCloud];
        assert_eq!(parse_services(&format_services(&l)), l);
        assert_eq!(format_services(&[]), "");
    }

    #[test]
    fn parse_duration_cases() {
        assert_eq!(parse_duration("6:00"), Some(360000));
        assert_eq!(parse_duration("1:02:03"), Some(3723000));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration(""), None);
    }
}
