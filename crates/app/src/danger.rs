//! The Danger Zone: removing any installed package by force (`pacman -Rdd`).
//!
//! Dependencies and protection are ignored on purpose, so the user is told exactly what would
//! break and has to type the package name before the button works.

use crate::format;
use crate::removal::{Job, Step};
use crate::ui::{self, SPACE_M, SPACE_S};
use crate::uninstall::{self, JobRequest};
use crate::window::{AppModel, Cmd, Msg};
use appreestr_core::i18n::{tr, trf, trn};
use appreestr_core::plan::{self, InstalledPkg, RemovalPlan};
use appreestr_core::reasons::{block_text, format_size, warning_text};
use relm4::ComponentSender;
use relm4::adw::{self, prelude::*};
use relm4::gtk;
use std::collections::HashMap;
use std::rc::Rc;

/// Most packages the search shows at once.
const MAX_RESULTS: usize = 40;

/// Everything installed, and which packages the system would refuse to remove.
pub struct Listing {
    pub packages: Vec<InstalledPkg>,
    /// Package → why it is protected.
    pub protected: HashMap<String, String>,
}

/// A package chosen for forced removal, planned with and without its unneeded dependencies.
pub struct Planned {
    pub name: String,
    pub alone: Result<RemovalPlan, String>,
    pub with_deps: Result<RemovalPlan, String>,
    pub need_agent: bool,
    /// Applications that disappear with it.
    pub app_ids: Vec<String>,
}

/// Read the package list in the background; the picker opens when it arrives.
pub fn load(m: &AppModel, sender: &ComponentSender<AppModel>) {
    let (sys, rules, data) = (m.sys.clone(), m.rules.clone(), m.data.clone());
    sender.spawn_oneshot_command(move || {
        let prot = uninstall::protection(&sys, &rules, &data);
        let packages = plan::open_local("/", "/var/lib/pacman").map(|h| plan::installed(&h)).unwrap_or_default();
        let protected = prot.blocked.iter().map(|(n, b)| (n.clone(), block_text(n, b))).collect();
        Cmd::DangerListing(Box::new(Listing { packages, protected }))
    });
}

/// Plan the forced removal of `name` in the background; the confirmation opens when it arrives.
pub fn plan_package(m: &AppModel, name: String, sender: &ComponentSender<AppModel>) {
    let (sys, rules, data) = (m.sys.clone(), m.rules.clone(), m.data.clone());
    let key = format!("pacman:{name}");
    let app_ids: Vec<String> = m
        .apps
        .iter()
        .filter(|a| a.installs.len() == 1 && a.installs[0].key.as_str() == key)
        .map(|a| a.id.clone())
        .collect();
    sender.spawn_oneshot_command(move || {
        let prot = uninstall::protection(&sys, &rules, &data);
        let targets = [name.clone()];
        let planned = |recursive: bool| {
            plan::open_local("/", "/var/lib/pacman")
                .and_then(|mut h| plan::plan_force(&mut h, &targets, recursive, Some(&prot)))
                .map_err(|e| e.to_string())
        };
        Cmd::ForcePlanned(Box::new(Planned {
            alone: planned(false),
            with_deps: planned(true),
            need_agent: !prot.polkit_agent,
            app_ids,
            name,
        }))
    });
}

