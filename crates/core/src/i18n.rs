//! Translations through gettext (English msgids, catalogs in `po/`).

use std::path::Path;

pub const DOMAIN: &str = "appreestr";

/// Translate a message.
pub fn tr(msgid: &str) -> String {
    gettextrs::dgettext(DOMAIN, msgid)
}

/// Translate a message and substitute `{}` placeholders in order.
pub fn trf(msgid: &str, args: &[&str]) -> String {
    let mut out = tr(msgid);
    for a in args {
        if let Some(i) = out.find("{}") {
            out.replace_range(i..i + 2, a);
        }
    }
    out
}

/// Plural-aware translation; `{}` is replaced by `n`.
pub fn trn(singular: &str, plural: &str, n: u64) -> String {
    gettextrs::dngettext(DOMAIN, singular, plural, n as u32).replacen("{}", &n.to_string(), 1)
}

/// Set the locale from the environment and bind the catalog directory.
///
/// Call once at startup, before other threads are spawned (setlocale is not thread-safe).
pub fn init(localedir: &Path) {
    // SAFETY: called at startup while the process is single-threaded.
    unsafe {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    let _ = gettextrs::bindtextdomain(DOMAIN, localedir);
    let _ = gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8");
}

/// The locale's decimal separator (`,` in Russian), read once after [`init`].
pub fn decimal_point() -> char {
    static POINT: std::sync::OnceLock<char> = std::sync::OnceLock::new();
    *POINT.get_or_init(|| {
        // SAFETY: localeconv returns a pointer to static data; we copy one byte right away.
        unsafe {
            let lc = libc::localeconv();
            if lc.is_null() || (*lc).decimal_point.is_null() {
                return '.';
            }
            let c = *(*lc).decimal_point as u8;
            if c.is_ascii_punctuation() { c as char } else { '.' }
        }
    })
}
