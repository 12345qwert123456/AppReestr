//! Small presentation helpers.

use relm4::gtk::glib;
use std::path::Path;

/// Locale-aware short date (`21 окт. 2026`).
pub fn date(ts: i64) -> String {
    glib::DateTime::from_unix_local(ts)
        .ok()
        .and_then(|d| d.format("%e %b %Y").ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `~/Applications/Foo.AppImage` instead of the full home path.
pub fn path(p: &Path) -> String {
    if let Some(home) = glib::home_dir().to_str()
        && let Ok(rest) = p.strip_prefix(home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

pub fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}