/// Search the installed packages and pick one.
pub fn picker(listing: Listing, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let search = gtk::SearchEntry::builder().placeholder_text(tr("Search installed packages")).hexpand(true).build();
    let results = ui::boxed_list();
    let hint = ui::label(&tr("Type part of a package name."), &["dimmed"]);
    hint.set_margin_top(SPACE_S);
    hint.set_halign(gtk::Align::Center);

    let warning = ui::boxed_list();
    warning.append(&ui::warning_row(&tr(
        "Dependencies and protection are ignored. Programs that need the package stop working.",
    )));

    let content = ui::vbox(SPACE_M);
    content.set_margin_top(SPACE_M);
    content.set_margin_bottom(SPACE_M);
    content.set_margin_start(SPACE_S);
    content.set_margin_end(SPACE_S);
    content.append(&warning);
    content.append(&search);
    content.append(&results);
    content.append(&hint);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&adw::Clamp::builder().maximum_size(560).child(&content).build())
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scrolled));
    let dialog = adw::Dialog::builder()
        .title(tr("Force Remove a Package"))
        .content_width(560)
        .content_height(620)
        .child(&toolbar)
        .focus_widget(&search)
        .build();

    let listing = Rc::new(listing);
    let (s, d) = (sender.clone(), dialog.clone());
    let (results2, hint2) = (results.clone(), hint.clone());
    search.connect_search_changed(move |e| {
        results2.remove_all();
        let found = find(&listing.packages, &e.text());
        hint2.set_visible(e.text().is_empty() || found.1 == 0);
        hint2.set_label(&if e.text().is_empty() {
            tr("Type part of a package name.")
        } else {
            tr("No packages found.")
        });
        for pkg in found.0 {
            let row = adw::ActionRow::builder()
                .title(format::escape(&pkg.name))
                .subtitle(format::escape(&format!("{} · {}", pkg.version, pkg.desc)))
                .subtitle_lines(1)
                .activatable(true)
                .build();
            if let Some(why) = listing.protected.get(&pkg.name) {
                let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
                lock.set_tooltip_text(Some(why));
                row.add_prefix(&lock);
            }
            row.add_suffix(&ui::label(&format_size(pkg.size), &["dimmed", "numeric"]));
            row.add_suffix(&ui::row_hint("go-next-symbolic"));
            let (s, d, name) = (s.clone(), d.clone(), pkg.name.clone());
            row.connect_activated(move |_| {
                d.close();
                s.input(Msg::ForcePlan(name.clone()));
            });
            results2.append(&row);
        }
        if found.1 > MAX_RESULTS {
            let more = trn(
                "{} more package. Refine the search.",
                "{} more packages. Refine the search.",
                (found.1 - MAX_RESULTS) as u64,
            );
            results2.append(&adw::ActionRow::builder().title(format::escape(&more)).build());
        }
        results2.set_visible(found.1 > 0);
    });
    results.set_visible(false);
    dialog.present(Some(root));
}

/// Packages matching `text`: names that start with it first, then names and descriptions that
/// contain it. Returns the first `MAX_RESULTS` and how many matched in all.
fn find<'a>(packages: &'a [InstalledPkg], text: &str) -> (Vec<&'a InstalledPkg>, usize) {
    let q = text.trim().to_lowercase();
    if q.is_empty() {
        return (Vec::new(), 0);
    }
    let mut prefix: Vec<&InstalledPkg> = Vec::new();
    let mut inside: Vec<&InstalledPkg> = Vec::new();
    for p in packages {
        if p.name.starts_with(&q) {
            prefix.push(p);
        } else if p.name.contains(&q) || p.desc.to_lowercase().contains(&q) {
            inside.push(p);
        }
    }
    let total = prefix.len() + inside.len();
    prefix.extend(inside);
    prefix.truncate(MAX_RESULTS);
    (prefix, total)
}

fn names(list: &[String]) -> String {
    let shown = list.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
    match list.len().saturating_sub(6) {
        0 => shown,
        more => trf("{} and {} more", &[&shown, &more.to_string()]),
    }
}

/// Fill `box_` with what the removal would do: what breaks, what is protected, which packages go.
fn describe(box_: &gtk::Box, plan: &RemovalPlan) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
    let warnings = ui::boxed_list();
    for (pkg, block) in &plan.blocked {
        warnings.append(&ui::warning_row(&block_text(pkg, block)));
    }
    for (pkg, needers) in &plan.broken {
        let mut n = needers.clone();
        n.sort();
        n.dedup();
        warnings.append(&ui::warning_row(&trf("Packages that need {} will break: {}", &[pkg, &names(&n)])));
    }
    for (pkg, w) in &plan.warnings {
        warnings.append(&ui::warning_row(&warning_text(pkg, w)));
    }
    if warnings.first_child().is_some() {
        box_.append(&warnings);
    }
    let list = ui::boxed_list();
    let exp = adw::ExpanderRow::builder()
        .title(tr("Packages"))
        .subtitle(format::escape(&trf("{} · {}", &[&plan.removed.len().to_string(), &format_size(plan.total_size)])))
        .build();
    for pkg in &plan.removed {
        exp.add_row(&ui::row_with_size(&pkg.name, &pkg.version, Some(pkg.size)));
    }
    list.append(&exp);
    box_.append(&list);
}

