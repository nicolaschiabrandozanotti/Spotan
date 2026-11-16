use std::sync::Arc;

use cursive::event::{Event, EventResult};
use cursive::theme::{ColorStyle, PaletteColor};
use cursive::traits::{Nameable, Resizable};
use cursive::utils::markup::StyledString;
use cursive::view::{View, ViewWrapper};
use cursive::{Cursive, Printer, Rect, Vec2};
use cursive::views::{Button, DummyView, EditView, LinearLayout, TextView};
use librespot_core::authentication::Credentials as RespotCredentials;

use crate::application::UserData;
use crate::authentication::{self, ApiSession};
use crate::command::Command;
use crate::traits::ViewExt;
use crate::ui::spoti_background::Background;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsMode {
    /// A regular screen inside the main layout.
    Screen,
    /// Standalone startup screen: only the Spotify login is offered.
    Start,
}

pub const MSG: &str = "settings_msg";
const API_STATUS: &str = "settings_api_status";
const API_ID: &str = "settings_api_id";
const BTN_SPOTIFY: &str = "settings_btn_spotify";
const BTN_API: &str = "settings_btn_api";
const BTN_SHARED: &str = "settings_btn_shared";

/// Width of the text area and of the Client ID field; the box adds `PAD` around it.
const CONTENT_WIDTH: usize = 54;
const CLIENT_ID_WIDTH: usize = 36;
/// Border plus one column/row of breathing room: (left+right, top+bottom).
const PAD: Vec2 = Vec2 { x: 4, y: 2 };

pub struct SettingsView {
    inner: LinearLayout,
    mode: SettingsMode,
    background: Background,
    /// Where the content starts and the whole box, both set by `layout`.
    origin: Vec2,
    frame: Rect,
}

/// The rectangle of `content` size centered in `screen`, shrunk to fit when it does not.
pub fn centered_box(screen: Vec2, content: Vec2) -> Rect {
    let size = Vec2::min(content, screen);
    Rect::from_size((screen - size) / 2, size)
}

fn heading(text: &str) -> StyledString {
    StyledString::styled(text, ColorStyle::front(PaletteColor::TitlePrimary))
}

fn hint(text: &str) -> StyledString {
    StyledString::styled(text, ColorStyle::front(PaletteColor::Secondary))
}

fn api_status_text(session: &ApiSession, own_app: bool) -> &'static str {
    match (session, own_app) {
        (ApiSession::None, _) => "sin sesión",
        (ApiSession::Active, true) => "activa (app propia)",
        (ApiSession::Active, false) => "activa (app compartida)",
        (ApiSession::Expired, _) => "Sesión de API vencida - volvé a iniciar sesión",
    }
}

fn current_api_status() -> &'static str {
    api_status_text(&authentication::api_session(), authentication::api_client_id().is_some())
}

fn set_text(s: &mut Cursive, name: &str, text: String) {
    // Errors get the theme's error colors.
    let palette = &s.current_theme().palette;
    let styled = match (palette.custom("error"), palette.custom("error_bg")) {
        (Some(&fg), Some(&bg)) if text.starts_with("Error") => {
            StyledString::styled(text, ColorStyle::new(fg, bg))
        }
        _ => StyledString::plain(text),
    };
    s.call_on_name(name, |v: &mut TextView| v.set_content(styled));
}

/// Show `text` in the message line (styled as an error when it starts with "Error").
pub fn set_message(s: &mut Cursive, text: String) {
    set_text(s, MSG, text);
}

fn set_enabled(s: &mut Cursive, enabled: bool) {
    for name in [BTN_SPOTIFY, BTN_API, BTN_SHARED] {
        s.call_on_name(name, |b: &mut Button| b.set_enabled(enabled));
    }
}

/// Run a blocking login on a plain thread (never inside cursive/tokio) and bring the result back
/// to the UI thread. `login` receives the callback that publishes the URL.
fn start_login<T: Send + 'static>(
    s: &mut Cursive,
    login: impl FnOnce(&dyn Fn(&str)) -> Result<T, String> + Send + 'static,
    done: impl FnOnce(&mut Cursive, Result<T, String>) + Send + 'static,
) {
    set_enabled(s, false);
    set_text(s, MSG, format!("Esperando el navegador{}", crate::utils::glyph("…", "...")));
    let sink = s.cb_sink().clone();
    std::thread::spawn(move || {
        let url_sink = sink.clone();
        let result = login(&|url| {
            let url = url.to_string();
            let _ = url_sink.send(Box::new(move |s| {
                set_text(s, MSG, format!("Esperando el navegador{}\n{url}", crate::utils::glyph("…", "...")));
            }));
        });
        // `done` decides when to re-enable the buttons.
        let _ = sink.send(Box::new(move |s| {
            done(s, result);
        }));
    });
}

