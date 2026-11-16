//! Fuentes de bytes (HTTP por rangos, HLS fMP4) y decodificación AAC para YouTube y SoundCloud.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use super::hls::Hls;

const CHUNK: u64 = 1 << 20;

#[derive(Debug)]
pub enum FetchError {
    /// 403 o 410: la URL venció.
    Expired,
    Other(String),
}

type RangeFetch = Box<dyn FnMut(u64, u64) -> Result<Vec<u8>, FetchError> + Send + Sync>;

/// Fuente con búsqueda sobre HTTP por rangos; `fetch(inicio, fin)` con `fin` inclusive.
pub struct RangeSource {
    len: u64,
    pos: u64,
    buf_start: u64,
    buf: Vec<u8>,
    fetch: RangeFetch,
    refresh: Option<Box<dyn FnMut() -> Result<(), String> + Send + Sync>>,
}

impl RangeSource {
    pub fn new(len: u64, fetch: RangeFetch) -> Self {
        Self { len, pos: 0, buf_start: 0, buf: Vec::new(), fetch, refresh: None }
    }

    pub fn with_refresh(mut self, refresh: Box<dyn FnMut() -> Result<(), String> + Send + Sync>) -> Self {
        self.refresh = Some(refresh);
        self
    }

    fn load(&mut self) -> io::Result<()> {
        let end = (self.pos + CHUNK).min(self.len) - 1;
        let mut res = (self.fetch)(self.pos, end);
        if let (Err(FetchError::Expired), Some(refresh)) = (&res, self.refresh.as_mut()) {
            refresh().map_err(io::Error::other)?;
            res = (self.fetch)(self.pos, end);
        }
        self.buf = res.map_err(|e| io::Error::other(format!("{e:?}")))?;
        self.buf_start = self.pos;
        Ok(())
    }
}

impl Read for RangeSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        let off = self.pos.wrapping_sub(self.buf_start);
        if self.pos < self.buf_start || off >= self.buf.len() as u64 {
            self.load()?;
            if self.buf.is_empty() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "rango vacío"));
            }
        }
        let off = (self.pos - self.buf_start) as usize;
        let n = out.len().min(self.buf.len() - off);
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for RangeSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let new = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "posición negativa"))?;
        Ok(self.pos)
    }
}

impl MediaSource for RangeSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Entrega `init` y luego los segmentos desde `from_segment`, bajando uno por vez.
pub struct HlsSource {
    hls: Hls,
    /// `None`: falta el init; `Some(i)`: próximo segmento a bajar.
    next: Option<usize>,
    cur: Cursor<Vec<u8>>,
    fetch: Box<dyn FnMut(&str) -> io::Result<Vec<u8>> + Send + Sync>,
    started: bool,
}

impl HlsSource {
    pub fn new(hls: Hls, from_segment: usize, fetch: Box<dyn FnMut(&str) -> io::Result<Vec<u8>> + Send + Sync>) -> Self {
        Self { hls, next: Some(from_segment), cur: Cursor::new(Vec::new()), fetch, started: false }
    }
}

impl Read for HlsSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.cur.read(out)?;
            if n > 0 || out.is_empty() {
                return Ok(n);
            }
            let bytes = if !self.started {
                self.started = true;
                (self.fetch)(&self.hls.init)?
            } else {
                let Some(i) = self.next else { return Ok(0) };
                let Some(seg) = self.hls.segments.get(i) else { return Ok(0) };
                self.next = Some(i + 1);
                (self.fetch)(&seg.url)?
            };
            self.cur = Cursor::new(bytes);
        }
    }
}

impl Seek for HlsSource {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "HLS no admite búsqueda"))
    }
}

impl MediaSource for HlsSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

pub struct Decoding {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    seekable: bool,
}

fn e(err: SymError) -> String {
    err.to_string()
}

impl Decoding {
    pub fn open(src: Box<dyn MediaSource>) -> Result<Self, String> {
        let seekable = src.is_seekable();
        let mss = MediaSourceStream::new(src, Default::default());
        let mut hint = Hint::new();
        hint.with_extension("mp4");
        let format = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(e)?
            .format;
        let track = format.default_track().ok_or("sin pista de audio")?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(e)?;
        Ok(Self { format, decoder, track_id, seekable })
    }

    /// Siguiente bloque intercalado (muestras, canales, Hz); `None` al terminar.
    pub fn next_samples(&mut self) -> Result<Option<(Vec<f32>, usize, u32)>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(err)) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(err) => return Err(e(err)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    buf.copy_interleaved_ref(decoded);
                    return Ok(Some((buf.samples().to_vec(), spec.channels.count(), spec.rate)));
                }
                Err(SymError::DecodeError(_)) => continue, // paquete dañado: se salta
                Err(err) => return Err(e(err)),
            }
        }
    }

    pub fn seek(&mut self, position_ms: u32) -> Result<(), String> {
        if !self.seekable {
            return Err("la fuente no admite búsqueda".into());
        }
        let time = Time::new(u64::from(position_ms / 1000), f64::from(position_ms % 1000) / 1000.0);
        self.format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(e)?;
        self.decoder.reset();
        Ok(())
    }
}

