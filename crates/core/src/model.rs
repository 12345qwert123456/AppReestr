//! Data model shared by every source, the classifier and the UI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable identifier of one installation unit: what a single uninstall action removes.
///
/// Format: `<source>:<id>`, e.g. `pacman:firefox`, `flatpak:user:org.mozilla.firefox`,
/// `appimage:/home/u/Applications/Foo.AppImage`, `manual:/opt/foo`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnitKey(pub String);

impl UnitKey {
    pub fn new(source: &str, id: impl AsRef<str>) -> Self {
        Self(format!("{source}:{}", id.as_ref()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UnitKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RepoKind {
    /// core / extra / multilib and their testing variants.
    ArchOfficial,
    /// Official repositories of an Arch derivative (CachyOS, EndeavourOS, Manjaro…).
    Distro,
    /// Any other repository configured in pacman.conf (chaotic-aur, archlinuxcn…).
    ThirdParty,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AurEvidence {
    /// A build directory of an AUR helper exists (`~/.cache/paru/clone/<pkgbase>`…).
    HelperCache(String),
    /// pacman.log shows the package was installed from a locally built file.
    LocalBuild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlatpakScope {
    User,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShortcutKind {
    Steam,
    Wine,
    GameLauncher,
    WebApp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    Repo {
        repo: String,
        kind: RepoKind,
        label: String,
    },
    Aur {
        evidence: AurEvidence,
    },
    /// Not found in any sync database and no evidence of AUR.
    Foreign,
    Flatpak {
        scope: FlatpakScope,
        remote: Option<String>,
        flatpak_ref: String,
    },
    AppImage,
    Snap {
        publisher: Option<String>,
        channel: Option<String>,
    },
    Manual,
    Shortcut {
        kind: ShortcutKind,
    },
}

/// Coarse source categories used by the filter chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceFilter {
    Official,
    ThirdPartyRepo,
    Aur,
    Flatpak,
    AppImage,
    Snap,
    Manual,
}

impl SourceFilter {
    pub const ALL: [SourceFilter; 7] = [
        SourceFilter::Official,
        SourceFilter::ThirdPartyRepo,
        SourceFilter::Aur,
        SourceFilter::Flatpak,
        SourceFilter::AppImage,
        SourceFilter::Snap,
        SourceFilter::Manual,
    ];
}

impl Source {
    pub fn filter(&self) -> SourceFilter {
        match self {
            Source::Repo { kind: RepoKind::ArchOfficial | RepoKind::Distro, .. } => SourceFilter::Official,
            Source::Repo { kind: RepoKind::ThirdParty, .. } => SourceFilter::ThirdPartyRepo,
            Source::Aur { .. } | Source::Foreign => SourceFilter::Aur,
            Source::Flatpak { .. } => SourceFilter::Flatpak,
            Source::AppImage => SourceFilter::AppImage,
            Source::Snap { .. } => SourceFilter::Snap,
            Source::Manual | Source::Shortcut { .. } => SourceFilter::Manual,
        }
    }

    pub fn is_pacman(&self) -> bool {
        matches!(self, Source::Repo { .. } | Source::Aur { .. } | Source::Foreign)
    }
}

/// A parsed `.desktop` launcher.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DesktopEntry {
    /// Desktop file ID, e.g. `org.gnome.Calculator.desktop`.
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub startup_wm_class: Option<String>,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
    pub is_application: bool,
}

impl DesktopEntry {
    /// Whether the launcher appears in application menus.
    pub fn is_visible(&self) -> bool {
        self.is_application && !self.no_display && !self.hidden
    }

    pub fn id_stem(&self) -> &str {
        self.id.strip_suffix(".desktop").unwrap_or(&self.id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentKind {
    DesktopApplication,
    ConsoleApplication,
    WebApplication,
    Runtime,
    Font,
    Codec,
    Addon,
    Driver,
    Firmware,
    Localization,
    InputMethod,
    IconTheme,
    Other,
}

/// AppStream metadata (`/usr/share/metainfo/*.xml`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaInfo {
    pub id: String,
    pub kind: ComponentKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub launchables: Vec<String>,
    pub developer: Option<String>,
}

/// pacman-specific facts used by classification and protection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PkgFacts {
    pub base: Option<String>,
    pub explicit: bool,
    pub groups: Vec<String>,
    /// Installed packages this one depends on (resolved through provides).
    pub depends: Vec<String>,
    pub required_by: Vec<String>,
    pub optional_for: Vec<String>,
    /// Basenames of executables in /usr/bin.
    pub binaries: Vec<String>,
    pub file_count: u32,
    /// Has no files besides licenses/docs: a meta package.
    pub is_meta: bool,
    pub has_kernel: bool,
    pub has_services: bool,
    pub packager: Option<String>,
    pub build_date: Option<i64>,
    /// Installed during the OS installation (by the installer).
    pub installed_with_system: bool,
}

/// What a single uninstall action removes, from one source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Installation {
    pub key: UnitKey,
    pub source: Source,
    /// Technical name: package name, Flatpak app id, file name.
    pub unit_name: String,
    pub version: Option<String>,
    /// Bytes on disk, when known.
    pub size: Option<u64>,
    /// First installation time (unix seconds).
    pub installed_at: Option<i64>,
    /// Last update time (unix seconds).
    pub updated_at: Option<i64>,
    /// Command that installed it, from pacman.log.
    pub install_command: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub desktop_entries: Vec<DesktopEntry>,
    pub metainfo: Option<MetaInfo>,
    /// Main locations on disk (install dir, binaries).
    pub locations: Vec<PathBuf>,
    pub pkg: Option<PkgFacts>,
    /// Flatpak runtime/extension or Snap base/core.
    pub is_runtime: bool,
    /// Icon resolved by the source (absolute path), overrides the launcher icon.
    pub icon_path: Option<PathBuf>,
}

impl Installation {
    pub fn new(key: UnitKey, source: Source, unit_name: impl Into<String>) -> Self {
        Self {
            key,
            source,
            unit_name: unit_name.into(),
            version: None,
            size: None,
            installed_at: None,
            updated_at: None,
            install_command: None,
            description: None,
            url: None,
            desktop_entries: Vec::new(),
            metainfo: None,
            locations: Vec::new(),
            pkg: None,
            is_runtime: false,
            icon_path: None,
        }
    }

    pub fn visible_entries(&self) -> impl Iterator<Item = &DesktopEntry> {
        self.desktop_entries.iter().filter(|e| e.is_visible())
    }

    /// The launcher that best represents the unit.
    pub fn main_entry(&self) -> Option<&DesktopEntry> {
        let stem_matches = |e: &DesktopEntry| {
            let stem = e.id_stem().to_lowercase();
            let unit = self.unit_name.to_lowercase();
            stem == unit || stem.rsplit('.').next() == Some(unit.as_str())
        };
        let launchable =
            |e: &DesktopEntry| self.metainfo.as_ref().is_some_and(|m| m.launchables.iter().any(|l| l == &e.id));
        let visible: Vec<&DesktopEntry> = self.visible_entries().collect();
        let pool: Vec<&DesktopEntry> = if visible.is_empty() { self.desktop_entries.iter().collect() } else { visible };
        pool.iter()
            .copied()
            .max_by_key(|e| {
                (launchable(e) as u8) * 4 + (stem_matches(e) as u8) * 2 + (e.id_stem().contains("startcenter") as u8)
            })
            .or_else(|| pool.first().copied())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Visibility {
    /// Shown by default.
    App,
    /// Command-line tool installed by the user; behind the "console" toggle.
    Console,
    /// Libraries, services, drivers…; behind the "system" toggle.
    System,
}

/// The user's manual decision about an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserMark {
    App,
    Hidden,
}

/// Why an app was given its visibility. Rendered into text by [`crate::reasons`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    UserMarkedApp,
    UserHidden,
    RuleApp,
    RuleSystem(String),
    MetainfoDesktopApp,
    MetainfoConsoleApp,
    MetainfoComponent(String),
    HasLauncher,
    OnlyHiddenLaunchers,
    InstalledExplicitly,
    InstalledAsDependency(Vec<String>),
    PartOfMeta(Vec<String>),
    LeftoverDependency,
    CommandLineTools(Vec<String>),
    InstalledWithSystem,
    NamePattern(String),
    BaseGroup,
    Service,
    Kernel,
    NoExecutables,
    FlatpakApp,
    FlatpakRuntime,
    AppImageFile,
    SnapApp,
    SnapBase,
    ManualInstall,
    ShortcutOf(ShortcutKind),
}

/// One row of the list: an application possibly installed in several ways.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct App {
    /// Stable id: the key of the primary installation.
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub summary: Option<String>,
    pub installs: Vec<Installation>,
    pub visibility: Visibility,
    pub reasons: Vec<Reason>,
    pub user_mark: Option<UserMark>,
}

impl AsRef<App> for App {
    fn as_ref(&self) -> &App {
        self
    }
}

impl App {
    pub fn primary(&self) -> &Installation {
        &self.installs[0]
    }

    /// Launchers made by other programs (Steam, Wine…) are only listed, never removed from here.
    pub fn is_removable(&self) -> bool {
        self.installs.iter().any(|i| !matches!(i.source, Source::Shortcut { .. }))
    }

    pub fn total_size(&self) -> Option<u64> {
        let sizes: Vec<u64> = self.installs.iter().filter_map(|i| i.size).collect();
        (!sizes.is_empty()).then(|| sizes.iter().sum())
    }

    pub fn installed_at(&self) -> Option<i64> {
        self.installs.iter().filter_map(|i| i.installed_at).min()
    }

    pub fn filters(&self) -> Vec<SourceFilter> {
        let mut f: Vec<SourceFilter> = self.installs.iter().map(|i| i.source.filter()).collect();
        f.sort();
        f.dedup();
        f
    }

    /// Shown in the list with the current toggles.
    pub fn is_listed(&self, show_system: bool, show_console: bool) -> bool {
        if self.user_mark == Some(UserMark::Hidden) {
            return show_system;
        }
        match self.visibility {
            Visibility::App => true,
            Visibility::Console => show_console,
            Visibility::System => show_system,
        }
    }

    pub fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q)
            || self.installs.iter().any(|i| {
                i.unit_name.to_lowercase().contains(&q)
                    || i.desktop_entries.iter().any(|e| {
                        e.name.to_lowercase().contains(&q)
                            || e.generic_name.as_ref().is_some_and(|g| g.to_lowercase().contains(&q))
                            || e.keywords.iter().any(|k| k.to_lowercase().contains(&q))
                    })
            })
    }
}