fn refresh_api(s: &mut Cursive) {
    set_text(s, API_STATUS, current_api_status().into());
}

fn save_client_id(s: &mut Cursive) -> Result<(), String> {
    let id = s.call_on_name(API_ID, |v: &mut EditView| v.get_content()).unwrap_or_default();
    authentication::set_api_client_id(Some(id.as_str()))
}

impl SettingsView {
    pub fn new(
        mode: SettingsMode,
        account: Option<String>,
        on_logged_in: Box<dyn Fn(RespotCredentials) -> Result<(), String> + Send + Sync>,
    ) -> Self {
        let on_logged_in: Arc<dyn Fn(RespotCredentials) -> Result<(), String> + Send + Sync> = Arc::from(on_logged_in);
        let start = mode == SettingsMode::Start;
        let mut layout = LinearLayout::vertical()
            .child(TextView::new(heading(if start { "Spoti" } else { "Ajustes" })).center())
            .child(DummyView);
        let logged_in = account.is_some();
        if start {
            layout.add_child(TextView::new("Conectá tu cuenta de Spotify para empezar a escuchar."));
        } else {
            layout.add_child(TextView::new(heading("Cuenta de Spotify")));
            layout.add_child(TextView::new(account.as_deref().unwrap_or("sin sesión").to_string()));
        }
        layout.add_child(DummyView);
        let login_label = match (logged_in, start) {
            (true, _) => "Cerrar sesión",
            (false, true) => "Iniciar sesión con Spotify",
            (false, false) => "Iniciar sesión en Spotify",
        };
        layout.add_child(
            Button::new(login_label, move |s| {
                if logged_in {
                    crate::restart::request();
                    match s.user_data::<UserData>().cloned() {
                        // Quit saves the player state first.
                        Some(data) => data.cmd.handle(s, Command::Quit),
                        None => s.quit(),
                    }
                } else {
                    let on_logged_in = on_logged_in.clone();
                    start_login(
                        s,
                        // Runs on the login thread: the buttons stay disabled until it ends.
                        move |on_url| on_logged_in(authentication::create_credentials_with(on_url)?),
                        |s, result| match result {
                            // Start mode quits right after; keep the buttons disabled.
                            Ok(()) => set_text(s, MSG, "Sesión iniciada".into()),
                            Err(e) => {
                                set_enabled(s, true);
                                set_text(s, MSG, format!("Error: {e}"));
                            }
                        },
                    );
                }
            })
            .with_name(BTN_SPOTIFY),
        );
        if start {
            layout.add_child(DummyView);
            layout.add_child(TextView::new(hint("Se abre el navegador; cuando aceptes, volvé acá.")));
        }
        if mode == SettingsMode::Screen || logged_in {
            layout.add_child(DummyView);
            layout.add_child(TextView::new(heading("API oficial (opcional)")));
            layout.add_child(
                LinearLayout::horizontal()
                    .child(TextView::new("Client ID: "))
                    .child(
                        EditView::new()
                            .content(authentication::api_client_id().unwrap_or_default())
                            .with_name(API_ID)
                            .max_width(CLIENT_ID_WIDTH),
                    ),
            );
            layout.add_child(
                LinearLayout::horizontal()
                    .child(TextView::new("Estado: "))
                    .child(TextView::new(current_api_status()).with_name(API_STATUS)),
            );
            layout.add_child(
                LinearLayout::horizontal()
                    .child(
                        Button::new("Iniciar sesión en la API", |s| {
                            if let Err(e) = save_client_id(s) {
                                return set_text(s, MSG, format!("Error: {e}"));
                            }
                            start_login(
                                s,
                                |on_url| authentication::api_login(on_url),
                                |s, result| {
                                    set_enabled(s, true);
                                    refresh_api(s);
                                    set_text(s, MSG, match result {
                                        Ok(()) => "Sesión de API iniciada".into(),
                                        Err(e) => format!("Error: {e}"),
                                    });
                                },
                            );
                        })
                        .with_name(BTN_API),
                    )
                    .child(DummyView.fixed_width(1))
                    .child(
                        Button::new("Volver a la compartida", |s| {
                            match authentication::set_api_client_id(None) {
                                Ok(()) => {
                                    s.call_on_name(API_ID, |v: &mut EditView| v.set_content(""));
                                    set_text(s, MSG, String::new());
                                }
                                Err(e) => set_text(s, MSG, format!("Error: {e}")),
                            }
                            refresh_api(s);
                        })
                        .with_name(BTN_SHARED),
                    ),
            );
            layout.add_child(TextView::new(hint(
                "Cómo sacarla: developer.spotify.com > Create app > Redirect URI: http://127.0.0.1/ncspot_login > copiá el Client ID",
            )));
        }
        layout.add_child(DummyView);
        layout.add_child(TextView::new("").with_name(MSG));
        Self {
            inner: layout,
            mode,
            background: Background::default(),
            origin: Vec2::zero(),
            frame: Rect::from_size((0, 0), (0, 0)),
        }
    }
}

