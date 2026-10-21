//! Settings, data and cache an application leaves in the home directory.
//!
//! Only `$HOME` is considered, never shared or system locations. Exact name matches are
//! proposed pre-selected; partial matches are shown unselected. Folders that also match an
//! application that stays installed are not proposed. Removal goes to the trash.

use crate::desktop;
use crate::model::{App, Source};
use crate::sysroot::SysRoot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Settings,
    Data,
    Cache,
    Launcher,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leftover {
    pub path: PathBuf,
    pub kind: Kind,
    pub confidence: Confidence,
    /// Bytes, or `None` when counting was cut short.
    pub size: Option<u64>,
}

/// Never proposed, whatever matches (relative to $HOME).
const PROTECTED: &[&str] = &[
    "",
    ".config",
    ".local",
    ".local/share",
    ".local/state",
    ".cache",
    ".local/bin",
    ".ssh",
    ".gnupg",
    ".pki",
    ".local/share/keyrings",
    ".password-store",
    ".local/share/Trash",
    ".local/share/applications",
    ".local/share/icons",
    ".local/share/fonts",
    ".config/autostart",
    ".config/systemd",
    ".config/environment.d",
    ".cargo",
    ".rustup",
    ".npm",
    ".local/share/flatpak",
    ".var",
    ".var/app",
    "snap",
    ".mozilla",
    ".themes",
    ".icons",
    ".fonts",
    ".local/share/themes",
    ".config/gtk-3.0",
    ".config/gtk-4.0",
    ".config/dconf",
    ".local/share/mime",
    ".config/menus",
    ".local/share/recently-used.xbel",
    ".config/user-dirs.dirs",
    ".config/mimeapps.list",
    ".local/share/applications/mimeapps.list",
    ".bashrc",
    ".zshrc",
    ".profile",
    ".bash_profile",
    ".config/fish",
    ".config/pulse",
    ".local/state/wireplumber",
    ".config/kdeglobals",
    ".config/hypr",
    ".local/share/kwalletd",
    ".local/share/baloo",
    ".cache/thumbnails",
    ".cache/fontconfig",
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Videos",
    "Templates",
    "Public",
];

/// Tokens too generic to identify an application.
const GENERIC: &[&str] = &[
    "org",
    "com",
    "io",
    "net",
    "de",
    "ru",
    "dev",
    "app",
    "apps",
    "github",
    "gitlab",
    "kde",
    "gnome",
    "gtk",
    "qt",
    "desktop",
    "settings",
    "config",
    "data",
    "cache",
    "share",
    "lib",
    "bin",
    "local",
    "user",
    "tool",
    "tools",
    "linux",
    "the",
    "and",
    "electron",
    "java",
    "python",
    "bash",
    "shell",
    "x11",
    "wayland",
    "freedesktop",
    "client",
    "server",
    "manager",
    "viewer",
    "editor",
    "player",
    "browser",
    "launcher",
    "bin",
    "git",
];

/// Well-known locations that the name heuristics would miss or get too broad.
const KNOWN: &[(&str, &[&str])] = &[
    ("firefox", &[".mozilla/firefox", ".config/mozilla/firefox", ".cache/mozilla/firefox"]),
    ("thunderbird", &[".thunderbird", ".cache/thunderbird"]),
    ("google-chrome", &[".config/google-chrome", ".cache/google-chrome"]),
    ("chromium", &[".config/chromium", ".cache/chromium"]),
    ("code", &[".vscode", ".config/Code"]),
    ("visual-studio-code-bin", &[".vscode", ".config/Code"]),
    ("telegram-desktop", &[".local/share/TelegramDesktop"]),
    ("steam", &[".steam", ".local/share/Steam"]),
];

fn add_token(t: &mut BTreeSet<String>, s: &str) {
    let s = s.trim().to_lowercase();
    if s.len() >= 3 && !GENERIC.contains(&s.as_str()) {
        t.insert(s);
    }
}