pub fn to_stereo(samples: &[f32], channels: usize) -> Vec<f64> {
    match channels {
        0 => Vec::new(),
        1 => samples.iter().flat_map(|&s| [f64::from(s); 2]).collect(),
        2 => samples.iter().map(|&s| f64::from(s)).collect(),
        n => samples.chunks_exact(n).flat_map(|f| [f64::from(f[0]), f64::from(f[1])]).collect(),
    }
}

// shortcut: remuestreo lineal, se nota poco en voz/música a 48 kHz; usar rubato si alguien lo escucha mal
pub fn resample_to_44100(stereo: &[f64], from_hz: u32) -> Vec<f64> {
    if from_hz == 44100 || from_hz == 0 || stereo.len() < 2 {
        return stereo.to_vec();
    }
    let frames = stereo.len() / 2;
    let out_frames = (frames as u64 * 44100 / u64::from(from_hz)) as usize;
    let step = f64::from(from_hz) / 44100.0;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let p = i as f64 * step;
        let a = (p as usize).min(frames - 1);
        let b = (a + 1).min(frames - 1);
        let t = p - a as f64;
        for c in 0..2 {
            out.push(stereo[a * 2 + c] * (1.0 - t) + stereo[b * 2 + c] * t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::hls::parse_m3u8;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/external/fixtures")
    }

    fn m4a_source() -> RangeSource {
        let data = std::fs::read(fixtures().join("tone.m4a")).unwrap();
        RangeSource::new(
            data.len() as u64,
            Box::new(move |a, b| Ok(data[a as usize..=(b as usize).min(data.len() - 1)].to_vec())),
        )
    }

    fn hls_source(from: usize) -> HlsSource {
        let dir = fixtures().join("tone_hls");
        let hls = parse_m3u8(&std::fs::read_to_string(dir.join("tone.m3u8")).unwrap()).unwrap();
        HlsSource::new(hls, from, Box::new(move |name| std::fs::read(dir.join(name))))
    }

    /// Cuadros estéreo decodificados hasta el final.
    fn drain(mut d: Decoding) -> usize {
        let mut frames = 0;
        while let Some((s, ch, _)) = d.next_samples().unwrap() {
            frames += s.len() / ch;
        }
        frames
    }

    fn near(frames: usize, secs: f64, tol: f64) {
        let want = secs * 44100.0;
        assert!((frames as f64 - want).abs() <= want * tol, "{frames} vs {want}");
    }

    #[test]
    fn decodes_m4a_from_range_source() {
        near(drain(Decoding::open(Box::new(m4a_source())).unwrap()), 3.0, 0.05);
    }

    #[test]
    fn decodes_fmp4_hls_concatenated() {
        near(drain(Decoding::open(Box::new(hls_source(0))).unwrap()), 3.0, 0.10);
    }

    #[test]
    fn hls_from_middle_segment_decodes_rest() {
        near(drain(Decoding::open(Box::new(hls_source(1))).unwrap()), 2.0, 0.10);
    }

    #[test]
    fn seek_in_range_source() {
        let mut d = Decoding::open(Box::new(m4a_source())).unwrap();
        d.seek(2000).unwrap();
        near(drain(d), 1.0, 0.15);
        assert!(Decoding::open(Box::new(hls_source(0))).unwrap().seek(1000).is_err());
    }

    #[test]
    fn range_source_refreshes_url_once_on_403() {
        let calls = Arc::new(AtomicUsize::new(0));
        let refreshes = Arc::new(AtomicUsize::new(0));
        let (c, r) = (calls.clone(), refreshes.clone());
        let mut ok = RangeSource::new(
            4,
            Box::new(move |_, _| {
                if c.fetch_add(1, Ordering::SeqCst) == 0 { Err(FetchError::Expired) } else { Ok(vec![1, 2, 3, 4]) }
            }),
        )
        .with_refresh(Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        let mut out = Vec::new();
        ok.read_to_end(&mut out).unwrap();
        assert_eq!((out, refreshes.load(Ordering::SeqCst)), (vec![1, 2, 3, 4], 1));

        let r2 = Arc::new(AtomicUsize::new(0));
        let r3 = r2.clone();
        let mut bad = RangeSource::new(4, Box::new(|_, _| Err(FetchError::Expired)))
            .with_refresh(Box::new(move || {
                r3.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        assert!(bad.read_to_end(&mut Vec::new()).is_err());
        assert_eq!(r2.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn to_stereo_and_resample() {
        assert_eq!(to_stereo(&[0.5], 1), vec![0.5, 0.5]);
        assert_eq!(to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3), vec![1.0, 2.0, 4.0, 5.0]);
        let input = vec![0.1; 48000 * 2];
        let out = resample_to_44100(&input, 48000);
        assert!((out.len() as i64 / 2 - 44100).abs() <= 1);
        assert_eq!(resample_to_44100(&input, 44100), input);
    }
}