impl ViewWrapper for SettingsView {
    cursive::wrap_impl!(self.inner: LinearLayout);

    fn wrap_required_size(&mut self, req: Vec2) -> Vec2 {
        req
    }

    fn wrap_layout(&mut self, size: Vec2) {
        let width = CONTENT_WIDTH.min(size.x.saturating_sub(PAD.x));
        let wanted = self.inner.required_size(Vec2::new(width, size.y.saturating_sub(PAD.y)));
        self.frame = centered_box(size, wanted.map_x(|x| x.min(width)) + PAD);
        self.origin = self.frame.top_left() + (PAD / 2);
        self.inner.layout(self.frame.size().saturating_sub(PAD));
    }

    fn wrap_on_event(&mut self, event: Event) -> EventResult {
        self.inner.on_event(event.relativized(self.origin))
    }

    fn wrap_draw(&self, printer: &Printer) {
        self.background.clear(printer);
        let palette = &printer.theme.palette;
        // A different background keeps the animated dots out of the box.
        let fill = palette.custom("playing_bg").copied().unwrap_or(palette[PaletteColor::Highlight]);
        let inner = self.frame.size().saturating_sub(PAD);
        printer.with_color(ColorStyle::new(palette[PaletteColor::Primary], fill), |printer| {
            let size = self.frame.size();
            if size.x >= 2 && size.y >= 2 {
                let boxed = printer.offset(self.frame.top_left()).cropped(size);
                for y in 0..size.y {
                    boxed.print_hline((0, y), size.x, " ");
                }
                printer.with_color(ColorStyle::new(palette[PaletteColor::TitlePrimary], fill), |_| {
                    boxed.print_hline((1, 0), size.x - 2, crate::utils::glyph("─", "-"));
                    boxed.print_hline((1, size.y - 1), size.x - 2, crate::utils::glyph("─", "-"));
                    boxed.print_vline((0, 1), size.y - 2, crate::utils::glyph("│", "|"));
                    boxed.print_vline((size.x - 1, 1), size.y - 2, crate::utils::glyph("│", "|"));
                    boxed.print((0, 0), crate::utils::glyph("╭", "+"));
                    boxed.print((size.x - 1, 0), crate::utils::glyph("╮", "+"));
                    boxed.print((0, size.y - 1), crate::utils::glyph("╰", "+"));
                    boxed.print((size.x - 1, size.y - 1), crate::utils::glyph("╯", "+"));
                });
            }
            if inner.x > 0 && inner.y > 0 {
                self.inner.draw(&printer.offset(self.origin).cropped(inner));
            }
        });
        if self.mode == SettingsMode::Start {
            self.background.draw(printer, 0, printer.size.y);
        }
    }
}

impl ViewExt for SettingsView {
    fn title(&self) -> String {
        "Ajustes".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_box_centers_and_fits() {
        let c = |sx, sy, cx, cy| centered_box(Vec2::new(sx, sy), Vec2::new(cx, cy));
        let r = c(200, 60, 58, 20);
        assert_eq!((r.top_left(), r.size()), (Vec2::new(71, 20), Vec2::new(58, 20)));
        let r = c(80, 24, 58, 20);
        assert_eq!((r.top_left(), r.size()), (Vec2::new(11, 2), Vec2::new(58, 20)));
        let r = c(40, 12, 58, 20);
        assert_eq!((r.top_left(), r.size()), (Vec2::new(0, 0), Vec2::new(40, 12)));
    }

    #[test]
    fn draws_without_panic_at_any_size() {
        for mode in [SettingsMode::Start, SettingsMode::Screen] {
            for (w, h) in [(200, 60), (80, 24), (40, 12), (5, 3), (1, 1), (0, 0)] {
                let backend = cursive::backends::puppet::Backend::init(Some(Vec2::new(w, h)));
                let mut runner = Cursive::new().into_runner(backend);
                runner.set_theme(crate::theme::load(&None));
                runner.add_fullscreen_layer(SettingsView::new(mode, None, Box::new(|_| Ok(()))));
                runner.step();
                runner.refresh();
            }
        }
    }

    #[test]
    fn settings_api_status_text() {
        assert_eq!(api_status_text(&ApiSession::None, true), "sin sesión");
        assert_eq!(api_status_text(&ApiSession::Active, true), "activa (app propia)");
        assert_eq!(api_status_text(&ApiSession::Active, false), "activa (app compartida)");
        assert_eq!(
            api_status_text(&ApiSession::Expired, false),
            "Sesión de API vencida - volvé a iniciar sesión"
        );
    }
}
