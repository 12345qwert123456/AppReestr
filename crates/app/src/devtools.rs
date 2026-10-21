//! Debug builds only: render the window to a PNG and quit, for UI checks without a screen.
//!
//! `APPREESTR_SCREENSHOT=/tmp/shot.png` takes the picture once the first scan has finished.
//! `APPREESTR_SCREENSHOT_ACTION` runs one step first: `details:<app id>`, `search:<text>`,
//! `uninstall:<app id>`, `system`, `console`, `preferences`, `select:<id>,<id>…`, `bulk:<id>,<id>…`, `danger`,
//! `force:<package>`, `admin`.

use crate::window::{AppModel, Msg};
use relm4::ComponentSender;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, glib, gsk};
use std::time::Duration;

pub fn on_first_scan(root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let Ok(path) = std::env::var("APPREESTR_SCREENSHOT") else { return };
    if let Ok(action) = std::env::var("APPREESTR_SCREENSHOT_ACTION") {
        let (kind, arg) = action.split_once(':').unwrap_or((action.as_str(), ""));
        match kind {
            "details" => sender.input(Msg::Details(arg.to_string())),
            "search" => sender.input(Msg::Search(arg.to_string())),
            "uninstall" => sender.input(Msg::Uninstall(arg.to_string())),
            "system" => sender.input(Msg::ShowSystem(true)),
            "console" => sender.input(Msg::ShowConsole(true)),
            "preferences" => sender.input(Msg::Preferences),
            "select" => {
                sender.input(Msg::SelectApps(arg.split(',').filter(|s| !s.is_empty()).map(String::from).collect()))
            }
            "bulk" => {
                sender.input(Msg::SelectApps(arg.split(',').filter(|s| !s.is_empty()).map(String::from).collect()));
                sender.input(Msg::UninstallMany);
            }
            "danger" => sender.input(Msg::DangerZone),
            "force" => sender.input(Msg::ForcePlan(arg.to_string())),
            "admin" => sender.input(Msg::AskElevate),
            _ => {}
        }
    }
    let root = root.clone();
    let delay = std::env::var("APPREESTR_SCREENSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(1500);
    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        // Dialogs are separate surfaces in libadwaita only when floating; render the window and
        // any visible dialog on top of it.
        let w = root.width();
        let h = root.height();
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&root)).snapshot(&snapshot, w as f64, h as f64);
        if let Some(node) = snapshot.to_node() {
            let renderer = gsk::CairoRenderer::new();
            if renderer.realize(None::<&gdk::Surface>).is_ok() {
                let texture = renderer.render_texture(&node, None);
                if let Err(e) = texture.save_to_png(&path) {
                    eprintln!("screenshot: {e}");
                }
                renderer.unrealize();
            }
        }
        relm4::main_application().quit();
    });
}
