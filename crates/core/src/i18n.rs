//! Translations through gettext (English msgids, catalogs in `po/`).
//!
//! The interface language is the user's choice, English by default. It is applied before GTK
//! starts by pointing the process locale at it, so the application's own strings and the ones
//! GTK and libadwaita ship (dialog buttons, file chooser, dates) come out in the same language.
//! Any language with a catalog in the locale directory is offered, with no code changes.

use std::path::Path;
use std::process::Command;

pub const DOMAIN: &str = "appreestr";

/// The language of the msgids, which needs no catalog.
pub const SOURCE_LANGUAGE: &str = "en";

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

/// A language the interface can be shown in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    pub code: String,
    /// The language's name in itself, as it is shown in the picker.
    pub name: String,
}

/// English plus every language that has a catalog in `localedir`.
pub fn available(localedir: &Path) -> Vec<Language> {
    let mut codes: Vec<String> = std::fs::read_dir(localedir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("LC_MESSAGES").join(format!("{DOMAIN}.mo")).is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|c| c != SOURCE_LANGUAGE)
        .collect();
    codes.sort();
    std::iter::once(SOURCE_LANGUAGE.to_string())
        .chain(codes)
        .map(|code| Language { name: autonym(&code), code })
        .collect()
}

/// A language's own name. Unlisted codes are shown as they are.
fn autonym(code: &str) -> String {
    let base = code.split(['_', '@']).next().unwrap_or(code);
    let name = match (code, base) {
        ("pt_BR", _) => "Português (Brasil)",
        ("zh_CN", _) => "中文（简体）",
        ("zh_TW", _) => "中文（繁體）",
        (_, "ar") => "العربية",
        (_, "bg") => "Български",
        (_, "cs") => "Čeština",
        (_, "da") => "Dansk",
        (_, "de") => "Deutsch",
        (_, "el") => "Ελληνικά",
        (_, "en") => "English",
        (_, "es") => "Español",
        (_, "fi") => "Suomi",
        (_, "fr") => "Français",
        (_, "he") => "עברית",
        (_, "hu") => "Magyar",
        (_, "id") => "Bahasa Indonesia",
        (_, "it") => "Italiano",
        (_, "ja") => "日本語",
        (_, "ko") => "한국어",
        (_, "nl") => "Nederlands",
        (_, "pl") => "Polski",
        (_, "pt") => "Português",
        (_, "ro") => "Română",
        (_, "ru") => "Русский",
        (_, "sv") => "Svenska",
        (_, "tr") => "Türkçe",
        (_, "uk") => "Українська",
        (_, "zh") => "中文",
        _ => return code.to_string(),
    };
    name.to_string()
}

static ACTIVE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The language this run was started in (a change in the preferences applies on restart).
pub fn active() -> &'static str {
    ACTIVE.get().map_or(SOURCE_LANGUAGE, String::as_str)
}

/// Make the process speak `language` and bind the catalog directory.
///
/// Call once at startup, before GTK and before other threads are spawned: it changes the
/// environment, which GTK reads when it starts, and setlocale is not thread-safe.
pub fn init(localedir: &Path, language: &str) {
    let _ = ACTIVE.set(language.to_string());
    let locale = pick_locale(language);
    // SAFETY: called at startup while the process is single-threaded.
    unsafe {
        // LC_ALL beats whatever the session exports; LANGUAGE is what gettext consults first
        // (it works with any locale but plain "C", so `pick_locale` never returns that).
        std::env::set_var("LC_ALL", &locale);
        std::env::set_var("LANGUAGE", language);
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    let _ = gettextrs::bindtextdomain(DOMAIN, localedir);
    let _ = gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8");
}

/// A locale installed on this system that fits `language`: the session's own when it already
/// is that language, else the first installed one of the language, else `C.UTF-8`.
fn pick_locale(language: &str) -> String {
    let usable = |l: &str| !l.is_empty() && has_language(l, language) && try_locale(l);
    let session =
        ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty());
    if let Some(l) = session.filter(|l| usable(l)) {
        return l;
    }
    let mut installed: Vec<String> = Command::new("locale")
        .arg("-a")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect())
        .unwrap_or_default();
    // Prefer the language's main region (ru_RU, de_DE, en_US) and UTF-8 spellings.
    let region = if language == "en" { "US".to_string() } else { language.to_uppercase() };
    let main_region = |l: &String| l.starts_with(&format!("{language}_{region}"));
    installed.sort_by_key(|l| (!main_region(l), !l.to_lowercase().contains("utf")));
    installed
        .into_iter()
        .find(|l| l.to_lowercase().contains("utf") && usable(l))
        .unwrap_or_else(|| "C.UTF-8".to_string())
}

/// Whether the locale name (`ru_RU.UTF-8`) is for the language code (`ru`, `pt_BR`).
fn has_language(locale: &str, language: &str) -> bool {
    let name = locale.split(['.', '@']).next().unwrap_or("");
    name == language || name.split('_').next() == language.split('_').next() && !language.contains('_')
}

fn try_locale(name: &str) -> bool {
    // SAFETY: only called from `init`, at startup while the process is single-threaded.
    unsafe { gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, name).is_some() }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_names_match_language_codes() {
        assert!(has_language("ru_RU.UTF-8", "ru"));
        assert!(has_language("en_US.utf8", "en"));
        assert!(has_language("pt_BR.UTF-8", "pt_BR"));
        assert!(!has_language("pt_PT.UTF-8", "pt_BR"));
        assert!(!has_language("C", "en"));
        assert!(!has_language("ru_RU.UTF-8", "en"));
    }

    #[test]
    fn picker_lists_catalogs_found_on_disk() {
        let dir = std::env::temp_dir().join(format!("appreestr-loc-{}", std::process::id()));
        for l in ["ru", "de", "xx"] {
            let d = dir.join(l).join("LC_MESSAGES");
            std::fs::create_dir_all(&d).unwrap();
            if l != "xx" {
                std::fs::write(d.join("appreestr.mo"), b"").unwrap();
            }
        }
        let langs = available(&dir);
        let codes: Vec<_> = langs.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(codes, ["en", "de", "ru"]);
        assert_eq!(langs[0].name, "English");
        assert_eq!(langs[2].name, "Русский");
        let _ = std::fs::remove_dir_all(dir);
    }
}
