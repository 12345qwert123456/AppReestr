//! One row per application: installation units from different sources are merged when they
//! are the same program (same AppStream ID, launcher ID or window class, or the same name).

use crate::model::{App, Installation, Reason, Source, UserMark, Visibility};
use std::collections::HashMap;

pub struct Classified {
    pub unit: Installation,
    pub visibility: Visibility,
    pub reasons: Vec<Reason>,
}

fn family(s: &Source) -> u8 {
    match s {
        Source::Repo { .. } | Source::Aur { .. } | Source::Foreign => 0,
        Source::Flatpak { .. } => 1,
        Source::Snap { .. } => 2,
        Source::AppImage => 3,
        Source::Manual => 4,
        Source::Shortcut { .. } => 5,
    }
}

fn normalize(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Identity keys of a unit. Strong keys identify a program precisely; the name is weak.
fn keys(u: &Installation) -> (Vec<String>, Option<String>) {
    let mut strong = Vec::new();
    if let Some(m) = &u.metainfo {
        strong.push(format!("as:{}", m.id.trim_end_matches(".desktop").to_lowercase()));
        for l in &m.launchables {
            strong.push(format!("as:{}", l.trim_end_matches(".desktop").to_lowercase()));
        }
    }
    for e in u.visible_entries() {
        strong.push(format!("as:{}", e.id_stem().to_lowercase()));
        if let Some(w) = &e.startup_wm_class {
            strong.push(format!("wm:{}", w.to_lowercase()));
        }
    }
    if let Source::Flatpak { .. } = u.source {
        strong.push(format!("as:{}", u.unit_name.to_lowercase()));
    }
    let weak = display_name(u).map(|n| format!("name:{}", normalize(&n))).filter(|n| n.len() > 7);
    (strong, weak)
}

/// Name from metainfo or a visible launcher. Units without a visible launcher are known by
/// their technical name (a hidden launcher's title would be misleading).
pub fn display_name(u: &Installation) -> Option<String> {
    let visible = u.visible_entries().next().is_some();
    if !visible && !matches!(u.source, Source::Flatpak { .. } | Source::AppImage) {
        return None;
    }
    u.metainfo.as_ref().and_then(|m| m.name.clone()).or_else(|| u.main_entry().map(|e| e.name.clone()))
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut c = i;
    while parent[c] != r {
        let n = parent[c];
        parent[c] = r;
        c = n;
    }
    r
}

pub fn merge(items: Vec<Classified>, marks: &HashMap<String, UserMark>) -> Vec<App> {
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    // key → first unit index per source family
    let mut seen: HashMap<String, Vec<(u8, usize)>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        if it.visibility == Visibility::System {
            continue;
        }
        let fam = family(&it.unit.source);
        let (strong, weak) = keys(&it.unit);
        for k in strong.into_iter().chain(weak) {
            let slot = seen.entry(k).or_default();
            for &(f, j) in slot.iter() {
                if f != fam {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
            if !slot.iter().any(|&(f, _)| f == fam) {
                slot.push((fam, i));
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    // A row holds at most one unit per source family; a second unit of the same family
    // (linked only transitively) becomes its own row.
    let mut items: Vec<Option<Classified>> = items.into_iter().map(Some).collect();
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort();
    let mut apps = Vec::with_capacity(groups.len());
    for root in roots {
        let mut members: Vec<Classified> = groups[&root].iter().map(|&m| items[m].take().unwrap()).collect();
        members.sort_by_key(|c| (c.visibility, family(&c.unit.source)));
        let mut main: Vec<Classified> = Vec::new();
        let mut extra: Vec<Classified> = Vec::new();
        for c in members {
            if main.iter().any(|m| family(&m.unit.source) == family(&c.unit.source)) {
                extra.push(c);
            } else {
                main.push(c);
            }
        }
        apps.push(build_app(main, marks));
        apps.extend(extra.into_iter().map(|e| build_app(vec![e], marks)));
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn build_app(mut members: Vec<Classified>, marks: &HashMap<String, UserMark>) -> App {
    members.sort_by_key(|c| (c.visibility, family(&c.unit.source)));
    let visibility = members[0].visibility;
    let reasons = members[0].reasons.clone();
    let installs: Vec<Installation> = members.into_iter().map(|c| c.unit).collect();
    let p = &installs[0];
    let name = display_name(p)
        .or_else(|| installs.iter().find_map(display_name))
        .or_else(|| match &p.source {
            Source::Flatpak { .. } => p.description.clone(),
            _ => None,
        })
        .unwrap_or_else(|| p.unit_name.clone());
    let icon = p
        .icon_path
        .as_ref()
        .map(|i| i.to_string_lossy().into_owned())
        .or_else(|| installs.iter().find_map(|i| i.main_entry().and_then(|e| e.icon.clone())));
    let summary = p
        .metainfo
        .as_ref()
        .and_then(|m| m.summary.clone())
        .or_else(|| p.main_entry().and_then(|e| e.comment.clone().or(e.generic_name.clone())))
        .or_else(|| p.description.clone());
    let user_mark = installs.iter().find_map(|i| marks.get(i.key.as_str()).copied());
    let mut app = App { id: p.key.to_string(), name, icon, summary, installs, visibility, reasons, user_mark };
    match user_mark {
        Some(UserMark::App) => {
            app.visibility = Visibility::App;
            app.reasons.insert(0, Reason::UserMarkedApp);
        }
        Some(UserMark::Hidden) => app.reasons.insert(0, Reason::UserHidden),
        None => {}
    }
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DesktopEntry, FlatpakScope, RepoKind, UnitKey};

    fn unit(source: Source, name: &str, desktop_id: &str, label: &str) -> Classified {
        let mut u = Installation::new(UnitKey::new("x", name), source, name);
        u.desktop_entries.push(DesktopEntry {
            id: desktop_id.into(),
            name: label.into(),
            is_application: true,
            ..Default::default()
        });
        Classified { unit: u, visibility: Visibility::App, reasons: vec![] }
    }

    fn repo() -> Source {
        Source::Repo { repo: "extra".into(), kind: RepoKind::ArchOfficial, label: "Arch Linux".into() }
    }
    fn flatpak(id: &str) -> Source {
        Source::Flatpak { scope: FlatpakScope::User, remote: None, flatpak_ref: format!("app/{id}/x86_64/stable") }
    }

    #[test]
    fn merges_across_sources_by_name() {
        let apps = merge(
            vec![
                unit(repo(), "firefox", "firefox.desktop", "Firefox"),
                unit(flatpak("org.mozilla.firefox"), "org.mozilla.firefox", "org.mozilla.firefox.desktop", "Firefox"),
                unit(repo(), "gimp", "gimp.desktop", "GNU Image Manipulation Program"),
            ],
            &HashMap::new(),
        );
        assert_eq!(apps.len(), 2);
        let ff = apps.iter().find(|a| a.name == "Firefox").unwrap();
        assert_eq!(ff.installs.len(), 2);
        assert!(ff.installs[0].source.is_pacman());
    }

    #[test]
    fn merges_by_desktop_id() {
        let apps = merge(
            vec![
                unit(repo(), "gnome-calculator", "org.gnome.Calculator.desktop", "Calculator"),
                unit(flatpak("org.gnome.Calculator"), "org.gnome.Calculator", "org.gnome.Calculator.desktop", "Calc"),
            ],
            &HashMap::new(),
        );
        assert_eq!(apps.len(), 1);
    }

    #[test]
    fn never_merges_same_family() {
        let apps = merge(
            vec![
                unit(repo(), "firefox", "firefox.desktop", "Firefox Browser"),
                unit(repo(), "firefox-bin", "firefox-bin.desktop", "Firefox Browser"),
                unit(
                    flatpak("org.mozilla.firefox"),
                    "org.mozilla.firefox",
                    "org.mozilla.firefox.desktop",
                    "Firefox Browser",
                ),
            ],
            &HashMap::new(),
        );
        assert_eq!(apps.len(), 2);
        assert_eq!(apps.iter().map(|a| a.installs.len()).sum::<usize>(), 3);
    }

    #[test]
    fn user_marks() {
        let mut marks = HashMap::new();
        marks.insert("x:gimp".to_string(), UserMark::Hidden);
        let apps = merge(vec![unit(repo(), "gimp", "gimp.desktop", "GIMP")], &marks);
        assert_eq!(apps[0].user_mark, Some(UserMark::Hidden));
        assert!(!apps[0].is_listed(false, false));
    }
}
