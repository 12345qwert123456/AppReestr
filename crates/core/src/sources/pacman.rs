//! Packages from the pacman local database (official repos, derivatives' repos, AUR).

use super::pacman_log::{InstallMethod, PacmanLog, install_method};
use crate::appstream;
use crate::desktop::DesktopIndex;
use crate::model::{AurEvidence, ComponentKind, Installation, MetaInfo, PkgFacts, RepoKind, Source, UnitKey};
use crate::rules::{Rules, any_glob};
use crate::sysroot::SysRoot;
use alpm::{Alpm, PackageReason};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Which package owns a path: exact files for launcher/binary locations, and directories
/// owned by exactly one package for everything else.
#[derive(Debug, Default, Clone)]
pub struct OwnerIndex {
    files: HashMap<String, String>,
    /// Directory (without trailing slash) → owner; `None` when several packages list it.
    dirs: HashMap<String, Option<String>>,
}

/// Directories whose direct children are indexed exactly (launchers, binaries, unit files).
const EXACT_DIRS: &[&str] = &[
    "usr/bin/",
    "usr/local/bin/",
    "usr/local/share/applications/",
    "usr/share/applications/",
    "usr/lib/systemd/system/",
    "usr/lib/systemd/user/",
];
const MAX_DIR_DEPTH: usize = 4;

impl OwnerIndex {
    fn add(&mut self, rel: &str, pkg: &str) {
        if let Some(dir) = rel.strip_suffix('/') {
            if dir.matches('/').count() < MAX_DIR_DEPTH {
                self.dirs
                    .entry(dir.to_string())
                    .and_modify(|o| {
                        if o.as_deref() != Some(pkg) {
                            *o = None
                        }
                    })
                    .or_insert_with(|| Some(pkg.to_string()));
            }
        } else if EXACT_DIRS.iter().any(|d| rel.strip_prefix(d).is_some_and(|rest| !rest.contains('/'))) {
            self.files.insert(rel.to_string(), pkg.to_string());
        }
    }

    /// Record that `pkg` owns the file `rel` (relative to `/`).
    pub fn insert_file(&mut self, rel: &str, pkg: &str) {
        self.files.insert(rel.to_string(), pkg.to_string());
    }

    /// Owner of an absolute host path (exact file, or deepest uniquely owned directory).
    ///
    /// Only application-specific directories count: at least two levels deep, not under the
    /// home directories, and not the skeleton of the `filesystem` package.
    pub fn owner(&self, abs: &str) -> Option<&str> {
        let rel = abs.trim_start_matches('/');
        if let Some(p) = self.files.get(rel) {
            return Some(p);
        }
        if rel.starts_with("home/") || rel.starts_with("root/") || rel.starts_with("tmp/") {
            return None;
        }
        let mut cur = rel;
        while let Some((parent, _)) = cur.rsplit_once('/') {
            if parent.contains('/')
                && let Some(Some(owner)) = self.dirs.get(parent)
                && owner != "filesystem"
            {
                return Some(owner);
            }
            cur = parent;
        }
        None
    }

    /// Whether the directory itself is listed by any package.
    pub fn dir_is_owned(&self, abs: &str) -> bool {
        self.dirs.contains_key(abs.trim_start_matches('/').trim_end_matches('/'))
    }

    pub fn file_is_owned(&self, abs: &str) -> bool {
        self.files.contains_key(abs.trim_start_matches('/'))
    }
}

/// Everything the pacman scan produces.
#[derive(Debug, Default)]
pub struct PacmanData {
    pub units: Vec<Installation>,
    pub owners: OwnerIndex,
    /// Package → installed packages it depends on.
    pub depends: HashMap<String, Vec<String>>,
    pub hold_pkgs: Vec<String>,
    /// Installed kernel packages.
    pub kernels: Vec<String>,
    pub available: bool,
}