fn tokens(app: &App) -> BTreeSet<String> {
    let mut t = BTreeSet::new();
    let mut add = |s: &str| add_token(&mut t, s);
    for inst in &app.installs {
        if !matches!(inst.source, Source::AppImage | Source::Manual) {
            add(&inst.unit_name);
        }
        if let Some(p) = &inst.pkg {
            if let Some(b) = &p.base {
                add(b);
            }
            for b in &p.binaries {
                add(b);
            }
        }
        if let Some(m) = &inst.metainfo {
            add(&m.id);
            if let Some(last) = m.id.rsplit('.').next() {
                add(last);
            }
        }
        if let Source::Flatpak { .. } = inst.source
            && let Some(last) = inst.unit_name.rsplit('.').next()
        {
            add(last);
        }
        for e in inst.visible_entries() {
            add(e.id_stem());
            if let Some(last) = e.id_stem().rsplit('.').next() {
                add(last);
            }
            if let Some(w) = &e.startup_wm_class {
                add(w);
            }
        }
    }
    add(&app.name.replace(' ', ""));
    add(&app.name.replace(' ', "-"));
    // Strip packaging suffixes: `foo-bin` also leaves `foo`.
    let extra: Vec<String> = t
        .iter()
        .filter_map(|s| {
            ["-bin", "-git", "-appimage", "-desktop", "-stable", "-beta"].iter().find_map(|suf| s.strip_suffix(suf))
        })
        .map(String::from)
        .collect();
    for e in extra {
        add_token(&mut t, &e);
    }
    t
}

fn kind_of(rel: &Path) -> Kind {
    let s = rel.to_string_lossy();
    if s.starts_with(".cache") {
        Kind::Cache
    } else if s.starts_with(".config") {
        Kind::Settings
    } else {
        Kind::Data
    }
}

fn matches(entry: &str, tokens: &BTreeSet<String>) -> Option<Confidence> {
    let e = entry.trim_start_matches('.').to_lowercase();
    if tokens.contains(&e) {
        return Some(Confidence::High);
    }
    tokens.iter().any(|t| e.starts_with(t) && e[t.len()..].starts_with(['-', '_', '.', ' '])).then_some(Confidence::Low)
}

/// Size of a tree, giving up after `budget` entries (returns `None` then).
pub fn tree_size(path: &Path, budget: usize) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    let root_dev = std::fs::symlink_metadata(path).ok()?.dev();
    while let Some(p) = stack.pop() {
        seen += 1;
        if seen > budget {
            return None;
        }
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        if md.dev() != root_dev {
            continue;
        }
        if md.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        } else {
            total += md.blocks() * 512;
        }
    }
    Some(total)
}

