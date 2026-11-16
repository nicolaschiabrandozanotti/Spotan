//! Segundo reproductor: baja y decodifica YouTube/SoundCloud y escribe en un sink de librespot.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use librespot_playback::audio_backend::{Sink, SinkBuilder};
use librespot_playback::config::{AudioFormat, VolumeCtrl};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::mappings::MappedCtrl;
use symphonia::core::io::MediaSource;

use super::stream::{to_stereo, resample_to_44100, Decoding, FetchError, HlsSource, RangeSource};
use super::{http, soundcloud, youtube};
use crate::events::{Event, EventManager};
use crate::model::track::{Source, Track};
use crate::queue::QueueEvent;
use crate::spotify::PlayerEvent;

pub enum ExtCmd {
    Load { track: Track, start_playing: bool, position_ms: u32 },
    Play,
    Pause,
    Stop,
    Seek(u32),
    SetVolume(u16),
}

/// (fuente, ms donde empieza la fuente) para un tema y una posición pedida.
pub(crate) type Resolver = Box<dyn Fn(&Track, u32) -> Result<(Box<dyn MediaSource>, u32), String> + Send>;

#[derive(Clone)]
pub struct ExternalPlayer {
    inner: Arc<Inner>,
}

/// Al soltar el último clon se cierra el canal y el hilo termina.
struct Inner {
    tx: Sender<(u64, ExtCmd)>,
    generation: Arc<AtomicU64>,
}

impl ExternalPlayer {
    /// False if the engine thread is gone (the command was not delivered).
    pub fn send(&self, cmd: ExtCmd) -> bool {
        // Load y Stop invalidan todo lo que el tema anterior todavía pueda emitir
        let g = &self.inner.generation;
        let epoch = if matches!(cmd, ExtCmd::Load { .. } | ExtCmd::Stop) {
            g.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            g.load(Ordering::SeqCst)
        };
        self.inner.tx.send((epoch, cmd)).is_ok()
    }
}

pub fn spawn(events: EventManager, backend: SinkBuilder, device: Option<String>, volume: u16) -> ExternalPlayer {
    spawn_with_resolver(events, backend, device, volume, Box::new(resolve))
}

pub(crate) fn spawn_with_resolver(events: EventManager, backend: SinkBuilder, device: Option<String>, volume: u16, resolver: Resolver) -> ExternalPlayer {
    let (tx, rx) = mpsc::channel();
    let generation = Arc::new(AtomicU64::new(0));
    let epoch = generation.clone();
    // Converter y sink no son Send: el Worker se arma dentro del hilo
    std::thread::Builder::new()
        .name("external-player".into())
        .spawn(move || {
            Worker {
                events,
                backend,
                device,
                resolver,
                generation: epoch,
                my_gen: 0,
                gain: gain(volume),
                sink: None,
                converter: Converter::new(None),
                current: None,
                playing: false,
            }
            .run(rx)
        })
        .expect("hilo del reproductor externo");
    ExternalPlayer { inner: Arc::new(Inner { tx, generation }) }
}

fn gain(volume: u16) -> f64 {
    VolumeCtrl::default().to_mapped(volume)
}

fn resolve(track: &Track, position_ms: u32) -> Result<(Box<dyn MediaSource>, u32), String> {
    let id = track.id.clone().ok_or("tema sin id")?;
    match track.source {
        Source::YouTube => resolve_youtube(id),
        Source::SoundCloud => resolve_soundcloud(&id, position_ms),
        Source::Spotify => Err("no es un tema externo".into()),
    }
}

fn resolve_youtube(id: String) -> Result<(Box<dyn MediaSource>, u32), String> {
    let info = youtube::stream(&id)?;
    let url = Arc::new(Mutex::new(info.url));
    let fetch_url = url.clone();
    let fetch = Box::new(move |start: u64, end: u64| {
        let url = fetch_url.lock().unwrap().clone();
        let resp = http()
            .get(url)
            .header("Range", format!("bytes={start}-{end}"))
            .header("User-Agent", youtube::UA)
            .send()
            .map_err(|e| FetchError::Other(e.to_string()))?;
        match resp.status().as_u16() {
            403 | 410 => Err(FetchError::Expired),
            s if !resp.status().is_success() => Err(FetchError::Other(format!("HTTP {s}"))),
            _ => resp.bytes().map(|b| b.to_vec()).map_err(|e| FetchError::Other(e.to_string())),
        }
    });
    let src = RangeSource::new(info.content_length, fetch).with_refresh(Box::new(move || {
        *url.lock().unwrap() = youtube::stream(&id)?.url;
        Ok(())
    }));
    Ok((Box::new(src), 0))
}