pub fn scan(sys: &SysRoot, desktop: &DesktopIndex, rules: &Rules) -> Result<PacmanData> {
    let conf = if sys.is_host() {
        pacmanconf::Config::new().context("reading pacman.conf")?
    } else {
        let conf_path = sys.path("etc/pacman.conf").to_string_lossy().into_owned();
        let root = sys.root.to_string_lossy().into_owned();
        pacmanconf::Config::with_opts(None, Some(conf_path), Some(root)).context("reading fixture pacman.conf")?
    };
    let handle = alpm_utils::alpm_with_conf(&conf).context("opening the pacman database")?;
    let log_text = std::fs::read_to_string(sys.path(&conf.log_file)).unwrap_or_default();
    let log = PacmanLog::parse(&log_text);
    let os_id = os_release_id(sys);
    Ok(collect(sys, &handle, &log, desktop, rules, os_id.as_deref(), conf.hold_pkg))
}

fn os_release_id(sys: &SysRoot) -> Option<String> {
    let text = std::fs::read_to_string(sys.path("etc/os-release")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("ID=")).map(|v| v.trim_matches('"').to_string())
}

pub fn repo_source(repo: &str, rules: &Rules, os_id: Option<&str>) -> Source {
    let r = &rules.repos;
    let official = r.arch_official.iter().any(|o| o == repo);
    if official {
        if let Some(label) = os_id.and_then(|id| r.same_name_repos.get(id)) {
            return Source::Repo { repo: repo.into(), kind: RepoKind::Distro, label: label.clone() };
        }
        return Source::Repo { repo: repo.into(), kind: RepoKind::ArchOfficial, label: "Arch Linux".into() };
    }
    if let Some((_, label)) = r.distro.iter().find(|(pat, _)| crate::rules::glob(pat, repo)) {
        return Source::Repo { repo: repo.into(), kind: RepoKind::Distro, label: label.clone() };
    }
    Source::Repo { repo: repo.into(), kind: RepoKind::ThirdParty, label: repo.into() }
}

const AUR_CACHES: &[(&str, &str)] = &[
    ("paru", ".cache/paru/clone"),
    ("yay", ".cache/yay"),
    ("pikaur", ".cache/pikaur/aur_repos"),
    ("pikaur", ".cache/pikaur/build"),
    ("trizen", ".cache/trizen"),
    ("aurutils", ".cache/aurutils/sync"),
    ("pakku", ".cache/pakku"),
];

fn aur_cache(sys: &SysRoot, base: &str) -> Option<String> {
    AUR_CACHES.iter().find(|(_, dir)| sys.home.join(dir).join(base).is_dir()).map(|(h, _)| h.to_string())
}

const GENERIC_SHARE_DIRS: &[&str] = &[
    "applications",
    "icons",
    "doc",
    "licenses",
    "man",
    "locale",
    "metainfo",
    "appdata",
    "pixmaps",
    "bash-completion",
    "zsh",
    "fish",
    "dbus-1",
    "mime",
    "glib-2.0",
    "info",
    "polkit-1",
    "help",
    "systemd",
    "kservices5",
    "kservices6",
    "knotifications5",
    "knotifications6",
    "qlogging-categories5",
    "qlogging-categories6",
    "xdg-desktop-portal",
    "gir-1.0",
    "vala",
    "pkgconfig",
    "cmake",
    "fonts",
    "themes",
    "sounds",
    "thumbnailers",
    "gnome-shell",
    "plasma",
    "kio",
    "dbus",
    "X11",
    "xsessions",
    "wayland-sessions",
    "zoneinfo",
    "terminfo",
    "nvim",
    "vim",
];

fn name_matches_dir(pkg: &str, dir: &str) -> bool {
    let d = dir.to_lowercase();
    let p = pkg.to_lowercase();
    let core = p
        .trim_end_matches("-bin")
        .trim_end_matches("-git")
        .trim_end_matches("-appimage")
        .trim_end_matches("-fresh")
        .trim_end_matches("-still")
        .trim_end_matches("-desktop");
    d.len() > 2 && (d == p || d == core || (core.len() > 3 && d.starts_with(core)))
}

