use std::sync::Arc;

use cursive::event::{Event, EventResult, EventTrigger, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, PaletteColor};
use cursive::traits::{Nameable, Resizable};
use cursive::utils::markup::StyledString;
use cursive::view::{View, ViewWrapper};
use cursive::{Cursive, Printer, Rect, Vec2};
use cursive::views::{Button, Checkbox, DummyView, EditView, LinearLayout, NamedView, OnEventView, TextView};
use librespot_core::authentication::Credentials as RespotCredentials;

use crate::application::UserData;
use crate::authentication::{self, ApiSession};
use crate::command::Command;
use crate::external::sc_library::{self, ScUser};
use crate::external::yt_library;
use crate::model::track::Source;
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
const BTN_ENTER: &str = "settings_btn_enter";
const SC_PROFILE: &str = "settings_sc_profile";
const SC_STATUS: &str = "settings_sc_status";
const BTN_SC_LOAD: &str = "settings_btn_sc_load";
const BTN_SC_REMOVE: &str = "settings_btn_sc_remove";
const YT_LINK: &str = "settings_yt_link";
const YT_STATUS: &str = "settings_yt_status";
const YT_LIST: &str = "settings_yt_list";
const BTN_YT_ADD: &str = "settings_btn_yt_add";
const BTN_YT_REMOVE: &str = "settings_btn_yt_remove_";

/// Marker left in the start screen's user data by "Entrar sin Spotify".
pub struct EnterWithoutSpotify;

/// Cambio de SoundCloud o YouTube que la aplicación debe reflejar en la biblioteca.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibraryChange {
    /// Se cargó un perfil: bajar su biblioteca.
    ScLoaded,
    /// Se quitó el perfil: vaciar lo de SoundCloud y su caché.
    ScRemoved,
    /// Se prendió o apagó el servicio: recalcular las vistas.
    ScToggled,
    /// Cambió la lista de links de YouTube: volver a bajarla (o vaciarla si no quedan).
    YtLinksChanged,
    /// Se prendió o apagó YouTube: recalcular las vistas.
    YtToggled,
}

/// "Entrar sin Spotify" needs at least one external service on.
pub fn can_enter_without_spotify(services: &[Source]) -> bool {
    !services.is_empty()
}

/// A service checkbox: toggling it saves right away and refreshes the enter button.
fn service_checkbox(source: Source, label: &str, on_sc_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) -> LinearLayout {
    let on = crate::external::enabled().contains(&source);
    LinearLayout::horizontal()
        .child(Checkbox::new().with_checked(on).on_change(move |s, on| {
            match crate::external::set_enabled(source, on) {
                Ok(()) => set_text(s, MSG, String::new()),
                Err(e) => set_text(s, MSG, format!("Error: {e}")),
            }
            let can = can_enter_without_spotify(&crate::external::enabled());
            s.call_on_name(BTN_ENTER, |b: &mut Button| b.set_enabled(can));
            if source == Source::SoundCloud {
                set_sc_enabled(s, on);
                on_sc_change(LibraryChange::ScToggled);
            }
            if source == Source::YouTube {
                set_yt_enabled(s, on);
                on_sc_change(LibraryChange::YtToggled);
            }
        }))
        .child(TextView::new(format!(" {label}")))
}

/// The profile field and its buttons only work while SoundCloud is on.
fn set_sc_enabled(s: &mut Cursive, on: bool) {
    s.call_on_name(SC_PROFILE, |v: &mut EditView| v.set_enabled(on));
    for name in [BTN_SC_LOAD, BTN_SC_REMOVE] {
        s.call_on_name(name, |b: &mut Button| b.set_enabled(on));
    }
}

/// El campo de links y sus botones solo funcionan con YouTube prendido.
fn set_yt_enabled(s: &mut Cursive, on: bool) {
    s.call_on_name(YT_LINK, |v: &mut EditView| v.set_enabled(on));
    s.call_on_name(BTN_YT_ADD, |b: &mut Button| b.set_enabled(on));
    for i in 0..yt_library::links().len() {
        s.call_on_name(&format!("{BTN_YT_REMOVE}{i}"), |b: &mut Button| b.set_enabled(on));
    }
}

/// Links resultantes de sumar `input` a `current`, con el link canónico al final.
fn add_link(current: &[String], input: &str) -> Result<Vec<String>, String> {
    let link = yt_library::parse_link(input)
        .map(|l| yt_library::link_url(&l))
        .ok_or("Error: Eso no parece un link de playlist o canal de YouTube")?;
    if current.contains(&link) {
        return Err("Ya está agregado".into());
    }
    Ok(current.iter().cloned().chain([link]).collect())
}

