//! Appearance: light/dark scheme and window transparency.
//!
//! Themes such as Noctalia's or Ambxst's write a `~/.config/gtk-4.0/gtk.css` that hard-codes the
//! named colours (always one scheme), so libadwaita's own light/dark switch changes nothing. When the
//! user forces a scheme we therefore load a provider one step above the user's CSS that restates the
//! libadwaita palette of that scheme. "Follow the system" leaves the user's theme alone.

use appreestr_core::config::{MAX_TRANSPARENCY, Settings, Theme};
use relm4::adw;
use relm4::gtk::{self, gdk};
use std::cell::RefCell;
use std::fmt::Write;

/// The libadwaita palette: (name with underscores, value).
type Palette = &'static [(&'static str, &'static str)];

const LIGHT: Palette = &[
    ("accent_color", "#1a5fb4"),
    ("accent_bg_color", "#3584e4"),
    ("accent_fg_color", "#ffffff"),
    ("destructive_color", "#c01c28"),
    ("destructive_bg_color", "#e01b24"),
    ("destructive_fg_color", "#ffffff"),
    ("success_color", "#1b8553"),
    ("success_bg_color", "#2ec27e"),
    ("success_fg_color", "#ffffff"),
    ("warning_color", "#9c6e03"),
    ("warning_bg_color", "#e5a50a"),
    ("warning_fg_color", "rgba(0, 0, 0, 0.8)"),
    ("error_color", "#c01c28"),
    ("error_bg_color", "#e01b24"),
    ("error_fg_color", "#ffffff"),
    ("window_bg_color", "#fafafb"),
    ("window_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("view_bg_color", "#ffffff"),
    ("view_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("headerbar_bg_color", "#ffffff"),
    ("headerbar_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("headerbar_border_color", "rgba(0, 0, 6, 0.8)"),
    ("headerbar_backdrop_color", "#fafafb"),
    ("popover_bg_color", "#ffffff"),
    ("popover_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("card_bg_color", "#ffffff"),
    ("card_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("dialog_bg_color", "#fafafb"),
    ("dialog_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("sidebar_bg_color", "#ebebed"),
    ("sidebar_fg_color", "rgba(0, 0, 6, 0.8)"),
    ("sidebar_backdrop_color", "#f2f2f4"),
    ("sidebar_border_color", "rgba(0, 0, 0, 0.07)"),
    ("shade_color", "rgba(0, 0, 0, 0.07)"),
];

const DARK: Palette = &[
    ("accent_color", "#78aeed"),
    ("accent_bg_color", "#3584e4"),
    ("accent_fg_color", "#ffffff"),
    ("destructive_color", "#ff7b63"),
    ("destructive_bg_color", "#c01c28"),
    ("destructive_fg_color", "#ffffff"),
    ("success_color", "#8ff0a4"),
    ("success_bg_color", "#26a269"),
    ("success_fg_color", "#ffffff"),
    ("warning_color", "#f8e45c"),
    ("warning_bg_color", "#cd9309"),
    ("warning_fg_color", "rgba(0, 0, 0, 0.8)"),
    ("error_color", "#ff7b63"),
    ("error_bg_color", "#c01c28"),
    ("error_fg_color", "#ffffff"),
    ("window_bg_color", "#222226"),
    ("window_fg_color", "#ffffff"),
    ("view_bg_color", "#1d1d20"),
    ("view_fg_color", "#ffffff"),
    ("headerbar_bg_color", "#2e2e32"),
    ("headerbar_fg_color", "#ffffff"),
    ("headerbar_border_color", "#ffffff"),
    ("headerbar_backdrop_color", "#222226"),
    ("popover_bg_color", "#36363a"),
    ("popover_fg_color", "#ffffff"),
    ("card_bg_color", "rgba(255, 255, 255, 0.08)"),
    ("card_fg_color", "#ffffff"),
    ("dialog_bg_color", "#36363a"),
    ("dialog_fg_color", "#ffffff"),
    ("sidebar_bg_color", "#2e2e32"),
    ("sidebar_fg_color", "#ffffff"),
    ("sidebar_backdrop_color", "#28282c"),
    ("sidebar_border_color", "rgba(255, 255, 255, 0.07)"),
    ("shade_color", "rgba(0, 0, 0, 0.36)"),
];

thread_local! {
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

fn palette_css(p: Palette) -> String {
    let mut css = String::new();
    let mut vars = String::new();
    for (name, value) in p {
        let _ = writeln!(css, "@define-color {name} {value};");
        let _ = writeln!(vars, "  --{}: {value};", name.replace('_', "-"));
    }
    format!("{css}:root {{\n{vars}}}\n")
}

/// The stylesheet for the settings; empty when nothing needs overriding.
fn build_css(settings: &Settings) -> String {
    let mut css = match settings.theme {
        Theme::System => String::new(),
        Theme::Light => palette_css(LIGHT),
        Theme::Dark => palette_css(DARK),
    };
    let transparency = settings.transparency.min(MAX_TRANSPARENCY);
    if transparency > 0 {
        let alpha = 1.0 - f64::from(transparency) / 100.0;
        // Only the main window's surfaces: dialogs stay opaque so text on them stays readable.
        let _ = write!(
            css,
            "window.translucent {{ background-color: alpha(@window_bg_color, {alpha:.2}); }}\n\
             toolbarview.translucent > .top-bar.raised,\n\
             toolbarview.translucent > .top-bar.raised:backdrop {{ background-color: alpha(@headerbar_bg_color, {alpha:.2}); }}\n\
             toolbarview.translucent listview, toolbarview.translucent .view {{ background-color: transparent; }}\n"
        );
    }
    css
}

pub fn apply(settings: &Settings) {
    let scheme = match settings.theme {
        Theme::System => adw::ColorScheme::Default,
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
    };
    adw::StyleManager::default().set_color_scheme(scheme);

    let css = build_css(settings);
    PROVIDER.with(|cell| {
        let mut slot = cell.borrow_mut();
        let provider = slot.get_or_insert_with(|| {
            let provider = gtk::CssProvider::new();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
            }
            provider
        });
        provider.load_from_string(&css);
    });
}
