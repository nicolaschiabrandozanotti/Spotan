//! Barra `[Spotify] [YouTube] [SoundCloud]` de la pestaña Tracks y la lógica pura
//! para mezclar y filtrar resultados por servicio.

use crate::external::Page;
use crate::model::track::{Source, Track};
use cursive::event::{Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::ColorStyle;
use cursive::{Printer, Vec2, View};
use std::collections::HashSet;

const ORDER: [Source; 3] = [Source::Spotify, Source::YouTube, Source::SoundCloud];

/// Orden estable Spotify -> YouTube -> SoundCloud (cada uno en su orden); solo los `on`.
pub fn merge_and_filter(all: &[Track], on: &HashSet<Source>) -> Vec<Track> {
    ORDER
        .iter()
        .filter(|s| on.contains(s))
        .flat_map(|s| all.iter().filter(move |t| t.source == *s))
        .cloned()
        .collect()
}

/// Junta los resultados de cada servicio; un servicio que falló solo deja un aviso.
pub fn build_results(results: Vec<(Source, Result<Page, String>)>) -> (Vec<Track>, Vec<String>) {
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    for (source, result) in results {
        match result {
            Ok(page) => tracks.extend(page.tracks),
            Err(_) => warnings.push(format!("{}: no se pudo buscar", source.name())),
        }
    }
    (tracks, warnings)
}

pub struct SourceFilter {
    active: Vec<Source>,
    on: HashSet<Source>,
    on_change: Option<Box<dyn Fn(&HashSet<Source>) + Send + Sync>>,
}

impl SourceFilter {
    /// Todos los servicios activos arrancan prendidos.
    pub fn new(active: Vec<Source>) -> Self {
        let on = active.iter().copied().collect();
        Self { active, on, on_change: None }
    }

    pub fn on_change(&mut self, cb: impl Fn(&HashSet<Source>) + Send + Sync + 'static) {
        self.on_change = Some(Box::new(cb));
    }

    /// Con menos de dos servicios no hay nada que filtrar.
    pub fn visible(&self) -> bool {
        self.active.len() >= 2
    }

    pub fn toggle(&mut self, index: usize) {
        let Some(source) = self.active.get(index).copied() else { return };
        if !self.on.remove(&source) {
            self.on.insert(source);
        }
        if let Some(cb) = &self.on_change {
            cb(&self.on);
        }
    }

    /// Tecla `1`..`9` -> posición del servicio activo que alterna.
    pub fn digit_index(active_len: usize, c: char) -> Option<usize> {
        let i = (c.to_digit(10)? as usize).checked_sub(1)?;
        (i < active_len).then_some(i)
    }

    /// Alterna con una tecla numérica; `true` si la consumió.
    pub fn toggle_digit(&mut self, c: char) -> bool {
        match self.visible().then(|| Self::digit_index(self.active.len(), c)).flatten() {
            Some(i) => {
                self.toggle(i);
                true
            }
            None => false,
        }
    }

    /// Posición x y ancho de cada botón: `[N Nombre]` separados por un espacio.
    fn spans(&self) -> Vec<(usize, usize)> {
        let mut x = 0;
        self.active
            .iter()
            .map(|s| {
                let w = s.name().len() + 4;
                let span = (x, w);
                x += w + 1;
                span
            })
            .collect()
    }
}

impl View for SourceFilter {
    fn required_size(&mut self, _: Vec2) -> Vec2 {
        if !self.visible() {
            return Vec2::zero();
        }
        let width = self.spans().last().map_or(0, |(x, w)| x + w);
        Vec2::new(width, 1)
    }

    fn draw(&self, printer: &Printer) {
        if !self.visible() {
            return;
        }
        for (i, (source, (x, _))) in self.active.iter().zip(self.spans()).enumerate() {
            let style = if self.on.contains(source) { ColorStyle::title_primary() } else { ColorStyle::secondary() };
            printer.with_color(style, |p| p.print((x, 0), &format!("[{} {}]", i + 1, source.name())));
        }
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        if let Event::Mouse { offset, position, event: MouseEvent::Press(MouseButton::Left) } = event
            && self.visible()
            && let Some(rel) = position.checked_sub(offset)
            && let Some(i) = self.spans().iter().position(|(x, w)| rel.y == 0 && rel.x >= *x && rel.x < x + w)
        {
            self.toggle(i);
            return EventResult::consumed();
        }
        EventResult::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(source: Source, id: &str) -> Track {
        Track::external(source, id.into(), id.into(), "a".into(), 1000, None, format!("https://x/{id}"))
    }

    fn all() -> Vec<Track> {
        vec![t(Source::SoundCloud, "sc1"), t(Source::YouTube, "yt1"), t(Source::Spotify, "s1"), t(Source::YouTube, "yt2")]
    }

    fn ids(v: &[Track]) -> Vec<String> {
        v.iter().map(|t| t.title.clone()).collect()
    }

    #[test]
    fn merge_orders_by_source_then_relevance() {
        let on = ORDER.iter().copied().collect();
        assert_eq!(ids(&merge_and_filter(&all(), &on)), ["s1", "yt1", "yt2", "sc1"]);
    }

    #[test]
    fn filter_hides_sources_off() {
        let on = HashSet::from([Source::YouTube, Source::SoundCloud]);
        assert_eq!(ids(&merge_and_filter(&all(), &on)), ["yt1", "yt2", "sc1"]);
    }

    #[test]
    fn failed_service_keeps_other_results() {
        let ok = Page { tracks: vec![t(Source::SoundCloud, "sc1")], next: None };
        let (tracks, warnings) =
            build_results(vec![(Source::YouTube, Err("boom".into())), (Source::SoundCloud, Ok(ok))]);
        assert_eq!(ids(&tracks), ["sc1"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("YouTube"));
    }

    #[test]
    fn filter_bar_hidden_with_one_service() {
        let mut one = SourceFilter::new(vec![Source::YouTube]);
        assert!(!one.visible());
        assert_eq!(one.required_size(Vec2::new(80, 5)), Vec2::zero());
        let mut two = SourceFilter::new(vec![Source::Spotify, Source::YouTube]);
        assert!(two.visible());
        assert_eq!(two.required_size(Vec2::new(80, 5)), Vec2::new(11 + 1 + 11, 1));
    }

    #[test]
    fn digits_map_to_active_services() {
        assert_eq!(SourceFilter::digit_index(2, '1'), Some(0));
        assert_eq!(SourceFilter::digit_index(2, '2'), Some(1));
        assert_eq!(SourceFilter::digit_index(2, '3'), None);
        assert_eq!(SourceFilter::digit_index(3, '0'), None);
        assert_eq!(SourceFilter::digit_index(3, 'x'), None);
        let mut one = SourceFilter::new(vec![Source::YouTube]);
        assert!(!one.toggle_digit('1'));
        let mut two = SourceFilter::new(vec![Source::YouTube, Source::SoundCloud]);
        assert!(two.toggle_digit('2'));
        assert!(!two.on.contains(&Source::SoundCloud));
    }

    #[test]
    fn toggle_switches_source_and_notifies() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let s2 = seen.clone();
        let mut f = SourceFilter::new(vec![Source::Spotify, Source::YouTube]);
        f.on_change(move |on| *s2.lock().unwrap() = Some(on.clone()));
        f.toggle(1);
        assert_eq!(seen.lock().unwrap().clone().unwrap(), HashSet::from([Source::Spotify]));
    }
}
