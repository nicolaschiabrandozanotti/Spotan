use spoti_control::{elapsed, Action, Snapshot};

pub const WIDTH: u32 = 436;
pub const CLOSED_HEIGHT: u32 = 24;
pub const OPEN_HEIGHT: u32 = 148;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Arrow,
    Previous,
    Backward,
    Play,
    Forward,
    Next,
    Mute,
    Seek,
    Volume,
}
#[derive(Clone, Copy, Debug)]
pub struct Rect(pub f64, pub f64, pub f64, pub f64);
impl Rect {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.0 && y >= self.1 && x < self.0 + self.2 && y < self.1 + self.3
    }
    pub fn fraction(self, x: f64) -> f64 {
        ((x - self.0) / self.2).clamp(0.0, 1.0)
    }
}
pub const ARROW: Rect = Rect(191.0, 0.0, 54.0, 18.0);
pub const SEEK: Rect = Rect(64.0, 76.0, 306.0, 22.0);
pub const VOLUME: Rect = Rect(290.0, 101.0, 88.0, 28.0);
pub fn hit(x: f64, y: f64, open: bool) -> Option<Target> {
    if ARROW.contains(x, y) {
        return Some(Target::Arrow);
    }
    if !open {
        return None;
    }
    if SEEK.contains(x, y) {
        return Some(Target::Seek);
    }
    if VOLUME.contains(x, y) {
        return Some(Target::Volume);
    }
    [
        (22.0, 40.0, Target::Previous),
        (66.0, 41.0, Target::Backward),
        (111.0, 50.0, Target::Play),
        (165.0, 41.0, Target::Forward),
        (210.0, 38.0, Target::Next),
        (252.0, 34.0, Target::Mute),
    ]
    .into_iter()
    .find_map(|(left, width, t)| Rect(left, 101.0, width, 28.0).contains(x, y).then_some(t))
}
#[derive(Default)]
pub struct Model {
    pub snapshot: Option<Snapshot>,
    pub open: bool,
    pub hover: Option<Target>,
    pub drag: Option<Target>,
    pub seek_preview: Option<f64>,
    pub volume_preview: Option<f64>,
    remembered_volume: Option<f64>,
    pressed: Option<Target>,
}
impl Model {
    pub fn available(&self) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|s| s.state.playable.as_ref())
            .is_some()
    }
    pub fn duration(&self) -> f64 {
        self.snapshot
            .as_ref()
            .and_then(|s| s.state.playable.as_ref())
            .map_or(0.0, |t| t.duration as f64 / 1000.0)
    }
    pub fn position(&self, now: f64) -> f64 {
        self.seek_preview
            .unwrap_or_else(|| {
                self.snapshot
                    .as_ref()
                    .map_or(0.0, |s| elapsed(&s.state, now))
            })
            .clamp(0.0, self.duration())
    }
    pub fn volume(&self) -> Option<f64> {
        self.volume_preview
            .or_else(|| self.snapshot.as_ref().and_then(|s| s.volume))
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
    }
    pub fn playing(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|s| s.state.mode.get("Playing").is_some())
    }
    pub fn apply(&mut self, snapshot: Option<Snapshot>) {
        self.snapshot = snapshot;
        if self.drag != Some(Target::Seek) {
            self.seek_preview = None;
        }
        if self.drag != Some(Target::Volume) {
            self.volume_preview = None;
        }
        if let Some(v) = self.volume().filter(|v| *v > 0.001) {
            self.remembered_volume = Some(v);
        }
    }
    pub fn motion(&mut self, x: f64, y: f64) -> Option<Action> {
        self.hover = hit(x, y, self.open);
        match self.drag {
            Some(Target::Seek) => {
                let s = SEEK.fraction(x) * self.duration();
                self.seek_preview = Some(s);
                Some(Action::Seek(s))
            }
            Some(Target::Volume) => {
                let v = VOLUME.fraction(x);
                self.volume_preview = Some(v);
                Some(Action::Volume(v))
            }
            _ => None,
        }
    }
    pub fn press(&mut self, x: f64, y: f64) -> Option<Action> {
        let t = hit(x, y, self.open);
        self.pressed = t;
        if !self.available() {
            return None;
        }
        if t == Some(Target::Seek) || (t == Some(Target::Volume) && self.volume().is_some()) {
            self.drag = t;
            return self.motion(x, y);
        }
        None
    }
    pub fn release(&mut self, x: f64, y: f64) -> Option<Action> {
        let drag = self.drag.take();
        let pressed = self.pressed.take();
        if drag.is_some() {
            return None;
        }
        let t = hit(x, y, self.open);
        if t != pressed {
            return None;
        }
        if t == Some(Target::Arrow) {
            self.open = !self.open;
            return None;
        }
        if !self.available() {
            return None;
        }
        match t? {
            Target::Previous => Some(Action::Previous),
            Target::Next => Some(Action::Next),
            Target::Backward => Some(Action::SeekRelative(-10)),
            Target::Forward => Some(Action::SeekRelative(10)),
            Target::Play => Some(Action::PlayPause),
            Target::Mute => {
                let v = self.volume()?;
                let new = if v > 0.001 {
                    self.remembered_volume = Some(v);
                    0.0
                } else {
                    self.remembered_volume.unwrap_or(0.5)
                };
                self.volume_preview = Some(new);
                Some(Action::Volume(new))
            }
            _ => None,
        }
    }
    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
        self.drag = None;
        self.pressed = None;
        self.seek_preview = None;
        self.volume_preview = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Model {
        let mut m = Model {
            open: true,
            ..Model::default()
        };
        m.apply(Some(Snapshot {
            state: spoti_control::State {
                mode: Default::default(),
                playable: Some(spoti_control::Track {
                    duration: 200_000,
                    ..Default::default()
                }),
            },
            volume: Some(0.73),
        }));
        m
    }
    #[test]
    fn buttons_fire_on_release_inside_only() {
        let mut m = ready();
        assert_eq!(m.press(130.0, 115.0), None);
        assert_eq!(m.release(130.0, 115.0), Some(Action::PlayPause));
        m.press(130.0, 115.0);
        assert_eq!(m.release(200.0, 115.0), None);
    }
    #[test]
    fn drag_is_captured_clamped_and_not_reset_by_snapshots() {
        let mut m = ready();
        assert_eq!(m.press(217.0, 85.0), Some(Action::Seek(100.0)));
        assert_eq!(m.motion(999.0, 500.0), Some(Action::Seek(200.0)));
        m.apply(m.snapshot.clone());
        assert_eq!(m.position(0.0), 200.0);
        m.release(999.0, 500.0);
        assert!(m.drag.is_none());
        m.apply(m.snapshot.clone());
        assert!(m.seek_preview.is_none());
    }
    #[test]
    fn mute_restores_exact_volume() {
        let mut m = ready();
        m.press(265.0, 115.0);
        assert_eq!(m.release(265.0, 115.0), Some(Action::Volume(0.0)));
        m.press(265.0, 115.0);
        assert_eq!(m.release(265.0, 115.0), Some(Action::Volume(0.73)));
    }
    #[test]
    fn disconnected_and_legacy_volume_cannot_send_controls() {
        let mut m = Model {
            open: true,
            ..Default::default()
        };
        assert_eq!(m.press(320.0, 115.0), None);
        assert_eq!(m.release(320.0, 115.0), None);
        m = ready();
        m.snapshot.as_mut().unwrap().volume = None;
        assert_eq!(m.press(320.0, 115.0), None);
        assert_eq!(m.release(320.0, 115.0), None);
    }
    #[test]
    fn closed_input_is_only_arrow() {
        assert_eq!(hit(130.0, 115.0, false), None);
        assert_eq!(hit(218.0, 8.0, false), Some(Target::Arrow));
    }
}