/// Vuelve a dibujar la lista de links con un «Quitar» por fila.
fn rebuild_yt_list(s: &mut Cursive, on_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) {
    s.call_on_name(YT_LIST, |l: &mut LinearLayout| {
        l.clear();
        fill_yt_list(l, on_change);
    });
}

fn fill_yt_list(l: &mut LinearLayout, on_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) {
    let on = crate::external::enabled().contains(&Source::YouTube);
    for (i, link) in yt_library::links().into_iter().enumerate() {
        let cb = on_change.clone();
        let mut remove = Button::new("Quitar", move |s| remove_yt_link(s, i, cb.clone()));
        remove.set_enabled(on);
        l.add_child(
            LinearLayout::horizontal()
                .child(TextView::new(format!("      {link} ")))
                .child(remove.with_name(format!("{BTN_YT_REMOVE}{i}"))),
        );
    }
}

fn remove_yt_link(s: &mut Cursive, index: usize, on_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) {
    let mut links = yt_library::links();
    if index >= links.len() {
        return;
    }
    links.remove(index);
    if let Err(e) = yt_library::set_links(&links) {
        return set_text(s, YT_STATUS, format!("Error: {e}"));
    }
    if links.is_empty() {
        // en la pantalla de inicio no hay Library que lo haga
        crate::library::remove_yt_cache();
    }
    set_text(s, YT_STATUS, String::new());
    rebuild_yt_list(s, on_change.clone());
    on_change(LibraryChange::YtLinksChanged);
}

fn add_yt_link(s: &mut Cursive, on_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) {
    let input = s.call_on_name(YT_LINK, |v: &mut EditView| v.get_content()).unwrap_or_default();
    let updated = match add_link(&yt_library::links(), &input) {
        Ok(u) => u,
        Err(e) => return set_text(s, YT_STATUS, e),
    };
    let link = updated.last().cloned().unwrap_or_default();
    set_text(s, YT_STATUS, format!("Cargando{}", crate::utils::glyph("…", "...")));
    s.call_on_name(BTN_YT_ADD, |b: &mut Button| b.set_enabled(false));
    let sink = s.cb_sink().clone();
    std::thread::spawn(move || {
        let result = yt_library::fetch_link(&link);
        let _ = sink.send(Box::new(move |s| {
            let still_on = crate::external::enabled().contains(&Source::YouTube);
            s.call_on_name(BTN_YT_ADD, |b: &mut Button| b.set_enabled(still_on));
            // Solo se guarda un link que cargó; se relee la lista por si cambió mientras tanto.
            let saved = result.and_then(|c| {
                yt_library::set_links(&add_link(&yt_library::links(), &link)?)?;
                Ok(c)
            });
            match saved {
                Ok(c) => {
                    s.call_on_name(YT_LINK, |v: &mut EditView| v.set_content(""));
                    set_text(s, YT_STATUS, format!("{} ({})", c.title, c.summary));
                    rebuild_yt_list(s, on_change.clone());
                    on_change(LibraryChange::YtLinksChanged);
                }
                Err(e) => set_text(s, YT_STATUS, if e.starts_with("Error") { e } else { format!("Error: {e}") }),
            }
        }));
    });
}

fn sc_status_text(r: &Result<ScUser, String>) -> String {
    match r {
        Ok(u) => format!("{} · {} Likes · {} playlists", u.username, u.likes, u.playlists),
        Err(e) => format!("Error: {e}"),
    }
}

fn load_sc_profile(s: &mut Cursive, on_change: Arc<dyn Fn(LibraryChange) + Send + Sync>) {
    let input = s.call_on_name(SC_PROFILE, |v: &mut EditView| v.get_content()).unwrap_or_default();
    let Some(url) = sc_library::normalize_profile(&input) else {
        return set_text(s, SC_STATUS, "Error: Eso no parece un perfil de SoundCloud".into());
    };
    set_text(s, SC_STATUS, format!("Cargando{}", crate::utils::glyph("…", "...")));
    for name in [BTN_SC_LOAD, BTN_SC_REMOVE] {
        s.call_on_name(name, |b: &mut Button| b.set_enabled(false));
    }
    let sink = s.cb_sink().clone();
    std::thread::spawn(move || {
        let result = sc_library::resolve(&url);
        let _ = sink.send(Box::new(move |s| {
            // SoundCloud pudo apagarse mientras cargaba.
            let still_on = crate::external::enabled().contains(&Source::SoundCloud);
            set_sc_enabled(s, still_on);
            // Only a profile that resolved gets saved.
            let previous = sc_library::profile();
            let result = result.and_then(|u| sc_library::set_profile(Some(&url)).map(|()| u));
            set_text(s, SC_STATUS, sc_status_text(&result));
            if result.is_ok() && still_on {
                // otro perfil: lo del anterior no se queda mostrando
                if previous.is_some_and(|p| p != url) {
                    // en la pantalla de inicio on_change no hace nada: la caché se borra acá
                    crate::library::remove_sc_cache();
                    on_change(LibraryChange::ScRemoved);
                }
                on_change(LibraryChange::ScLoaded);
            }
        }));
    });
}

