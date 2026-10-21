//! AppReestr: the installed applications registry for Arch Linux.

mod danger;
mod details;
#[cfg(debug_assertions)]
mod devtools;
mod dialogs;
mod elevate;
mod format;
mod icons;
mod prefs;
mod removal;
mod row;
mod theme;
mod ui;
mod uninstall;
mod watcher;
mod window;

use appreestr_core::APP_ID;
use relm4::RelmApp;
use std::path::PathBuf;

/// Translations: the build directory during development, the system locale dir when installed.
fn localedir() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(dev) = option_env!("APPREESTR_DEV_LOCALEDIR")
    {
        return PathBuf::from(dev);
    }
    PathBuf::from(option_env!("APPREESTR_LOCALEDIR").unwrap_or("/usr/share/locale"))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,relm4=warn".into()),
        )
        .init();
    // The privileged copy takes its display and language from the arguments, before GTK starts.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let notify_ready = elevate::adopt_args(&args);
    appreestr_core::i18n::init(&localedir());
    let app = RelmApp::new(APP_ID).with_args(Vec::new());
    if elevate::is_root() {
        // The unprivileged copy is still running when this one starts.
        app.allow_multiple_instances(true);
    }
    relm4::set_global_css(include_str!("style.css"));
    app.run::<window::AppModel>(notify_ready);
}
