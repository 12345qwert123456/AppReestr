//! What counts as an application: a pure function from facts to a visibility with reasons.

use crate::model::{ComponentKind, Installation, Reason, Source, Visibility};
use crate::rules::{Rules, any_glob};
use std::collections::HashSet;

pub struct Context<'a> {
    pub rules: &'a Rules,
    /// Installed "sets": packages that are not programs themselves (no launcher, nothing in
    /// /usr/bin) — meta packages and desktop configuration packages that pull in applications.
    pub sets: &'a HashSet<String>,
}

/// Whether a pacman unit is a "set" rather than a program (see [`Context::sets`]).
pub fn is_set(unit: &Installation) -> bool {
    let Some(pkg) = &unit.pkg else { return false };
    if pkg.is_meta || unit.unit_name.ends_with("-meta") {
        return true;
    }
    pkg.binaries.is_empty() && unit.visible_entries().next().is_none() && !pkg.depends.is_empty()
}

fn is_system_component(kind: ComponentKind) -> Option<&'static str> {
    Some(match kind {
        ComponentKind::Runtime => "runtime",
        ComponentKind::Font => "font",
        ComponentKind::Codec => "codec",
        ComponentKind::Driver => "driver",
        ComponentKind::Firmware => "firmware",
        ComponentKind::Localization => "localization",
        ComponentKind::InputMethod => "inputmethod",
        ComponentKind::IconTheme => "icon-theme",
        _ => return None,
    })
}