fn remove_sc_profile(s: &mut Cursive, on_change: &dyn Fn(LibraryChange)) {
    match sc_library::set_profile(None) {
        Ok(()) => {
            // en la pantalla de inicio no hay Library que lo haga
            crate::library::remove_sc_cache();
            s.call_on_name(SC_PROFILE, |v: &mut EditView| v.set_content(""));
            set_text(s, SC_STATUS, String::new());
            on_change(LibraryChange::ScRemoved);
        }
        Err(e) => set_text(s, SC_STATUS, format!("Error: {e}")),
    }
}

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

#[cfg(feature = "share_clipboard")]
fn clipboard_text() -> Option<String> {
    crate::sharing::read_share().ok()
}

#[cfg(not(feature = "share_clipboard"))]
fn clipboard_text() -> Option<String> {
    None
}

/// Windows' classic console neither pastes into raw-mode apps nor lets mouse capture keep right-click.
fn is_paste(e: &Event) -> bool {
    matches!(e, Event::CtrlChar('v') | Event::Mouse { event: MouseEvent::Press(MouseButton::Right), .. })
}

// Replaces instead of inserting: the field only ever holds one Client ID.
fn paste_into(v: &mut NamedView<EditView>) -> Option<EventResult> {
    let text: String = clipboard_text()?.chars().filter(|c| !c.is_whitespace()).collect();
    Some(EventResult::Consumed(Some(v.get_mut().set_content(text))))
}

