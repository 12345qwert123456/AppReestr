//! Human-readable texts for reasons and sources.

use crate::i18n::{tr, trf};
use crate::model::{AurEvidence, FlatpakScope, Reason, RepoKind, ShortcutKind, Source, SourceFilter};

fn list(items: &[String]) -> String {
    const MAX: usize = 4;
    let mut s = items.iter().take(MAX).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > MAX {
        s.push_str(&trf(" and {} more", &[&(items.len() - MAX).to_string()]));
    }
    s
}

pub fn describe(r: &Reason) -> String {
    match r {
        Reason::UserMarkedApp => tr("You marked it as an application"),
        Reason::UserHidden => tr("You hid it from the list"),
        Reason::RuleApp => tr("Listed as an application by the rules"),
        Reason::RuleSystem(p) => trf("Listed as a system component by the rules ({})", &[p]),
        Reason::MetainfoDesktopApp => tr("Describes itself as a desktop application"),
        Reason::MetainfoConsoleApp => tr("Describes itself as a command-line application"),
        Reason::MetainfoComponent(k) => trf("Describes itself as a system component ({})", &[k]),
        Reason::HasLauncher => tr("Has a launcher in the application menu"),
        Reason::OnlyHiddenLaunchers => tr("Its launchers are hidden from the menu"),
        Reason::InstalledExplicitly => tr("Installed on purpose, not as a dependency"),
        Reason::InstalledAsDependency(by) if by.is_empty() => tr("Installed as a dependency"),
        Reason::InstalledAsDependency(by) => trf("Installed as a part of: {}", &[&list(by)]),
        Reason::PartOfMeta(by) => trf("Installed as part of the set: {}", &[&list(by)]),
        Reason::LeftoverDependency => tr("Was installed as a dependency, nothing needs it any more"),
        Reason::CommandLineTools(bins) => trf("Command-line tool: {}", &[&list(bins)]),
        Reason::InstalledWithSystem => tr("Installed together with the operating system"),
        Reason::NamePattern(p) => trf("System component by name ({})", &[p]),
        Reason::BaseGroup => tr("Part of the base system"),
        Reason::Service => tr("Background service"),
        Reason::Kernel => tr("Linux kernel"),
        Reason::NoExecutables => tr("Has no programs to run (library or data)"),
        Reason::FlatpakApp => tr("Flatpak application"),
        Reason::FlatpakRuntime => tr("Shared Flatpak runtime used by applications"),
        Reason::AppImageFile => tr("AppImage file"),
        Reason::SnapApp => tr("Snap application"),
        Reason::SnapBase => tr("Snap base or runtime"),
        Reason::ManualInstall => tr("Installed manually, outside any package manager"),
        Reason::ShortcutOf(k) => trf("Launcher created by {}", &[&shortcut_label(*k)]),
    }
}

pub fn shortcut_label(k: ShortcutKind) -> String {
    match k {
        ShortcutKind::Steam => "Steam".into(),
        ShortcutKind::Wine => "Wine".into(),
        ShortcutKind::GameLauncher => tr("a game launcher"),
        ShortcutKind::WebApp => tr("a web browser"),
    }
}

/// Short label for the list row ("Arch Linux", "AUR", "Flatpak"…).
pub fn source_short(s: &Source) -> String {
    match s {
        Source::Repo { label, kind: RepoKind::ThirdParty, .. } => label.clone(),
        Source::Repo { label, .. } => label.clone(),
        Source::Aur { .. } => "AUR".into(),
        Source::Foreign => tr("Local package"),
        Source::Flatpak { .. } => "Flatpak".into(),
        Source::AppImage => "AppImage".into(),
        Source::Snap { .. } => "Snap".into(),
        Source::Manual => tr("Manual"),
        Source::Shortcut { kind } => shortcut_label(*kind),
    }
}