pub fn find(sys: &SysRoot, app: &App, others: &[&App]) -> Vec<Leftover> {
    let home = &sys.home;
    let mine = tokens(app);
    let theirs: BTreeSet<String> = others.iter().flat_map(|o| tokens(o)).collect();
    let mut out: Vec<Leftover> = Vec::new();
    let mut push = |path: PathBuf, kind: Kind, confidence: Confidence| {
        let Ok(rel) = path.strip_prefix(home) else { return };
        if PROTECTED.iter().any(|p| Path::new(p) == rel) || !path.exists() && path.symlink_metadata().is_err() {
            return;
        }
        if out.iter().any(|l| path.starts_with(&l.path)) {
            return;
        }
        out.retain(|l| !l.path.starts_with(&path));
        out.push(Leftover { path, kind, confidence, size: None });
    };

    for inst in &app.installs {
        match &inst.source {
            Source::Flatpak { .. } => push(home.join(".var/app").join(&inst.unit_name), Kind::Data, Confidence::High),
            Source::Snap { .. } => push(home.join("snap").join(&inst.unit_name), Kind::Data, Confidence::High),
            _ => {}
        }
        for (pkg, paths) in KNOWN {
            if inst.unit_name == *pkg {
                for p in *paths {
                    push(home.join(p), kind_of(Path::new(p)), Confidence::High);
                }
            }
        }
    }

    // Name matches in the usual places.
    for base in [".config", ".local/share", ".local/state", ".cache", ""] {
        let dir = home.join(base);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if base.is_empty() && !name.starts_with('.') {
                continue;
            }
            let Some(conf) = matches(&name, &mine) else { continue };
            // Shared with an application that stays: leave it alone. A weak match of ours
            // yields to any match of another application.
            match (conf, matches(&name, &theirs)) {
                (_, Some(Confidence::High)) | (Confidence::Low, Some(_)) => continue,
                _ => {}
            }
            let rel = Path::new(base).join(&name);
            push(home.join(&rel), kind_of(&rel), conf);
        }
    }

    // Launchers in ~/.local/share/applications that start this app (copies, "open with" entries).
    let binaries: BTreeSet<String> = app
        .installs
        .iter()
        .flat_map(|i| {
            i.pkg
                .iter()
                .flat_map(|p| p.binaries.iter().cloned())
                .chain(i.locations.iter().map(|l| l.to_string_lossy().into_owned()))
        })
        .collect();
    let own_ids: BTreeSet<&str> =
        app.installs.iter().flat_map(|i| i.desktop_entries.iter().map(|e| e.id.as_str())).collect();
    let apps_dir = sys.data_home.join("applications");
    if let Ok(rd) = std::fs::read_dir(&apps_dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let id = e.file_name().to_string_lossy().into_owned();
            let Some(entry) = desktop::parse(&text, &id, &path, None) else { continue };
            let prog = entry.exec.as_deref().and_then(desktop::exec_program).unwrap_or_default();
            let prog_name = Path::new(&prog).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let starts_app = binaries.contains(&prog_name)
                || binaries.iter().any(|b| b.starts_with('/') && prog.starts_with(b.as_str()));
            if own_ids.contains(id.as_str()) && path.starts_with(&apps_dir) || starts_app {
                out.push(Leftover { path, kind: Kind::Launcher, confidence: Confidence::High, size: None });
            }
        }
    }

    for l in &mut out {
        l.size =
            if l.path.is_dir() { tree_size(&l.path, 200_000) } else { l.path.symlink_metadata().ok().map(|m| m.len()) };
    }
    out.sort_by(|a, b| (a.confidence, &a.path).cmp(&(b.confidence, &b.path)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DesktopEntry, Installation, PkgFacts, RepoKind, UnitKey, Visibility};

    fn app(name: &str, pkg: &str, bins: &[&str]) -> App {
        let mut i = Installation::new(
            UnitKey::new("pacman", pkg),
            Source::Repo { repo: "extra".into(), kind: RepoKind::ArchOfficial, label: "Arch".into() },
            pkg,
        );
        i.pkg = Some(PkgFacts { binaries: bins.iter().map(|s| s.to_string()).collect(), ..Default::default() });
        i.desktop_entries.push(DesktopEntry {
            id: format!("{pkg}.desktop"),
            name: name.into(),
            is_application: true,
            ..Default::default()
        });
        App {
            id: pkg.into(),
            name: name.into(),
            icon: None,
            summary: None,
            installs: vec![i],
            visibility: Visibility::App,
            reasons: vec![],
            user_mark: None,
        }
    }

    #[test]
    fn finds_and_respects_others() {
        let dir = std::env::temp_dir().join(format!("appreestr-left-{}", std::process::id()));
        let sys = SysRoot::fixture(&dir);
        let h = &sys.home;
        for d in [
            ".config/qbittorrent",
            ".local/share/qBittorrent",
            ".cache/qBittorrent",
            ".config/qbittorrent-nox-extra",
            ".config/kitty",
            ".ssh",
            ".config/gimp",
        ] {
            std::fs::create_dir_all(h.join(d)).unwrap();
        }
        std::fs::write(h.join(".config/qbittorrent/qBittorrent.conf"), "x".repeat(5000)).unwrap();
        std::fs::create_dir_all(sys.data_home.join("applications")).unwrap();
        std::fs::write(
            sys.data_home.join("applications/userapp-qBittorrent-ABC.desktop"),
            "[Desktop Entry]\nType=Application\nNoDisplay=true\nExec=qbittorrent %U\nName=qBittorrent\n",
        )
        .unwrap();

        let qb = app("qBittorrent", "qbittorrent", &["qbittorrent"]);
        let kitty = app("kitty", "kitty", &["kitty"]);
        let found = find(&sys, &qb, &[&kitty]);
        let rel: Vec<String> = found.iter().map(|l| l.path.strip_prefix(h).unwrap().display().to_string()).collect();
        assert!(rel.contains(&".config/qbittorrent".to_string()), "{rel:?}");
        assert!(rel.contains(&".local/share/qBittorrent".to_string()));
        assert!(rel.contains(&".cache/qBittorrent".to_string()));
        assert!(rel.iter().any(|r| r.ends_with("userapp-qBittorrent-ABC.desktop")));
        assert!(!rel.iter().any(|r| r.contains("kitty") || r.contains(".ssh") || r.contains("gimp")));
        let low = found.iter().find(|l| l.path.ends_with("qbittorrent-nox-extra")).unwrap();
        assert_eq!(low.confidence, Confidence::Low);
        assert!(found.iter().find(|l| l.path.ends_with(".config/qbittorrent")).unwrap().size.unwrap() >= 4096);
        let _ = std::fs::remove_dir_all(dir);
    }
}
