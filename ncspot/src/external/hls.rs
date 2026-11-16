//! Lector mínimo de listas m3u8 (HLS fMP4: `init.mp4` + segmentos `.m4s`).

pub struct Segment {
    pub duration_ms: u32,
    pub url: String,
}

pub struct Hls {
    pub init: String,
    pub segments: Vec<Segment>,
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
}

pub fn parse_m3u8(text: &str) -> Result<Hls, String> {
    let (mut init, mut segments, mut pending) = (None, Vec::new(), None);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
            let rest = attrs.split("URI=\"").nth(1).ok_or("EXT-X-MAP sin URI")?;
            init = Some(unescape(rest.split('"').next().unwrap_or_default()));
        } else if let Some(inf) = line.strip_prefix("#EXTINF:") {
            let secs: f64 = inf.split(',').next().unwrap_or_default().parse().map_err(|_| format!("EXTINF inválido: {inf}"))?;
            pending = Some((secs * 1000.0) as u32);
        } else if !line.starts_with('#') {
            let duration_ms = pending.take().ok_or("segmento sin EXTINF")?;
            segments.push(Segment { duration_ms, url: unescape(line) });
        }
    }
    let init = init.ok_or("lista sin EXT-X-MAP (no es fMP4)")?;
    if segments.is_empty() {
        return Err("lista sin segmentos".into());
    }
    Ok(Hls { init, segments })
}

impl Hls {
    /// (índice del segmento, inicio en ms); pasado el final, el último segmento.
    pub fn segment_at(&self, position_ms: u32) -> (usize, u32) {
        let mut start = 0;
        for (i, s) in self.segments.iter().enumerate() {
            if position_ms < start + s.duration_ms || i + 1 == self.segments.len() {
                return (i, start);
            }
            start += s.duration_ms;
        }
        (0, 0)
    }

    pub fn duration_ms(&self) -> u32 {
        self.segments.iter().map(|s| s.duration_ms).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M3U8: &str = include_str!("fixtures/sc.m3u8");

    #[test]
    fn parse_m3u8_unescapes_and_sums() {
        let h = parse_m3u8(M3U8).unwrap();
        assert_eq!(h.segments.len(), 3);
        assert!(!h.init.contains("&amp;") && h.init.contains("init.mp4?expires=1&Policy=P"));
        assert!(h.segments.iter().all(|s| !s.url.contains("&amp;")));
        assert!(h.duration_ms().abs_diff(29999) <= 1);
    }

    #[test]
    fn segment_at_finds_segment() {
        let h = parse_m3u8(M3U8).unwrap();
        assert_eq!(h.segment_at(0), (0, 0));
        assert_eq!(h.segment_at(15000), (1, 10007));
        assert_eq!(h.segment_at(999_999), (2, 20014));
    }

    #[test]
    fn parse_m3u8_requires_map() {
        assert!(parse_m3u8("#EXTM3U\n#EXTINF:1.0,\nhttp://x/a.m4s\n").is_err());
    }
}
