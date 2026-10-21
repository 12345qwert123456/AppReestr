//! Light/dark appearance: follow the system unless the user forced one.

use appreestr_core::config::Theme;
use relm4::adw;

pub fn apply(theme: Theme) {
    let scheme = match theme {
        Theme::System => adw::ColorScheme::Default,
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}
