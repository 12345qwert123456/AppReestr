//! Resolving launcher icons: theme names, absolute paths and per-source fallbacks.

use appreestr_core::model::{App, Visibility};
use appreestr_core::sysroot::SysRoot;
use relm4::gtk::{self, gdk};
use std::path::Path;

/// Make icons from Flatpak exports and our AppImage cache visible to the icon theme.
pub fn setup(sys: &SysRoot) {
    let Some(display) = gdk::Display::default() else { return };
    let theme = gtk::IconTheme::for_display(&display);
    for dir in &sys.data_dirs {
        let icons = dir.join("icons");
        if icons.is_dir() && !theme.search_path().contains(&icons) {
            theme.add_search_path(&icons);
        }
    }
    let cache = sys.cache_home.join("appreestr/icons");
    if cache.is_dir() {
        theme.add_search_path(&cache);
    }
}

fn fallback(app: &App) -> &'static str {
    match app.visibility {
        Visibility::App => "application-x-executable",
        Visibility::Console => "utilities-terminal",
        Visibility::Addon => "application-x-addon",
        Visibility::System => "applications-system-symbolic",
    }
}

/// Show the app's icon in `image`.
pub fn set(image: &gtk::Image, app: &App) {
    let Some(icon) = app.icon.as_deref().filter(|i| !i.is_empty()) else {
        image.set_icon_name(Some(fallback(app)));
        return;
    };
    let path = Path::new(icon);
    if path.is_absolute() {
        if path.is_file() {
            image.set_from_file(Some(path));
        } else {
            image.set_icon_name(Some(fallback(app)));
        }
        return;
    }
    // Some launchers name the icon with an extension (`foo.png`).
    let name = icon
        .strip_suffix(".png")
        .or_else(|| icon.strip_suffix(".svg"))
        .or_else(|| icon.strip_suffix(".xpm"))
        .unwrap_or(icon);
    let has = gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name));
    image.set_icon_name(Some(if has { name } else { fallback(app) }));
}