fn collect(
    sys: &SysRoot,
    handle: &Alpm,
    log: &PacmanLog,
    desktop: &DesktopIndex,
    rules: &Rules,
    os_id: Option<&str>,
    hold_pkgs: Vec<String>,
) -> PacmanData {
    let local = handle.localdb();
    let syncdbs = handle.syncdbs();
    let pkgs: Vec<_> = local.pkgs().iter().collect();

    // Provider map for resolving dependencies to installed package names.
    let mut provider: HashMap<String, String> = HashMap::new();
    for p in &pkgs {
        provider.insert(p.name().to_string(), p.name().to_string());
    }
    for p in &pkgs {
        for prov in p.provides() {
            provider.entry(prov.name().to_string()).or_insert_with(|| p.name().to_string());
        }
    }
    let mut depends: HashMap<String, Vec<String>> = HashMap::new();
    let mut required_by: HashMap<String, Vec<String>> = HashMap::new();
    let mut optional_for: HashMap<String, Vec<String>> = HashMap::new();
    for p in &pkgs {
        let mut ds: Vec<String> =
            p.depends().iter().filter_map(|d| provider.get(d.name()).cloned()).filter(|d| d != p.name()).collect();
        ds.sort();
        ds.dedup();
        for d in &ds {
            required_by.entry(d.clone()).or_default().push(p.name().to_string());
        }
        for od in p.optdepends() {
            if let Some(o) = provider.get(od.name()) {
                optional_for.entry(o.clone()).or_default().push(p.name().to_string());
            }
        }
        depends.insert(p.name().to_string(), ds);
    }

    let mut owners = OwnerIndex::default();
    let mut units = Vec::with_capacity(pkgs.len());
    let metainfo_dirs = ["usr/share/metainfo/", "usr/share/appdata/"];

    for p in &pkgs {
        let name = p.name();
        let mut facts = PkgFacts {
            base: p.base().map(String::from),
            explicit: p.reason() == PackageReason::Explicit,
            groups: p.groups().iter().map(String::from).collect(),
            depends: depends.get(name).cloned().unwrap_or_default(),
            required_by: required_by.remove(name).unwrap_or_default(),
            optional_for: optional_for.remove(name).unwrap_or_default(),
            packager: p.packager().map(String::from),
            build_date: Some(p.build_date()),
            ..Default::default()
        };
        facts.required_by.sort();
        facts.optional_for.sort();
        facts.optional_for.dedup();

        let mut desktop_paths = Vec::new();
        let mut metainfo_paths = Vec::new();
        let mut opt_dirs: Vec<String> = Vec::new();
        let mut app_dirs: Vec<String> = Vec::new();
        let mut real_files = 0u32;
        for f in p.files().files() {
            let Ok(rel) = std::str::from_utf8(f.name()) else { continue };
            owners.add(rel, name);
            if rel.ends_with('/') {
                if let Some(rest) = rel.strip_prefix("opt/")
                    && let Some((top, "")) = rest.split_once('/')
                {
                    opt_dirs.push(format!("/opt/{top}"));
                }
                for prefix in ["usr/lib/", "usr/share/", "usr/lib32/"] {
                    if let Some(rest) = rel.strip_prefix(prefix)
                        && let Some((top, "")) = rest.split_once('/')
                        && !GENERIC_SHARE_DIRS.contains(&top)
                        && name_matches_dir(name, top)
                    {
                        app_dirs.push(format!("/{prefix}{top}"));
                    }
                }
                continue;
            }
            if !rel.starts_with("usr/share/licenses/") && !rel.starts_with("usr/share/doc/") {
                real_files += 1;
            }
            if let Some(bin) = rel.strip_prefix("usr/bin/") {
                facts.binaries.push(bin.to_string());
            } else if rel.starts_with("usr/share/applications/") && rel.ends_with(".desktop") {
                desktop_paths.push(sys.path(rel));
            } else if metainfo_dirs.iter().any(|d| rel.starts_with(d)) && rel.ends_with(".xml") {
                metainfo_paths.push(sys.path(rel));
            } else if rel.starts_with("usr/lib/modules/") && rel.ends_with("/vmlinuz") {
                facts.has_kernel = true;
            } else if (rel.starts_with("usr/lib/systemd/system/") || rel.starts_with("usr/lib/systemd/user/"))
                && (rel.ends_with(".service") || rel.ends_with(".socket"))
            {
                facts.has_services = true;
            }
        }
        facts.file_count = real_files;
        facts.is_meta = real_files == 0 && !facts.depends.is_empty();

        let log_entry = log.entries.get(name);
        facts.installed_with_system = log_entry.is_some_and(|e| e.with_system);

        let source = match syncdbs.iter().find(|db| db.pkg(name).is_ok()) {
            Some(db) => repo_source(db.name(), rules, os_id),
            None => {
                let base = p.base().unwrap_or(name);
                if let Some(helper) = aur_cache(sys, base) {
                    Source::Aur { evidence: AurEvidence::HelperCache(helper) }
                } else {
                    match log_entry.and_then(|e| e.command.as_deref()).map(|c| install_method(c, name, false)) {
                        Some(InstallMethod::LocalFile { helper: Some(h) }) => {
                            Source::Aur { evidence: AurEvidence::HelperCache(h) }
                        }
                        Some(InstallMethod::LocalFile { helper: None }) => {
                            Source::Aur { evidence: AurEvidence::LocalBuild }
                        }
                        _ => Source::Foreign,
                    }
                }
            }
        };

        let mut unit = Installation::new(UnitKey::new("pacman", name), source, name);
        unit.version = Some(p.version().as_str().to_string());
        unit.size = u64::try_from(p.isize()).ok();
        unit.updated_at = p.install_date();
        unit.installed_at = log_entry.and_then(|e| e.first_installed).or(p.install_date());
        unit.install_command = log_entry.and_then(|e| e.command.clone());
        unit.description = p.desc().map(String::from);
        unit.url = p.url().map(String::from);

        unit.desktop_entries =
            desktop_paths.iter().filter_map(|path| desktop.effective_for_path(path).cloned()).collect();
        unit.metainfo = pick_metainfo(&metainfo_paths, &unit, sys.locale.as_deref());

        opt_dirs.sort();
        opt_dirs.dedup();
        app_dirs.sort();
        app_dirs.dedup();
        unit.locations = opt_dirs
            .into_iter()
            .chain(app_dirs)
            .chain(facts.binaries.iter().take(5).map(|b| format!("/usr/bin/{b}")))
            .map(PathBuf::from)
            .collect();
        unit.pkg = Some(facts);
        units.push(unit);
    }

    let kernels =
        units.iter().filter(|u| u.pkg.as_ref().is_some_and(|p| p.has_kernel)).map(|u| u.unit_name.clone()).collect();
    PacmanData { units, owners, depends, hold_pkgs, kernels, available: true }
}

fn pick_metainfo(paths: &[PathBuf], unit: &Installation, locale: Option<&str>) -> Option<MetaInfo> {
    let parsed: Vec<MetaInfo> = paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|x| appstream::parse(&x, locale))
        .collect();
    let ids: HashSet<&str> = unit.desktop_entries.iter().map(|e| e.id.as_str()).collect();
    let score = |m: &MetaInfo| {
        let launch = m.launchables.iter().any(|l| ids.contains(l.as_str())) as u8;
        let app = (m.kind == ComponentKind::DesktopApplication) as u8;
        let named = m.id.to_lowercase().ends_with(&unit.unit_name.to_lowercase()) as u8;
        launch * 4 + app * 2 + named
    };
    parsed.into_iter().max_by_key(score)
}

/// Package names that are matched by any of `patterns`.
pub fn matching<'a>(names: impl Iterator<Item = &'a str>, patterns: &[String]) -> Vec<String> {
    names.filter(|n| any_glob(patterns, n).is_some()).map(String::from).collect()
}
