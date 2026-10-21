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
use appreestr_core::config::ConfigStore;
use appreestr_core::sysroot::SysRoot;
use relm4::RelmApp;
use relm4::gtk::gio::prelude::ApplicationExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Translations: the build directory during development, the system locale dir when installed.
pub fn localedir() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(dev) = option_env!("APPREESTR_DEV_LOCALEDIR")
    {
        return PathBuf::from(dev);
    }
    PathBuf::from(option_env!("APPREESTR_LOCALEDIR").unwrap_or("/usr/share/locale"))
}

/// Set by the preferences: start the application again once this run has ended.
static RESTART: AtomicBool = AtomicBool::new(false);

/// Close the application and start it again (to apply a new language).
pub fn restart() {
    RESTART.store(true, Ordering::Relaxed);
    relm4::main_application().quit();
}

/// Replace this process with a fresh copy of itself. Only returns if that failed.
fn exec_self() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/proc/self/exe"));
    // After an upgrade the running binary is reported as "… (deleted)".
    let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    // The administrator copy has already told its parent that it is ready.
    let args = std::env::args().skip(1).filter(|a| a != elevate::NOTIFY_READY);
    std::process::Command::new(exe).args(args).exec()
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
    // The language is chosen in the preferences and must be in place before GTK starts.
    let settings = ConfigStore::new(&SysRoot::host()).settings();
    appreestr_core::i18n::init(&localedir(), &settings.language);
    let app = RelmApp::new(APP_ID).with_args(Vec::new());
    if elevate::is_root() {
        // The unprivileged copy is still running when this one starts.
        app.allow_multiple_instances(true);
    }
    relm4::set_global_css(include_str!("style.css"));
    app.run::<window::AppModel>(notify_ready);
    if RESTART.load(Ordering::Relaxed) {
        eprintln!("restart failed: {}", exec_self());
    }
}