pub fn classify(unit: &Installation, ctx: &Context) -> (Visibility, Vec<Reason>) {
    use Visibility::*;
    let rules = &ctx.rules.classify;
    let visible: Vec<_> =
        unit.visible_entries().filter(|e| any_glob(&rules.system_desktop_ids, &e.id).is_none()).collect();

    match &unit.source {
        Source::Flatpak { .. } if unit.is_runtime => return (System, vec![Reason::FlatpakRuntime]),
        Source::Flatpak { .. } => {
            if visible.is_empty() && !unit.desktop_entries.is_empty() {
                return (System, vec![Reason::FlatpakApp, Reason::OnlyHiddenLaunchers]);
            }
            return (App, vec![Reason::FlatpakApp]);
        }
        Source::AppImage => return (App, vec![Reason::AppImageFile]),
        Source::Snap { .. } if unit.is_runtime => return (System, vec![Reason::SnapBase]),
        Source::Snap { .. } => {
            return if visible.is_empty() { (Console, vec![Reason::SnapApp]) } else { (App, vec![Reason::SnapApp]) };
        }
        Source::Manual => {
            return if visible.is_empty() {
                (Console, vec![Reason::ManualInstall])
            } else {
                (App, vec![Reason::ManualInstall, Reason::HasLauncher])
            };
        }
        Source::Shortcut { kind } => return (App, vec![Reason::ShortcutOf(*kind)]),
        Source::Repo { .. } | Source::Aur { .. } | Source::Foreign => {}
    }

    let name = unit.unit_name.as_str();
    let default_facts = Default::default();
    let pkg = unit.pkg.as_ref().unwrap_or(&default_facts);

    if any_glob(&rules.app_packages, name).is_some() {
        return (App, vec![Reason::RuleApp]);
    }
    if let Some(p) = any_glob(&rules.system_packages, name) {
        return (System, vec![Reason::RuleSystem(p)]);
    }
    if let Some(kind) = unit.metainfo.as_ref().and_then(|m| is_system_component(m.kind)) {
        return (System, vec![Reason::MetainfoComponent(kind.into())]);
    }

    if !visible.is_empty() {
        let mut reasons = vec![Reason::HasLauncher];
        if pkg.explicit {
            reasons.push(Reason::InstalledExplicitly);
            return (App, reasons);
        }
        if pkg.required_by.is_empty() {
            reasons.push(Reason::LeftoverDependency);
            return (App, reasons);
        }
        if pkg.required_by.iter().all(|r| ctx.sets.contains(r)) {
            reasons.push(Reason::PartOfMeta(pkg.required_by.clone()));
            return (App, reasons);
        }
        let parents: Vec<String> = pkg.required_by.iter().filter(|r| !ctx.sets.contains(*r)).cloned().collect();
        reasons.push(Reason::InstalledAsDependency(parents));
        return (System, reasons);
    }

    let mut reasons = Vec::new();
    if !unit.desktop_entries.is_empty() {
        reasons.push(Reason::OnlyHiddenLaunchers);
    }
    let system = |mut reasons: Vec<Reason>, r: Reason| {
        reasons.push(r);
        (System, reasons)
    };
    if pkg.has_kernel {
        return system(reasons, Reason::Kernel);
    }
    if pkg.groups.iter().any(|g| rules.system_groups.contains(g)) {
        return system(reasons, Reason::BaseGroup);
    }
    if let Some(p) = any_glob(&rules.system_name_patterns, name) {
        return system(reasons, Reason::NamePattern(p));
    }
    // What the user installed after the OS was set up: asked for by name, or kept after the
    // package was marked as a dependency (`pacman -D --asdeps`) while nothing else needs it.
    let by_user = !pkg.installed_with_system;
    let asked_for = pkg.explicit && by_user;
    let kept_orphan = !pkg.explicit && by_user && pkg.required_by.is_empty() && !pkg.binaries.is_empty();
    if !(asked_for && !pkg.binaries.is_empty())
        && let Some(p) = any_glob(&rules.system_name_patterns_weak, name)
    {
        return system(reasons, Reason::NamePattern(p));
    }
    if kept_orphan {
        reasons.push(Reason::LeftoverDependency);
        reasons.push(Reason::CommandLineTools(pkg.binaries.iter().take(5).cloned().collect()));
        return (Console, reasons);
    }
    if !pkg.explicit {
        return system(reasons, Reason::InstalledAsDependency(pkg.required_by.clone()));
    }
    if pkg.binaries.is_empty() {
        if asked_for && !pkg.is_meta && pkg.file_count > 0 {
            reasons.push(Reason::AddOn);
            return (Addon, reasons);
        }
        return system(reasons, Reason::NoExecutables);
    }
    if pkg.installed_with_system {
        return system(reasons, Reason::InstalledWithSystem);
    }
    // A daemon the user installed (incus, podman…) is still their choice: note it, keep it listed.
    if pkg.has_services {
        reasons.push(Reason::Service);
    }
    if unit.metainfo.as_ref().is_some_and(|m| m.kind == ComponentKind::ConsoleApplication) {
        reasons.push(Reason::MetainfoConsoleApp);
    }
    reasons.push(Reason::CommandLineTools(pkg.binaries.iter().take(5).cloned().collect()));
    (Console, reasons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DesktopEntry, PkgFacts, RepoKind, UnitKey};

    fn pkg(name: &str, explicit: bool, launcher: bool, f: impl FnOnce(&mut PkgFacts)) -> Installation {
        let mut u = Installation::new(
            UnitKey::new("pacman", name),
            Source::Repo { repo: "extra".into(), kind: RepoKind::ArchOfficial, label: "Arch Linux".into() },
            name,
        );
        if launcher {
            u.desktop_entries.push(DesktopEntry {
                id: format!("{name}.desktop"),
                name: name.into(),
                is_application: true,
                ..Default::default()
            });
        }
        let mut facts = PkgFacts { explicit, binaries: vec![name.into()], file_count: 10, ..Default::default() };
        f(&mut facts);
        u.pkg = Some(facts);
        u
    }

    fn run(u: &Installation, metas: &[&str]) -> Visibility {
        let rules = Rules::bundled();
        let sets: HashSet<String> = metas.iter().map(|s| s.to_string()).collect();
        classify(u, &Context { rules: &rules, sets: &sets }).0
    }

    #[test]
    fn explicit_app_with_launcher() {
        assert_eq!(run(&pkg("firefox", true, true, |_| {}), &[]), Visibility::App);
    }

    #[test]
    fn dependency_launchers() {
        let avahi = pkg("avahi", false, true, |f| f.required_by = vec!["cups".into()]);
        assert_eq!(run(&avahi, &[]), Visibility::System);
        let dolphin = pkg("dolphin", false, true, |f| f.required_by = vec!["plasma-meta".into()]);
        assert_eq!(run(&dolphin, &["plasma-meta"]), Visibility::App);
        let orphan = pkg("gimp", false, true, |_| {});
        assert_eq!(run(&orphan, &[]), Visibility::App);
    }

    #[test]
    fn console_and_system() {
        assert_eq!(run(&pkg("ripgrep", true, false, |_| {}), &[]), Visibility::Console);
        assert_eq!(run(&pkg("libva-utils", true, false, |_| {}), &[]), Visibility::System);
        assert_eq!(run(&pkg("sudo", true, false, |f| f.groups = vec!["base".into()]), &[]), Visibility::System);
        assert_eq!(run(&pkg("nano", true, false, |f| f.installed_with_system = true), &[]), Visibility::System);
        assert_eq!(run(&pkg("glibc", false, false, |f| f.required_by = vec!["bash".into()]), &[]), Visibility::System);
        assert_eq!(run(&pkg("linux-zen", true, false, |f| f.has_kernel = true), &[]), Visibility::System);
    }

    #[test]
    fn installed_by_the_user() {
        // A daemon installed by hand is a tool, not system plumbing.
        let incus = pkg("incus", true, false, |f| f.has_services = true);
        let (vis, reasons) = classify(&incus, &Context { rules: &Rules::bundled(), sets: &HashSet::new() });
        assert_eq!(vis, Visibility::Console);
        assert!(reasons.contains(&Reason::Service));
        // Weak name patterns only apply to what the user did not ask for.
        assert_eq!(run(&pkg("python-pip", true, false, |_| {}), &[]), Visibility::Console);
        assert_eq!(
            run(&pkg("python-requests", false, false, |f| f.required_by = vec!["x".into()]), &[]),
            Visibility::System
        );
        assert_eq!(run(&pkg("python-pip", true, false, |f| f.installed_with_system = true), &[]), Visibility::System);
        assert_eq!(run(&pkg("libhandy", true, false, |_| {}), &[]), Visibility::System);
        // Marked --asdeps later, but nothing needs it.
        assert_eq!(run(&pkg("tmux", false, false, |_| {}), &[]), Visibility::Console);
        assert_eq!(run(&pkg("tmux", false, false, |f| f.installed_with_system = true), &[]), Visibility::System);
        assert_eq!(run(&pkg("tmux", false, false, |f| f.required_by = vec!["x".into()]), &[]), Visibility::System);
        assert_eq!(run(&pkg("lib-data", false, false, |f| f.binaries.clear()), &[]), Visibility::System);
    }

    #[test]
    fn addons() {
        let none = |f: &mut PkgFacts| f.binaries.clear();
        assert_eq!(run(&pkg("incus-ui", true, false, none), &[]), Visibility::Addon);
        assert_eq!(run(&pkg("hunspell-ru", true, false, none), &[]), Visibility::Addon);
        // Installed with the OS, a meta package, a library or a dependency are not add-ons.
        assert_eq!(
            run(
                &pkg("hunspell-en_us", true, false, |f| {
                    f.binaries.clear();
                    f.installed_with_system = true
                }),
                &[]
            ),
            Visibility::System
        );
        assert_eq!(run(&pkg("some-meta", true, false, none), &[]), Visibility::System);
        assert_eq!(
            run(
                &pkg("gaming-set", true, false, |f| {
                    f.binaries.clear();
                    f.is_meta = true
                }),
                &[]
            ),
            Visibility::System
        );
        assert_eq!(run(&pkg("libfoo", true, false, none), &[]), Visibility::System);
        assert_eq!(run(&pkg("dep-data", false, false, none), &[]), Visibility::System);
    }

    #[test]
    fn hidden_launchers_and_rules() {
        let mut u = pkg("avahi", true, true, |_| {});
        u.desktop_entries[0].id = "avahi-discover.desktop".into();
        assert_eq!(run(&u, &[]), Visibility::Console);
        let mut hidden = pkg("helper", true, true, |f| {
            f.binaries.clear();
            f.installed_with_system = true;
        });
        hidden.desktop_entries[0].no_display = true;
        assert_eq!(run(&hidden, &[]), Visibility::System);
    }
}
