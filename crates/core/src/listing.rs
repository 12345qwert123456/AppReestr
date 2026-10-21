//! Search, filter and sort for the list view (kept here so it is testable without a GUI).

use crate::config::SortKey;
use crate::model::{App, Shown, SourceFilter};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ListQuery {
    pub text: String,
    pub filter: Option<SourceFilter>,
    pub show_system: bool,
    pub show_console: bool,
    pub show_addons: bool,
    pub sort: SortKey,
    pub descending: bool,
}

impl ListQuery {
    pub fn shown(&self) -> Shown {
        Shown { system: self.show_system, console: self.show_console, addons: self.show_addons }
    }
}

fn source_rank(a: &App) -> SourceFilter {
    a.primary().source.filter()
}

pub fn compare(a: &App, b: &App, key: SortKey) -> Ordering {
    let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
    match key {
        SortKey::Name => by_name(),
        // Unknown sizes and dates go last.
        SortKey::Size => b.total_size().unwrap_or(0).cmp(&a.total_size().unwrap_or(0)).then_with(by_name),
        SortKey::Date => b.installed_at().unwrap_or(0).cmp(&a.installed_at().unwrap_or(0)).then_with(by_name),
        SortKey::Source => source_rank(a).cmp(&source_rank(b)).then_with(by_name),
    }
}

/// Indices of the apps to show, in display order.
pub fn select<T: AsRef<App>>(apps: &[T], q: &ListQuery) -> Vec<usize> {
    let mut idx: Vec<usize> = apps
        .iter()
        .map(AsRef::as_ref)
        .enumerate()
        .filter(|(_, a)| a.is_listed(q.shown()))
        .filter(|(_, a)| q.filter.is_none_or(|f| a.filters().contains(&f)))
        .filter(|(_, a)| a.matches_query(q.text.trim()))
        .map(|(i, _)| i)
        .collect();
    idx.sort_by(|&x, &y| {
        let o = compare(apps[x].as_ref(), apps[y].as_ref(), q.sort);
        if q.descending { o.reverse() } else { o }
    });
    idx
}

/// How many listed apps each source filter would show (ignoring the text and the filter).
pub fn counts<T: AsRef<App>>(apps: &[T], shown: Shown) -> BTreeMap<SourceFilter, usize> {
    let mut m = BTreeMap::new();
    for a in apps.iter().map(AsRef::as_ref).filter(|a| a.is_listed(shown)) {
        for f in a.filters() {
            *m.entry(f).or_insert(0) += 1;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Installation, RepoKind, Source, UnitKey, Visibility};

    fn app(name: &str, size: u64, vis: Visibility, source: Source) -> App {
        let mut i = Installation::new(UnitKey::new("t", name), source, name);
        i.size = Some(size);
        App {
            id: name.into(),
            name: name.into(),
            icon: None,
            summary: None,
            installs: vec![i],
            visibility: vis,
            reasons: vec![],
            user_mark: None,
        }
    }

    #[test]
    fn filters_and_sorts() {
        let repo = Source::Repo { repo: "extra".into(), kind: RepoKind::ArchOfficial, label: "Arch".into() };
        let apps = vec![
            app("zeta", 10, Visibility::App, repo.clone()),
            app("Alpha", 30, Visibility::App, Source::AppImage),
            app("ripgrep", 5, Visibility::Console, repo.clone()),
            app("incus-ui", 4, Visibility::Addon, repo.clone()),
            app("glibc", 50, Visibility::System, repo),
        ];
        let mut q = ListQuery::default();
        assert_eq!(select(&apps, &q), vec![1, 0]);
        q.show_console = true;
        q.sort = SortKey::Size;
        assert_eq!(select(&apps, &q), vec![1, 0, 2]);
        q.filter = Some(SourceFilter::AppImage);
        assert_eq!(select(&apps, &q), vec![1]);
        q.filter = None;
        q.text = "RIP".into();
        assert_eq!(select(&apps, &q), vec![2]);
        q.text.clear();
        q.show_addons = true;
        assert_eq!(select(&apps, &q), vec![1, 0, 2, 3]);
        let all = Shown { system: true, console: true, addons: true };
        assert_eq!(counts(&apps, all)[&SourceFilter::Official], 4);
        assert_eq!(counts(&apps, Shown::default())[&SourceFilter::Official], 1);
    }
}