/// The confirmation: the consequences, two options, and the name to type.
pub fn confirm(p: Planned, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let name = p.name.clone();
    let (alone, with_deps) = match (p.alone, p.with_deps) {
        (Ok(a), Ok(w)) => (a, w),
        (Err(e), _) | (_, Err(e)) => {
            let d = adw::AlertDialog::new(Some(&trf("Cannot check “{}”", &[&name])), Some(&e));
            d.add_response("close", &tr("Close"));
            d.present(Some(root));
            return;
        }
    };

    let consequences = ui::vbox(SPACE_S);
    describe(&consequences, &alone);

    let options = ui::boxed_list();
    let extra = with_deps.removed.len().saturating_sub(alone.removed.len());
    let deps_row = adw::SwitchRow::builder()
        .title(tr("Also remove unneeded dependencies"))
        .subtitle(if extra == 0 {
            tr("Nothing else would be removed.")
        } else {
            trn("{} more component would be removed.", "{} more components would be removed.", extra as u64)
        })
        .sensitive(extra > 0)
        .build();
    let nosave_row = adw::SwitchRow::builder()
        .title(tr("Delete its configuration files too"))
        .subtitle(tr("Otherwise edited files in /etc are kept as .pacsave."))
        .build();
    options.append(&deps_row);
    options.append(&nosave_row);

    let entry = adw::EntryRow::builder().title(trf("Type “{}” to confirm", &[&name])).build();
    let confirm_list = ui::boxed_list();
    confirm_list.append(&entry);

    let body = ui::vbox(SPACE_S);
    body.append(&consequences);
    body.append(&options);
    body.append(&confirm_list);

    let d = adw::AlertDialog::new(
        Some(&trf("Force Remove “{}”?", &[&name])),
        Some(&tr("Dependencies and protection are ignored. The system may stop working.")),
    );
    d.set_extra_child(Some(&body));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("remove", &tr("Force Remove"));
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_response_enabled("remove", false);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");

    let plans = Rc::new((alone, with_deps));
    // Switching the dependencies on or off changes what is listed.
    let (c, pl) = (consequences.clone(), plans.clone());
    deps_row.connect_active_notify(move |r| describe(&c, if r.is_active() { &pl.1 } else { &pl.0 }));
    let (d2, n2) = (d.clone(), name.clone());
    entry.connect_changed(move |e| d2.set_response_enabled("remove", e.text() == n2));
    let (d3, n3) = (d.clone(), name.clone());
    entry.connect_entry_activated(move |e| {
        if e.text() == n3 {
            d3.emit_by_name::<()>("response", &[&"remove"]);
        }
    });

    let s = sender.clone();
    let (need_agent, app_ids) = (p.need_agent, p.app_ids);
    d.connect_response(None, move |_, r| {
        if r != "remove" {
            return;
        }
        let with = deps_row.is_active();
        let plan = if with { &plans.1 } else { &plans.0 };
        let mut req = JobRequest::new(Job {
            title: name.clone(),
            steps: vec![Step::PacmanForce {
                pkgs: plan.targets.clone(),
                fingerprint: plan.fingerprint.clone(),
                recursive: with,
                nosave: nosave_row.is_active(),
            }],
            need_agent,
        });
        req.heading = trf("Removing “{}”", &[&name]);
        req.done_text = trf("“{}” was removed", &[&name]);
        req.app_ids = app_ids.clone();
        s.input(Msg::StartJob(Box::new(req)));
    });
    d.present(Some(root));
    entry.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, desc: &str) -> InstalledPkg {
        InstalledPkg { name: name.into(), version: "1".into(), desc: desc.into(), size: 0, explicit: true }
    }

    #[test]
    fn search_puts_name_prefixes_first() {
        let all = [pkg("libfoo", "a library"), pkg("foo", "tool"), pkg("bar", "uses Foo inside"), pkg("baz", "x")];
        let (found, total) = find(&all, " FOO ");
        assert_eq!(total, 3);
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["foo", "libfoo", "bar"]);
        assert_eq!(find(&all, "").1, 0);
    }

    #[test]
    fn search_is_capped_but_counts_everything() {
        let all: Vec<InstalledPkg> = (0..100).map(|i| pkg(&format!("lib{i}"), "")).collect();
        let (found, total) = find(&all, "lib");
        assert_eq!((found.len(), total), (MAX_RESULTS, 100));
    }
}