fn copy_redirect(s: &mut Cursive) {
    #[cfg(feature = "share_clipboard")]
    let msg = match crate::sharing::write_share(authentication::OWN_APP_REDIRECT.into()) {
        Ok(()) => "Redirect URI copiada: pegala en Spotify con Ctrl+V".to_string(),
        Err(e) => format!("Error: {e}"),
    };
    #[cfg(not(feature = "share_clipboard"))]
    let msg = format!("Redirect URI: {}", authentication::OWN_APP_REDIRECT);
    set_text(s, MSG, msg);
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
        on_sc_change: Box<dyn Fn(LibraryChange) + Send + Sync>,
    ) -> Self {
        let on_sc_change: Arc<dyn Fn(LibraryChange) + Send + Sync> = Arc::from(on_sc_change);
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
            layout.add_child(TextView::new(account.as_deref().unwrap_or("sin sesión de Spotify").to_string()));
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
                        move |s, result| match result {
                            // Start mode quits right after; keep the buttons disabled.
                            Ok(()) if start => set_text(s, MSG, "Sesión iniciada".into()),
                            // The credentials are saved: relaunch (without logging out) to enter with them.
                            Ok(()) => {
                                crate::restart::request_keep_session();
                                match s.user_data::<UserData>().cloned() {
                                    Some(data) => data.cmd.handle(s, Command::Quit),
                                    None => s.quit(),
                                }
                            }
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
        layout.add_child(DummyView);
        layout.add_child(TextView::new(heading("Otros servicios")));
        layout.add_child(service_checkbox(Source::YouTube, "YouTube", on_sc_change.clone()));
        let yt_on = crate::external::enabled().contains(&Source::YouTube);
        let add_cb = on_sc_change.clone();
        let mut add = Button::new("Agregar", move |s| add_yt_link(s, add_cb.clone()));
        add.set_enabled(yt_on);
        layout.add_child(
            LinearLayout::horizontal()
                .child(TextView::new("    Links: "))
                .child(
                    OnEventView::new(EditView::new().with_enabled(yt_on).with_name(YT_LINK))
                        .on_pre_event_inner(EventTrigger::from_fn(is_paste), |v, _| paste_into(v))
                        .max_width(CLIENT_ID_WIDTH),
                )
                .child(DummyView.fixed_width(1))
                .child(add.with_name(BTN_YT_ADD)),
        );
        let mut list = LinearLayout::vertical();
        fill_yt_list(&mut list, on_sc_change.clone());
        layout.add_child(list.with_name(YT_LIST));
        layout.add_child(TextView::new("").with_name(YT_STATUS));
        layout.add_child(TextView::new(hint(
            "    Pegá links de playlists o canales públicos. Ctrl+V o clic derecho para pegar.",
        )));
        layout.add_child(service_checkbox(Source::SoundCloud, "SoundCloud", on_sc_change.clone()));
        let sc_on = crate::external::enabled().contains(&Source::SoundCloud);
        let (load_cb, remove_cb) = (on_sc_change.clone(), on_sc_change.clone());
        let mut load = Button::new("Cargar", move |s| load_sc_profile(s, load_cb.clone()));
        let mut remove = Button::new("Quitar", move |s| remove_sc_profile(s, &*remove_cb));
        load.set_enabled(sc_on);
        remove.set_enabled(sc_on);
        layout.add_child(
            LinearLayout::horizontal()
                .child(TextView::new("    Perfil: "))
                .child(
                    OnEventView::new(
                        EditView::new()
                            .content(sc_library::profile().unwrap_or_default())
                            .with_enabled(sc_on)
                            .with_name(SC_PROFILE),
                    )
                    .on_pre_event_inner(EventTrigger::from_fn(is_paste), |v, _| paste_into(v))
                    .max_width(CLIENT_ID_WIDTH),
                ),
        );
        layout.add_child(
            LinearLayout::horizontal()
                .child(TextView::new("    "))
                .child(load.with_name(BTN_SC_LOAD))
                .child(DummyView.fixed_width(1))
                .child(remove.with_name(BTN_SC_REMOVE)),
        );
        layout.add_child(TextView::new("").with_name(SC_STATUS));
        if start {
            let mut enter = Button::new("Entrar sin Spotify", |s| {
                s.set_user_data(EnterWithoutSpotify);
                s.quit();
            });
            enter.set_enabled(can_enter_without_spotify(&crate::external::enabled()));
            layout.add_child(enter.with_name(BTN_ENTER));
        }
        if mode == SettingsMode::Screen || logged_in {
            layout.add_child(DummyView);
            layout.add_child(TextView::new(heading("API oficial (opcional)")));
            layout.add_child(
                LinearLayout::horizontal()
                    .child(TextView::new("Client ID: "))
                    .child(
                        OnEventView::new(
                            EditView::new()
                                .content(authentication::api_client_id().unwrap_or_default())
                                .with_name(API_ID),
                        )
                        .on_pre_event_inner(EventTrigger::from_fn(is_paste), |v, _| paste_into(v))
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
            layout.add_child(TextView::new(hint(&format!(
                "Cómo sacarla: Abrir developer.spotify.com > Create app > Redirect URI: {} (botón Copiar) > Save > copiá el Client ID y pegalo arriba con Ctrl+V o clic derecho",
                authentication::OWN_APP_REDIRECT
            ))));
            layout.add_child(
                LinearLayout::horizontal()
                    .child(Button::new("Abrir developer.spotify.com", |_| {
                        open::that_in_background("https://developer.spotify.com/dashboard");
                    }))
                    .child(DummyView.fixed_width(1))
                    .child(Button::new("Copiar Redirect URI", copy_redirect)),
            );
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
    fn paste_on_ctrl_v_or_right_click() {
        use cursive::XY;
        assert!(is_paste(&Event::CtrlChar('v')));
        assert!(!is_paste(&Event::Char('v')));
        let mouse = |b| Event::Mouse { offset: XY::zero(), position: XY::zero(), event: MouseEvent::Press(b) };
        assert!(is_paste(&mouse(MouseButton::Right)));
        assert!(!is_paste(&mouse(MouseButton::Left)));
    }

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
                runner.add_fullscreen_layer(SettingsView::new(mode, None, Box::new(|_| Ok(())), Box::new(|_| ())));
                runner.step();
                runner.refresh();
            }
        }
    }

    #[test]
    fn profile_status_text() {
        let ok = Ok(ScUser { id: 183, username: "Forss".into(), likes: 288, playlists: 4 });
        assert_eq!(sc_status_text(&ok), "Forss · 288 Likes · 4 playlists");
        assert_eq!(sc_status_text(&Err("No existe".into())), "Error: No existe");
    }

    #[test]
    fn add_link_validates_and_dedups() {
        let a = "https://www.youtube.com/@coldplay".to_string();
        assert_eq!(add_link(&[], "youtube.com/@coldplay"), Ok(vec![a.clone()]));
        let pl = "https://www.youtube.com/playlist?list=PLabc123";
        assert_eq!(add_link(&[a.clone()], pl), Ok(vec![a.clone(), pl.to_string()]));
        assert_eq!(add_link(&[a.clone()], "https://www.youtube.com/@coldplay"), Err("Ya está agregado".into()));
        assert_eq!(
            add_link(&[], "hola"),
            Err("Error: Eso no parece un link de playlist o canal de YouTube".into())
        );
    }

    #[test]
    fn start_enter_disabled_without_services() {
        assert!(!can_enter_without_spotify(&[]));
        assert!(can_enter_without_spotify(&[Source::YouTube]));
        assert!(can_enter_without_spotify(&[Source::SoundCloud, Source::YouTube]));
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
