//! AppReestr: the installed applications registry for Arch Linux.

mod details;
#[cfg(debug_assertions)]
mod devtools;
mod dialogs;
mod format;
mod icons;
mod prefs;
mod removal;
mod row;
mod theme;
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
    appreestr_core::i18n::init(&localedir());
    let app = RelmApp::new(APP_ID).with_args(Vec::new());
    relm4::set_global_css(include_str!("style.css"));
    app.run::<window::AppModel>(());
}