fn resolve_soundcloud(id: &str, position_ms: u32) -> Result<(Box<dyn MediaSource>, u32), String> {
    let hls = soundcloud::playlist(id)?;
    let (idx, start_ms) = hls.segment_at(position_ms);
    let fetch = Box::new(|url: &str| {
        http()
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.bytes())
            .map(|b| b.to_vec())
            .map_err(io::Error::other)
    });
    Ok((Box::new(HlsSource::new(hls, idx, fetch)), start_ms))
}

/// Tema cargado y por dónde va.
struct Current {
    track: Track,
    dec: Decoding,
    /// ms de la fuente en el cuadro 0 de `frames`.
    base_ms: u32,
    /// Cuadros a 44100 Hz decodificados desde `base_ms`.
    frames: u64,
    /// Se descartan muestras hasta esta posición (HLS no admite búsqueda).
    skip_until_ms: u32,
}

impl Current {
    fn position_ms(&self) -> u32 {
        // durante el descarte (HLS) el audio todavía no llegó a `skip_until_ms`
        (self.base_ms + (self.frames * 1000 / 44100) as u32).max(self.skip_until_ms)
    }
}

struct Worker {
    events: EventManager,
    backend: SinkBuilder,
    device: Option<String>,
    resolver: Resolver,
    generation: Arc<AtomicU64>,
    my_gen: u64,
    gain: f64,
    sink: Option<Box<dyn Sink>>,
    converter: Converter,
    current: Option<Current>,
    playing: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<(u64, ExtCmd)>) {
        loop {
            let msg = if self.playing {
                match rx.try_recv() {
                    Ok(m) => Some(m),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => break,
                }
            } else {
                match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => break,
                }
            };
            match msg {
                Some((epoch, cmd)) => self.handle(epoch, cmd),
                None => self.step(),
            }
        }
        self.close_sink();
    }

    fn stale(&self) -> bool {
        self.generation.load(Ordering::SeqCst) != self.my_gen
    }

    fn emit(&self, e: PlayerEvent) {
        if !self.stale() {
            self.events.send(Event::Player(e));
        }
    }

    fn emit_state(&self) {
        let Some(cur) = &self.current else { return };
        let pos = Duration::from_millis(u64::from(cur.position_ms()));
        self.emit(if self.playing {
            PlayerEvent::Playing(SystemTime::now() - pos)
        } else {
            PlayerEvent::Paused(pos)
        });
    }

    fn close_sink(&mut self) {
        if let Some(mut s) = self.sink.take() {
            let _ = s.stop();
        }
    }

    /// `uri`: the failed track; if None, the current one. The queue drops it so repeat can't retry forever.
    fn fail(&mut self, source: Source, uri: Option<String>) {
        if !self.stale() {
            self.events.notify(format!("{}: no se pudo reproducir", source.name()));
            let uri = uri.or_else(|| self.current.as_ref().map(|c| c.track.uri.clone()));
            if let Some(uri) = uri {
                self.events.send(Event::Queue(QueueEvent::TrackFailed(uri)));
            }
        }
        self.finish();
    }

    /// The audio device failed: say so and stop. The track is fine, so it isn't marked, and
    /// no FinishedTrack so the queue doesn't advance (or spin under repeat) into the same failure.
    fn sink_fail(&mut self, source: Source) {
        if !self.stale() {
            self.events.notify(format!("{}: no se pudo abrir la salida de audio", source.name()));
        }
        self.close_sink();
        self.current = None;
        self.playing = false;
        self.emit(PlayerEvent::Stopped);
    }

    fn finish(&mut self) {
        self.close_sink();
        self.current = None;
        self.playing = false;
        self.emit(PlayerEvent::FinishedTrack);
    }

    fn handle(&mut self, epoch: u64, cmd: ExtCmd) {
        match cmd {
            ExtCmd::Load { track, start_playing, position_ms } => {
                self.close_sink();
                self.current = None;
                self.playing = false;
                self.my_gen = epoch;
                let (source, uri) = (track.source, track.uri.clone());
                match self.open(track, position_ms) {
                    Ok(cur) if !self.stale() => {
                        self.current = Some(cur);
                        self.playing = start_playing;
                        self.emit_state();
                    }
                    Ok(_) => {}
                    Err(_) => self.fail(source, Some(uri)),
                }
            }
            ExtCmd::Play => {
                if self.current.is_some() && !self.playing {
                    self.playing = true;
                    self.emit_state();
                }
            }
            ExtCmd::Pause => {
                if self.playing {
                    self.playing = false;
                    self.close_sink();
                    self.emit_state();
                }
            }
            ExtCmd::Stop => {
                self.my_gen = epoch;
                self.close_sink();
                self.current = None;
                self.playing = false;
            }
            ExtCmd::Seek(ms) => {
                if let Some(mut cur) = self.current.take() {
                    let (source, uri) = (cur.track.source, cur.track.uri.clone());
                    match self.seek(&mut cur, ms) {
                        Ok(()) => {
                            self.current = Some(cur);
                            self.emit_state();
                        }
                        Err(_) => self.fail(source, Some(uri)),
                    }
                }
            }
            ExtCmd::SetVolume(v) => self.gain = gain(v),
        }
    }

    fn open(&self, track: Track, position_ms: u32) -> Result<Current, String> {
        let (src, base_ms) = (self.resolver)(&track, 0)?;
        let mut cur = Current { track, dec: Decoding::open(src)?, base_ms, frames: 0, skip_until_ms: 0 };
        if position_ms > 0 {
            self.seek(&mut cur, position_ms)?;
        }
        Ok(cur)
    }

    /// Primero busca en la fuente; si no admite (HLS), resuelve de nuevo desde la posición y descarta hasta ella.
    fn seek(&self, cur: &mut Current, ms: u32) -> Result<(), String> {
        if cur.dec.seek(ms).is_ok() {
            (cur.base_ms, cur.skip_until_ms) = (ms, 0);
        } else {
            let (src, base_ms) = (self.resolver)(&cur.track, ms)?;
            cur.dec = Decoding::open(src)?;
            (cur.base_ms, cur.skip_until_ms) = (base_ms, ms);
        }
        cur.frames = 0;
        Ok(())
    }

    /// Decodifica y escribe un paquete.
    fn step(&mut self) {
        let Some(cur) = self.current.as_mut() else {
            self.playing = false;
            return;
        };
        let source = cur.track.source;
        let (samples, channels, hz) = match cur.dec.next_samples() {
            Ok(Some(b)) => b,
            Ok(None) => return self.finish(),
            Err(_) => return self.fail(source, None),
        };
        let mut stereo = to_stereo(&samples, channels);
        if hz != 44100 {
            stereo = resample_to_44100(&stereo, hz);
        }
        cur.frames += (stereo.len() / 2) as u64;
        if cur.position_ms() <= cur.skip_until_ms {
            return;
        }
        for s in &mut stereo {
            *s *= self.gain;
        }
        if self.sink.is_none() {
            let mut sink = (self.backend)(self.device.clone(), AudioFormat::default());
            if sink.start().is_err() {
                return self.sink_fail(source);
            }
            self.sink = Some(sink);
        }
        let sink = self.sink.as_mut().unwrap();
        if sink.write(AudioPacket::Samples(stereo), &mut self.converter).is_err() {
            self.sink_fail(source);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::external::stream::RangeSource;
    use crate::model::track::Source;
    use crate::spotify::PlayerEvent;
    use crate::events::Event;
    use librespot_playback::audio_backend::{Sink, SinkResult};
    use librespot_playback::config::AudioFormat;
    use librespot_playback::convert::Converter;
    use librespot_playback::decoder::AudioPacket;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    // el SinkBuilder es un fn pointer: no captura, así que se cuenta en un static
    static PACKETS: AtomicUsize = AtomicUsize::new(0);

    struct CountSink;
    impl Sink for CountSink {
        fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            PACKETS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5)); // simula el tiempo real de la salida
            Ok(())
        }
    }

    pub(crate) fn test_backend(_: Option<String>, _: AudioFormat) -> Box<dyn Sink> {
        Box::new(CountSink)
    }

    pub(crate) fn tone_resolver() -> Resolver {
        Box::new(|_, _| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/external/fixtures/tone.m4a");
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            let src = RangeSource::new(
                data.len() as u64,
                Box::new(move |a, b| Ok(data[a as usize..=(b as usize).min(data.len() - 1)].to_vec())),
            );
            Ok((Box::new(src) as Box<dyn MediaSource>, 0))
        })
    }

    fn track() -> Track {
        Track::external(Source::YouTube, "x".into(), "t".into(), "a".into(), 3000, None, "u".into())
    }

    fn load(p: &ExternalPlayer, start_playing: bool) {
        p.send(ExtCmd::Load { track: track(), start_playing, position_ms: 0 });
    }

    /// Mantener `_keep` vivo: si se suelta, `EventManager::trigger` hace panic.
    fn setup() -> (ExternalPlayer, EventManager, impl Sized) {
        let (sink, keep) = crossbeam_channel::unbounded();
        let events = EventManager::new(sink);
        let p = spawn_with_resolver(events.clone(), test_backend, None, 65535, tone_resolver());
        (p, events, keep)
    }

    /// Eventos de reproductor hasta que `stop(evento)` o pasen `secs` segundos.
    fn collect(events: &EventManager, secs: u64, stop: impl Fn(&PlayerEvent) -> bool) -> Vec<PlayerEvent> {
        let end = Instant::now() + Duration::from_secs(secs);
        let mut got = Vec::new();
        while Instant::now() < end {
            for e in events.msg_iter() {
                if let Event::Player(e) = e {
                    let done = stop(&e);
                    got.push(e);
                    if done {
                        return got;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        got
    }

    #[test]
    fn plays_to_end_and_emits_finished() {
        let before = PACKETS.load(Ordering::SeqCst);
        let (p, events, _k) = setup();
        load(&p, true);
        let got = collect(&events, 10, |e| *e == PlayerEvent::FinishedTrack);
        assert!(matches!(got.first(), Some(PlayerEvent::Playing(_))), "{got:?}");
        assert_eq!(got.last(), Some(&PlayerEvent::FinishedTrack));
        assert!(PACKETS.load(Ordering::SeqCst) > before);
    }

    #[test]
    fn stale_generation_events_are_dropped() {
        let (p, events, _k) = setup();
        load(&p, true);
        // A ya está sonando cuando entra B
        collect(&events, 5, |e| matches!(e, PlayerEvent::Playing(_)));
        load(&p, true);
        let got = collect(&events, 10, |e| *e == PlayerEvent::FinishedTrack);
        std::thread::sleep(Duration::from_millis(300));
        let rest: Vec<_> = events.msg_iter().collect();
        assert!(matches!(got.as_slice(), [PlayerEvent::Playing(_), PlayerEvent::FinishedTrack]), "{got:?}");
        assert!(rest.is_empty());
    }

    #[test]
    fn stale_resolve_does_not_leak_events() {
        let inner = tone_resolver();
        let slow_a: Resolver = Box::new(move |t, pos| {
            if t.id.as_deref() == Some("A") {
                std::thread::sleep(Duration::from_millis(300));
            }
            inner(t, pos)
        });
        let (sink, _keep) = crossbeam_channel::unbounded();
        let events = EventManager::new(sink);
        let p = spawn_with_resolver(events.clone(), test_backend, None, 65535, slow_a);
        for id in ["A", "B"] {
            let mut t = track();
            t.id = Some(id.into());
            p.send(ExtCmd::Load { track: t, start_playing: true, position_ms: 0 });
        }
        let got = collect(&events, 10, |e| *e == PlayerEvent::FinishedTrack);
        std::thread::sleep(Duration::from_millis(300));
        assert!(matches!(got.as_slice(), [PlayerEvent::Playing(_), PlayerEvent::FinishedTrack]), "{got:?}");
        assert!(events.msg_iter().next().is_none());
    }

    #[test]
    fn hls_positioned_load_reports_target_not_segment_start() {
        use crate::external::hls::parse_m3u8;
        use crate::external::stream::HlsSource;
        let resolver: Resolver = Box::new(|_, pos| {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/external/fixtures/tone_hls");
            let hls = parse_m3u8(&std::fs::read_to_string(dir.join("tone.m3u8")).unwrap()).unwrap();
            let (idx, start) = hls.segment_at(pos);
            let src = HlsSource::new(hls, idx, Box::new(move |n| std::fs::read(dir.join(n))));
            Ok((Box::new(src) as Box<dyn MediaSource>, start))
        });
        let (sink, _keep) = crossbeam_channel::unbounded();
        let events = EventManager::new(sink);
        let p = spawn_with_resolver(events.clone(), test_backend, None, 65535, resolver);
        p.send(ExtCmd::Load { track: track(), start_playing: false, position_ms: 1500 });
        let got = collect(&events, 5, |e| matches!(e, PlayerEvent::Paused(_)));
        match got.last() {
            Some(PlayerEvent::Paused(d)) => assert!(*d >= Duration::from_millis(1500), "{d:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stop_emits_nothing() {
        let (p, events, _k) = setup();
        load(&p, true);
        collect(&events, 5, |e| matches!(e, PlayerEvent::Playing(_)));
        p.send(ExtCmd::Stop);
        std::thread::sleep(Duration::from_millis(300));
        assert!(events.msg_iter().all(|e| !matches!(e, Event::Player(_))));
    }

    #[test]
    fn pause_reports_position() {
        let (p, events, _k) = setup();
        load(&p, true);
        collect(&events, 5, |e| matches!(e, PlayerEvent::Playing(_)));
        std::thread::sleep(Duration::from_millis(150));
        p.send(ExtCmd::Pause);
        let got = collect(&events, 5, |e| matches!(e, PlayerEvent::Paused(_)));
        match got.last() {
            Some(PlayerEvent::Paused(d)) => assert!(*d > Duration::ZERO && *d < Duration::from_secs(3), "{d:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn send_reports_a_dead_engine() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let p = ExternalPlayer { inner: Arc::new(Inner { tx, generation: Arc::new(AtomicU64::new(0)) }) };
        assert!(!p.send(ExtCmd::Pause));
    }

    #[test]
    fn failed_load_asks_queue_to_drop_the_track() {
        let (sink, _keep) = crossbeam_channel::unbounded();
        let events = EventManager::new(sink);
        let failing: Resolver = Box::new(|_, _| Err("boom".into()));
        let p = spawn_with_resolver(events.clone(), test_backend, None, 65535, failing);
        load(&p, true);
        let end = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while Instant::now() < end && !got.iter().any(|e| matches!(e, Event::Player(PlayerEvent::FinishedTrack))) {
            got.extend(events.msg_iter());
            std::thread::sleep(Duration::from_millis(10));
        }
        let uri = track().uri;
        let failed = got.iter().position(|e| matches!(e, Event::Queue(crate::queue::QueueEvent::TrackFailed(u)) if *u == uri));
        let finished = got.iter().position(|e| matches!(e, Event::Player(PlayerEvent::FinishedTrack)));
        assert!(failed.is_some() && failed < finished, "{}", got.len());
    }

    struct BrokenSink;
    impl Sink for BrokenSink {
        fn start(&mut self) -> SinkResult<()> {
            Err(librespot_playback::audio_backend::SinkError::NotConnected("sin dispositivo".into()))
        }
        fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            Ok(())
        }
    }

    #[test]
    fn sink_failure_stops_without_marking_the_track_or_advancing() {
        let (sink, _keep) = crossbeam_channel::unbounded();
        let events = EventManager::new(sink);
        let p = spawn_with_resolver(events.clone(), |_, _| Box::new(BrokenSink), None, 65535, tone_resolver());
        load(&p, true);
        std::thread::sleep(Duration::from_secs(2));
        let got: Vec<_> = events.msg_iter().collect();
        assert!(!got.iter().any(|e| matches!(e, Event::Queue(_))), "{}", got.len());
        assert!(!got.iter().any(|e| matches!(e, Event::Player(PlayerEvent::FinishedTrack))));
        assert!(got.iter().any(|e| matches!(e, Event::Player(PlayerEvent::Stopped))));
    }
}
