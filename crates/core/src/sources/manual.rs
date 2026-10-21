//! Programs installed outside any package manager: unpacked archives and installers in the
//! home folder or `/opt`, binaries dropped into `/usr/local/bin`. Also recognises launchers
//! created by Steam, Wine, game launchers and browsers ("web apps"), and user launchers that
//! actually start a packaged program (those are attached to the package).

use crate::desktop::{self, DesktopIndex};
use crate::leftovers::tree_size;
use crate::model::{DesktopEntry, Installation, ShortcutKind, Source, UnitKey};
use crate::sources::pacman::OwnerIndex;
use crate::sysroot::SysRoot;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Folders people keep programs in; the program's own folder is the next level down.
const CONTAINERS: &[&str] = &[
    "Apps",
    "apps",
    "Applications",
    "Programs",
    "programs",
    "Software",
    "software",
    "opt",
    "Games",
    "games",
    "bin",
    "AppImages",
    "Downloads",
    "Desktop",
    ".local/opt",
    ".local/share",
    ".local/bin",
    ".local/lib",
];

pub struct ManualScan {
    pub units: Vec<Installation>,
    /// User launchers that start a packaged program: (package, launcher).
    pub links: Vec<(String, DesktopEntry)>,
}

pub fn shortcut_kind(e: &DesktopEntry) -> Option<ShortcutKind> {
    let exec = e.exec.as_deref().unwrap_or("");
    let path = e.path.to_string_lossy();
    if exec.contains("steam://rungameid") || exec.contains("steam steam://") {
        return Some(ShortcutKind::Steam);
    }
    if path.contains("/applications/wine/")
        || exec.contains("WINEPREFIX=")
        || exec.contains(" wine ")
        || exec.starts_with("wine ")
        || exec.contains(".exe")
    {
        return Some(ShortcutKind::Wine);
    }
    if exec.contains("lutris:rungame")
        || exec.contains("heroic://launch")
        || exec.contains("bottles-cli")
        || exec.contains("legendary launch")
    {
        return Some(ShortcutKind::GameLauncher);
    }
    if exec.contains("--app-id=") || exec.contains("--app=http") || exec.contains("firefoxpwa site launch") {
        return Some(ShortcutKind::WebApp);
    }
    None
}