/// Full sentence for the details page.
pub fn source_long(s: &Source) -> String {
    match s {
        Source::Repo { label, kind: RepoKind::ArchOfficial, .. } => trf("From the official {} repositories", &[label]),
        Source::Repo { label, kind: RepoKind::Distro, .. } => trf("From the official {} repositories", &[label]),
        Source::Repo { label, kind: RepoKind::ThirdParty, .. } => trf("From a third-party repository: {}", &[label]),
        Source::Aur { evidence: AurEvidence::HelperCache(h) } => {
            trf("From the AUR (community repository), built with {}", &[h])
        }
        Source::Aur { evidence: AurEvidence::LocalBuild } => tr("Built locally, most likely from the AUR"),
        Source::Foreign => tr("Installed from a package file, or removed from the repositories"),
        Source::Flatpak { scope, remote, .. } => {
            let who = match scope {
                FlatpakScope::User => tr("for this user"),
                FlatpakScope::System => tr("for all users"),
            };
            match remote {
                Some(r) => trf("Flatpak from {}, {}", &[r, &who]),
                None => trf("Flatpak, {}", &[&who]),
            }
        }
        Source::AppImage => tr("AppImage file: a self-contained program"),
        Source::Snap { publisher: Some(p), .. } => trf("Snap from {}", &[p]),
        Source::Snap { .. } => tr("Snap package"),
        Source::Manual => tr("Installed manually (unpacked archive or installer)"),
        Source::Shortcut { kind } => trf("Launcher created by {}", &[&shortcut_label(*kind)]),
    }
}

pub fn filter_label(f: SourceFilter) -> String {
    match f {
        SourceFilter::Official => tr("Official"),
        SourceFilter::ThirdPartyRepo => tr("Third-party"),
        SourceFilter::Aur => "AUR".into(),
        SourceFilter::Flatpak => "Flatpak".into(),
        SourceFilter::AppImage => "AppImage".into(),
        SourceFilter::Snap => "Snap".into(),
        SourceFilter::Manual => tr("Manual"),
    }
}

/// `245 MB`, decimal units like GNOME, with the locale's decimal separator.
pub fn format_size(bytes: u64) -> String {
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < 4 {
        v /= 1000.0;
        i += 1;
    }
    let unit = match i {
        0 => tr("B"),
        1 => tr("kB"),
        2 => tr("MB"),
        3 => tr("GB"),
        _ => tr("TB"),
    };
    let num = if i == 0 || v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    format!("{} {unit}", num.replace('.', &crate::i18n::decimal_point().to_string()))
}

/// Why a package cannot be removed, in plain words.
pub fn block_text(pkg: &str, b: &crate::protect::Block) -> String {
    use crate::protect::Block;
    match b {
        Block::Essential(_) => trf("{} is an essential part of the system.", &[pkg]),
        Block::Kernel(k) => trf("{} is a Linux kernel the system boots with.", &[k]),
        Block::HoldPkg(_) => trf("{} is held by the package manager configuration.", &[pkg]),
        Block::Session(p) => trf("Your desktop is running on it right now ({}).", &[p]),
        Block::DisplayManager(dm) => trf("It is the login screen of this computer ({}).", &[dm]),
        Block::PolkitAgent(a) => trf("It asks for passwords in your session ({}).", &[a]),
        Block::NeededBy { root, reason } => {
            let why = block_text(root, reason);
            trf("{} needs it. {}", &[root, &why])
        }
    }
}

pub fn warning_text(pkg: &str, w: &crate::protect::Warning) -> String {
    use crate::protect::Warning;
    match w {
        Warning::Running(_) => trf("{} is running now. Save your work and close it first.", &[pkg]),
        Warning::DesktopComponent(c) => {
            trf("{} is part of your desktop ({}); it will disappear from the screen.", &[pkg, c])
        }
        Warning::Service(units) => {
            trf("{} runs as a background service ({}); it will stop working.", &[pkg, &list(units)])
        }
        Warning::OptionalFor(users) => trf("Some features of these programs will stop working: {}", &[&list(users)]),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(245_300_000), "245 MB");
        assert_eq!(format_size(1_250_000_000), "1.2 GB");
    }
}
