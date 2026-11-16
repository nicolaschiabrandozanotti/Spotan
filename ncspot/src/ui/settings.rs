use std::sync::Arc;

use cursive::Cursive;
use cursive::traits::Nameable;
use cursive::view::ViewWrapper;
use cursive::views::{Button, DummyView, EditView, LinearLayout, TextView};
use librespot_core::authentication::Credentials as RespotCredentials;

use crate::application::UserData;
use crate::authentication::{self, ApiSession};
use crate::command::Command;
use crate::traits::ViewExt;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsMode {
    /// A regular screen inside the main layout.
    Screen,
    /// Standalone startup screen: only the Spotify login is offered.
    Start,
}

const MSG: &str = "settings_msg";
const API_STATUS: &str = "settings_api_status";
const API_ID: &str = "settings_api_id";
const BTN_SPOTIFY: &str = "settings_btn_spotify";
const BTN_API: &str = "settings_btn_api";
const BTN_SHARED: &str = "settings_btn_shared";

pub struct SettingsView {
    inner: LinearLayout,
}

fn api_status_text(session: &ApiSession, own_app: bool) -> &'static str {
    match (session, own_app) {
        (ApiSession::None, _) => "sin sesión",
        (ApiSession::Active, true) => "activa (app propia)",
        (ApiSession::Active, false) => "activa (app compartida)",
        (ApiSession::Expired, _) => "Sesión de API vencida — volvé a iniciar sesión",
    }
}

fn current_api_status() -> &'static str {
    api_status_text(&authentication::api_session(), authentication::api_client_id().is_some())
}

fn set_text(s: &mut Cursive, name: &str, text: String) {
    s.call_on_name(name, |v: &mut TextView| v.set_content(text));
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
    set_text(s, MSG, "Esperando el navegador…".into());
    let sink = s.cb_sink().clone();
    std::thread::spawn(move || {
        let url_sink = sink.clone();
        let result = login(&|url| {
            let url = url.to_string();
            let _ = url_sink.send(Box::new(move |s| {
                set_text(s, MSG, format!("Esperando el navegador…\n{url}"));
            }));
        });
        let _ = sink.send(Box::new(move |s| {
            set_enabled(s, true);
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
        on_logged_in: Box<dyn Fn(RespotCredentials) + Send + Sync>,
    ) -> Self {
        let on_logged_in: Arc<dyn Fn(RespotCredentials) + Send + Sync> = Arc::from(on_logged_in);
        let mut layout = LinearLayout::vertical().child(TextView::new("Ajustes"));
        if mode == SettingsMode::Start {
            layout.add_child(TextView::new("Iniciá sesión para empezar"));
        }
        layout.add_child(DummyView);
        layout.add_child(TextView::new(format!(
            "Cuenta de Spotify: {}",
            account.as_deref().unwrap_or("sin sesión")
        )));
        let logged_in = account.is_some();
        layout.add_child(
            Button::new(if logged_in { "Cerrar sesión" } else { "Iniciar sesión en Spotify" }, move |s| {
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
                        |on_url| authentication::create_credentials_with(on_url),
                        move |s, result| match result {
                            Ok(credentials) => {
                                set_text(s, MSG, "Sesión iniciada".into());
                                // May block (startup flow): keep it off the UI thread.
                                std::thread::spawn(move || on_logged_in(credentials));
                            }
                            Err(e) => set_text(s, MSG, format!("Error: {e}")),
                        },
                    );
                }
            })
            .with_name(BTN_SPOTIFY),
        );
        if mode == SettingsMode::Screen || logged_in {
            layout.add_child(DummyView);
            layout.add_child(TextView::new("API oficial (opcional)"));
            layout.add_child(
                LinearLayout::horizontal()
                    .child(TextView::new("Client ID: "))
                    .child(
                        EditView::new()
                            .content(authentication::api_client_id().unwrap_or_default())
                            .with_name(API_ID),
                    ),
            );
            layout.add_child(
                LinearLayout::horizontal()
                    .child(TextView::new("Estado: "))
                    .child(TextView::new(current_api_status()).with_name(API_STATUS)),
            );
            layout.add_child(
                Button::new("Iniciar sesión en la API", |s| {
                    if let Err(e) = save_client_id(s) {
                        return set_text(s, MSG, format!("Error: {e}"));
                    }
                    start_login(
                        s,
                        |on_url| authentication::api_login(on_url),
                        |s, result| {
                            refresh_api(s);
                            set_text(s, MSG, match result {
                                Ok(()) => "Sesión de API iniciada".into(),
                                Err(e) => format!("Error: {e}"),
                            });
                        },
                    );
                })
                .with_name(BTN_API),
            );
            layout.add_child(
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
            );
            layout.add_child(TextView::new(
                "Cómo sacarla: developer.spotify.com → Create app → Redirect URI: http://127.0.0.1/ncspot_login → copiá el Client ID",
            ));
        }
        layout.add_child(DummyView);
        layout.add_child(TextView::new("").with_name(MSG));
        Self { inner: layout }
    }
}

impl ViewWrapper for SettingsView {
    cursive::wrap_impl!(self.inner: LinearLayout);
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
    fn settings_api_status_text() {
        assert_eq!(api_status_text(&ApiSession::None, true), "sin sesión");
        assert_eq!(api_status_text(&ApiSession::Active, true), "activa (app propia)");
        assert_eq!(api_status_text(&ApiSession::Active, false), "activa (app compartida)");
        assert_eq!(
            api_status_text(&ApiSession::Expired, false),
            "Sesión de API vencida — volvé a iniciar sesión"
        );
    }
}
