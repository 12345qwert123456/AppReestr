//! Desktop entries (`.desktop` launchers) from every XDG applications directory.

use crate::model::DesktopEntry;
use crate::sysroot::SysRoot;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Parse a desktop entry. Returns `None` for files without a `[Desktop Entry]` group.
pub fn parse(content: &str, id: &str, path: &Path, locale: Option<&str>) -> Option<DesktopEntry> {
    let mut in_main = false;
    let mut seen_main = false;
    let mut kv: HashMap<&str, &str> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            seen_main |= in_main;
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim_end()).or_insert(v.trim_start());
        }
    }
    if !seen_main {
        return None;
    }

    let localized = |key: &str| -> Option<String> {
        if let Some(loc) = locale {
            if let Some(v) = kv.get(format!("{key}[{loc}]").as_str()) {
                return Some(unescape(v));
            }
            if let Some(lang) = loc.split('_').next()
                && let Some(v) = kv.get(format!("{key}[{lang}]").as_str())
            {
                return Some(unescape(v));
            }
        }
        kv.get(key).map(|v| unescape(v))
    };
    let string = |key: &str| kv.get(key).map(|v| unescape(v)).filter(|v| !v.is_empty());
    let boolean = |key: &str| kv.get(key).is_some_and(|v| v.trim() == "true");
    let list = |value: Option<String>| -> Vec<String> {
        value
            .map(|v| v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    };

    let name = localized("Name").unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
    Some(DesktopEntry {
        id: id.to_string(),
        path: path.to_path_buf(),
        name,
        generic_name: localized("GenericName").filter(|s| !s.is_empty()),
        comment: localized("Comment").filter(|s| !s.is_empty()),
        icon: string("Icon"),
        exec: kv.get("Exec").map(|v| v.to_string()).filter(|v| !v.is_empty()),
        try_exec: string("TryExec"),
        categories: list(string("Categories")),
        keywords: list(localized("Keywords")),
        startup_wm_class: string("StartupWMClass"),
        only_show_in: list(string("OnlyShowIn")),
        not_show_in: list(string("NotShowIn")),
        no_display: boolean("NoDisplay"),
        hidden: boolean("Hidden"),
        terminal: boolean("Terminal"),
        is_application: kv.get("Type").is_none_or(|t| t.trim() == "Application"),
    })
}

fn unescape(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Split an `Exec` value into arguments, honouring the spec's double quoting.
pub fn exec_args(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// The program an `Exec` line starts, skipping `env VAR=value` wrappers.
pub fn exec_program(exec: &str) -> Option<String> {
    let args = exec_args(exec);
    let mut it = args.into_iter().peekable();
    if it.peek().is_some_and(|a| a == "env" || a.ends_with("/env")) {
        it.next();
        while it.peek().is_some_and(|a| a.contains('=') || a.starts_with('-')) {
            it.next();
        }
    }
    it.find(|a| !a.contains('=') || a.starts_with('/'))
}

/// All desktop entries visible to the user, with XDG precedence applied.
#[derive(Debug, Default)]
pub struct DesktopIndex {
    entries: Vec<DesktopEntry>,
    /// Effective entry per desktop ID (the first in XDG precedence order).
    by_id: HashMap<String, usize>,
    by_path: HashMap<PathBuf, usize>,
    /// Applications directories in precedence order (host paths inside the sysroot).
    pub dirs: Vec<PathBuf>,
}

impl DesktopIndex {
    pub fn scan(sys: &SysRoot) -> Self {
        let mut dirs = vec![sys.data_home.join("applications")];
        dirs.extend(sys.data_dirs.iter().map(|d| d.join("applications")));
        dirs.dedup();
        let mut index = DesktopIndex { dirs: dirs.clone(), ..Default::default() };
        for dir in &dirs {
            index.scan_dir(dir, dir, sys.locale.as_deref());
        }
        index
    }

    fn scan_dir(&mut self, base: &Path, dir: &Path, locale: Option<&str>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                self.scan_dir(base, &path, locale);
                continue;
            }
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let Ok(rel) = path.strip_prefix(base) else { continue };
            let id = rel.to_string_lossy().replace('/', "-");
            let Ok(content) = fs::read_to_string(&path) else { continue };
            let Some(entry) = parse(&content, &id, &path, locale) else { continue };
            let idx = self.entries.len();
            self.by_id.entry(id).or_insert(idx);
            self.by_path.insert(path, idx);
            self.entries.push(entry);
        }
    }

    pub fn by_path(&self, path: &Path) -> Option<&DesktopEntry> {
        self.by_path.get(path).map(|&i| &self.entries[i])
    }

    pub fn by_id(&self, id: &str) -> Option<&DesktopEntry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// The entry the menu shows for the launcher stored at `path`: a user override with the
    /// same ID takes precedence over the packaged file.
    pub fn effective_for_path(&self, path: &Path) -> Option<&DesktopEntry> {
        let own = self.by_path(path)?;
        self.by_id(&own.id).or(Some(own))
    }

    /// Effective entries only (one per desktop ID).
    pub fn effective(&self) -> impl Iterator<Item = &DesktopEntry> {
        self.by_id.values().map(|&i| &self.entries[i])
    }

    pub fn all(&self) -> &[DesktopEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_localized_entry() {
        let src = "[Desktop Entry]\nType=Application\nName=Calculator\nName[ru]=Калькулятор\n\
                   Exec=gnome-calculator %U\nCategories=GNOME;Utility;\nNoDisplay=false\n\
                   [Desktop Action new]\nName=Ignored\n";
        let e = parse(src, "calc.desktop", Path::new("/x/calc.desktop"), Some("ru_RU")).unwrap();
        assert_eq!(e.name, "Калькулятор");
        assert_eq!(e.categories, vec!["GNOME", "Utility"]);
        assert!(e.is_visible());
        assert_eq!(exec_program(e.exec.as_deref().unwrap()).as_deref(), Some("gnome-calculator"));
    }

    #[test]
    fn hidden_and_non_app() {
        let e = parse("[Desktop Entry]\nType=Link\nName=X\n", "x.desktop", Path::new("/x"), None).unwrap();
        assert!(!e.is_visible());
        let e = parse("[Desktop Entry]\nName=X\nNoDisplay=true\n", "x.desktop", Path::new("/x"), None).unwrap();
        assert!(!e.is_visible());
        assert!(parse("garbage", "x.desktop", Path::new("/x"), None).is_none());
    }

    #[test]
    fn exec_parsing() {
        assert_eq!(
            exec_args(r#""/opt/claude code/bin/claude" --handle-uri %u"#),
            vec!["/opt/claude code/bin/claude", "--handle-uri", "%u"]
        );
        assert_eq!(exec_program("env LUTRIS_SKIP_INIT=1 lutris lutris:rungameid/1").as_deref(), Some("lutris"));
        assert_eq!(exec_program("env -u FOO=1 /usr/bin/foo").as_deref(), Some("/usr/bin/foo"));
    }
}