/// Resolve a launcher's program to an absolute path.
fn resolve(sys: &SysRoot, prog: &str) -> Option<PathBuf> {
    let p = Path::new(prog);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    let mut dirs = vec![sys.home.join(".local/bin"), sys.path("usr/local/bin"), sys.path("usr/bin")];
    if let Some(path) = std::env::var_os("PATH").filter(|_| sys.is_host()) {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().map(|d| d.join(prog)).find(|c| c.is_file()).map(|c| sys.host_path(&c))
}

/// The folder a manually installed program lives in.
pub fn install_root(sys: &SysRoot, program: &Path) -> PathBuf {
    let host = program;
    if let Ok(rest) = host.strip_prefix("/opt")
        && let Some(first) = rest.components().next()
    {
        return Path::new("/opt").join(first);
    }
    if let Ok(rel) = host.strip_prefix(&sys.home) {
        let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if comps.is_empty() {
            return host.to_path_buf();
        }
        // Skip one container folder (possibly two levels deep, like `.local/share`).
        let skip = if comps.len() > 2 && CONTAINERS.contains(&format!("{}/{}", comps[0], comps[1]).as_str()) {
            2
        } else if comps.len() > 1 && CONTAINERS.contains(&comps[0].as_str()) {
            1
        } else {
            0
        };
        let end = skip.min(comps.len() - 1);
        return sys.home.join(comps[..=end].iter().collect::<PathBuf>());
    }
    host.to_path_buf()
}

fn created(p: &Path) -> Option<i64> {
    let md = std::fs::symlink_metadata(p).ok()?;
    md.created().or_else(|_| md.modified()).ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

fn mtime_minute(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64 / 60)
        .unwrap_or(0)
}

/// Name for a group of files installed together: their common prefix, or the name the
/// others contain (`teleport` for `tsh`, `teleport`, `teleport-update`, `fdpass-teleport`).
pub fn group_name(names: &[String]) -> String {
    if names.len() == 1 {
        return names[0].clone();
    }
    let first = &names[0];
    let mut prefix_len = first.len();
    for n in &names[1..] {
        prefix_len = prefix_len.min(first.chars().zip(n.chars()).take_while(|(a, b)| a == b).count());
    }
    let prefix = first[..prefix_len].trim_end_matches(['-', '_', '.']);
    if prefix.len() >= 3 {
        return prefix.to_string();
    }
    names
        .iter()
        .max_by_key(|n| (names.iter().filter(|o| o.contains(n.as_str())).count(), std::cmp::Reverse(n.len())))
        .cloned()
        .unwrap_or_default()
}

pub fn scan(sys: &SysRoot, desktop: &DesktopIndex, owners: &OwnerIndex, claimed: &HashSet<PathBuf>) -> ManualScan {
    let mut out = ManualScan { units: Vec::new(), links: Vec::new() };

    // Loose binaries in /usr/local/bin, grouped by installation time (one installer run).
    let mut by_minute: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(sys.path("usr/local/bin")) {
        for f in rd.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if owners.file_is_owned(&format!("/usr/local/bin/{name}")) {
                continue;
            }
            // Wrappers shadowing a system command are part of the system setup: leave them be.
            if owners.file_is_owned(&format!("/usr/bin/{name}")) {
                continue;
            }
            by_minute.entry(mtime_minute(&f.path())).or_default().push(name);
        }
    }
    let mut groups: Vec<(PathBuf, Vec<PathBuf>, Vec<DesktopEntry>)> = by_minute
        .into_values()
        .map(|mut names| {
            names.sort();
            let key = Path::new("/usr/local/bin").join(group_name(&names));
            (key, names.iter().map(|n| Path::new("/usr/local/bin").join(n)).collect(), Vec::new())
        })
        .collect();

    let mut roots: BTreeMap<PathBuf, Vec<DesktopEntry>> = BTreeMap::new();
    let export_dirs =
        [sys.path("var/lib/flatpak/exports"), sys.data_home.join("flatpak/exports"), sys.path("var/lib/snapd/desktop")];
    for e in desktop.effective() {
        if claimed.contains(&e.path) || export_dirs.iter().any(|d| e.path.starts_with(d)) {
            continue;
        }
        let host = sys.host_path(&e.path);
        if owners.file_is_owned(&host.to_string_lossy()) || !e.is_visible() {
            continue;
        }
        if let Some(kind) = shortcut_kind(e) {
            let mut u = Installation::new(
                UnitKey::new("shortcut", host.to_string_lossy()),
                Source::Shortcut { kind },
                e.id.clone(),
            );
            u.desktop_entries = vec![e.clone()];
            u.locations = vec![e.path.clone()];
            u.installed_at = created(&e.path);
            out.units.push(u);
            continue;
        }
        let Some(prog) = e.exec.as_deref().and_then(desktop::exec_program) else { continue };
        let resolved = resolve(sys, &prog);
        if let Some(path) = &resolved {
            if let Some(pkg) = owners.owner(&path.to_string_lossy()) {
                out.links.push((pkg.to_string(), e.clone()));
                continue;
            }
            if let Some(g) = groups.iter_mut().find(|(_, files, _)| files.contains(path)) {
                g.2.push(e.clone());
                continue;
            }
        }
        let root = match &resolved {
            Some(p) => install_root(sys, p),
            // The program is gone: only the launcher is left.
            None => e.path.clone(),
        };
        roots.entry(root).or_default().push(e.clone());
    }

    // Unowned folders in /opt.
    if let Ok(rd) = std::fs::read_dir(sys.path("opt")) {
        for d in rd.flatten() {
            let host = Path::new("/opt").join(d.file_name());
            let h = host.to_string_lossy();
            if owners.dir_is_owned(&h) || owners.owner(&format!("{h}/x")).is_some() {
                continue;
            }
            roots.entry(host).or_default();
        }
    }

    for (root, entries) in roots {
        let on_disk = sys.path(&root);
        let mut u = Installation::new(
            UnitKey::new("manual", root.to_string_lossy()),
            Source::Manual,
            root.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        );
        u.size = if on_disk.is_dir() {
            tree_size(&on_disk, 100_000)
        } else {
            std::fs::metadata(&on_disk).ok().map(|m| m.len())
        };
        u.installed_at = created(&on_disk);
        u.locations = vec![root.clone()];
        u.locations.extend(entries.iter().filter(|e| e.path != root).map(|e| e.path.clone()));
        u.desktop_entries = entries;
        out.units.push(u);
    }

    for (key, files, entries) in groups {
        let name = key.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let mut u = Installation::new(UnitKey::new("manual", key.to_string_lossy()), Source::Manual, &name);
        u.size = Some(files.iter().filter_map(|f| std::fs::metadata(sys.path(f)).ok()).map(|m| m.len()).sum());
        u.installed_at = files.first().and_then(|f| created(&sys.path(f)));
        u.locations = files;
        u.locations.extend(entries.iter().map(|e| e.path.clone()));
        u.desktop_entries = entries;
        out.units.push(u);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots() {
        let sys = SysRoot::fixture(Path::new("/x"));
        let h = &sys.home;
        assert_eq!(install_root(&sys, Path::new("/opt/foo/bin/foo")), PathBuf::from("/opt/foo"));
        assert_eq!(install_root(&sys, &h.join("Apps/v2rayN-linux-64/v2rayN")), h.join("Apps/v2rayN-linux-64"));
        assert_eq!(
            install_root(&sys, &h.join(".local/share/JetBrains/Toolbox/bin/jetbrains-toolbox")),
            h.join(".local/share/JetBrains")
        );
        assert_eq!(install_root(&sys, &h.join("tools/foo/run.sh")), h.join("tools"));
        assert_eq!(install_root(&sys, &h.join(".local/bin/script")), h.join(".local/bin/script"));
    }

    #[test]
    fn groups() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            group_name(&n(&["fdpass-teleport", "tbot", "tctl", "teleport", "teleport-update", "tsh"])),
            "teleport"
        );
        assert_eq!(
            group_name(&n(&["multipassport-cli", "multipassport-daemon", "multipassport-gui"])),
            "multipassport"
        );
        assert_eq!(group_name(&n(&["ambxst"])), "ambxst");
    }

    #[test]
    fn shortcuts() {
        let e = |exec: &str| DesktopEntry { exec: Some(exec.into()), ..Default::default() };
        assert_eq!(shortcut_kind(&e("steam steam://rungameid/570")), Some(ShortcutKind::Steam));
        assert_eq!(shortcut_kind(&e("env WINEPREFIX=/x wine C:/a.exe")), Some(ShortcutKind::Wine));
        assert_eq!(
            shortcut_kind(&e("/opt/google/chrome/google-chrome --profile-directory=Default --app-id=abc")),
            Some(ShortcutKind::WebApp)
        );
        assert_eq!(shortcut_kind(&e("firefox %u")), None);
    }
}
