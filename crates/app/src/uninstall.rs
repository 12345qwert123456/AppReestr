//! The uninstall flow: choose what to remove, plan it, confirm it, run it, report.

use crate::format;
use crate::removal::{self, Event, Job, Step};
use crate::ui::{self, DIALOG_WIDTH, SPACE_L, SPACE_M, SPACE_S, SPACE_XS};
use crate::window::{AppModel, Cmd, Msg, ScanData};
use appreestr_core::i18n::{tr, trf, trn};
use appreestr_core::leftovers::{self, Confidence, Leftover};
use appreestr_core::model::{App, FlatpakScope, Installation, Source};
use appreestr_core::plan::{self, PlannedPkg, RemovalPlan};
use appreestr_core::protect::Protection;
use appreestr_core::reasons::{block_text, format_size, shortcut_label, source_short, warning_text};
use appreestr_core::rules::Rules;
use appreestr_core::sysroot::SysRoot;
use relm4::ComponentSender;
use relm4::adw::{self, prelude::*};
use relm4::gtk;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Result of planning, shown in the confirmation dialog.
pub struct Planned {
    /// The applications that go, each with the installations that are removed.
    pub items: Vec<(Arc<App>, Vec<Installation>)>,
    /// The user's home folder: what is inside it goes to the trash, the rest needs the helper.
    pub home: PathBuf,
    pub pacman: Option<Result<RemovalPlan, String>>,
    pub leftovers: Vec<Leftover>,
    pub need_agent: bool,
    pub orphans_before: Vec<String>,
    /// Selected applications that were left out, with the reason (protected, or others need them).
    pub skipped: Vec<(Arc<App>, String)>,
}

impl Planned {
    /// The name of the one application, or a count for several.
    fn subject(&self) -> String {
        match self.items.as_slice() {
            [(app, _)] => app.name.clone(),
            items => trn("{} application", "{} applications", items.len() as u64),
        }
    }

    pub fn units(&self) -> impl Iterator<Item = &Installation> {
        self.items.iter().flat_map(|(_, u)| u.iter())
    }
}

/// The progress dialog of a running job.
pub struct Progress {
    pub dialog: adw::Dialog,
    spinner: adw::Spinner,
    status: gtk::Label,
    log: gtk::TextBuffer,
    expander: gtk::Expander,
    close: gtk::Button,
    header: adw::HeaderBar,
    pub app_ids: Vec<String>,
    pub done_text: String,
    pub had_pacman: bool,
    pub had_flatpak: Option<FlatpakScope>,
    pub orphans_before: Vec<String>,
}

pub fn protection(sys: &SysRoot, rules: &Rules, data: &ScanData) -> Protection {
    let processes = appreestr_core::protect::processes(sys);
    appreestr_core::protect::analyze(&appreestr_core::protect::Input {
        sys,
        rules,
        owners: &data.owners,
        depends: &data.depends,
        hold_pkgs: &data.hold_pkgs,
        kernels: &data.kernels,
        processes: &processes,
    })
}

fn orphan_names() -> Vec<String> {
    plan::open_local("/", "/var/lib/pacman")
        .map(|h| plan::orphans(&h).into_iter().map(|p| p.name).collect())
        .unwrap_or_default()
}

fn info_dialog(root: &adw::ApplicationWindow, heading: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(heading), Some(body));
    d.add_response("close", &tr("Close"));
    d.present(Some(root));
}

/// Entry point: the user asked to uninstall an app (or one of its installations).
pub fn request(
    m: &mut AppModel,
    app_id: &str,
    unit: Option<String>,
    root: &adw::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
) {
    let Some(app) = m.apps.iter().find(|a| a.id == app_id).cloned() else { return };
    if m.progress.is_some() {
        return;
    }
    let units: Vec<Installation> = match &unit {
        Some(k) => app.installs.iter().filter(|i| i.key.as_str() == k).cloned().collect(),
        None => app.installs.clone(),
    };
    if let Some(Source::Shortcut { kind }) = units.first().map(|u| &u.source)
        && units.len() == 1
    {
        info_dialog(
            root,
            &trf("“{}” is managed by {}", &[&app.name, &shortcut_label(*kind)]),
            &tr("Remove it from the program that created it. AppReestr only shows such launchers."),
        );
        return;
    }
    if unit.is_none() && units.len() > 1 {
        choose_units(&app, root, sender);
        return;
    }
    plan_in_background(m, vec![(app, units)], sender);
}

/// Several applications ticked in selection mode: remove each of them completely.
pub fn request_many(
    m: &mut AppModel,
    ids: &[String],
    root: &adw::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
) {
    if m.progress.is_some() {
        return;
    }
    let items: Vec<(Arc<App>, Vec<Installation>)> = m
        .apps
        .iter()
        .filter(|a| ids.contains(&a.id) && a.is_removable())
        .map(|a| {
            // Launchers made by other programs are not ours to remove.
            let units = a.installs.iter().filter(|i| !matches!(i.source, Source::Shortcut { .. })).cloned().collect();
            (a.clone(), units)
        })
        .collect();
    match items.len() {
        0 => {}
        1 => {
            let id = items[0].0.id.clone();
            request(m, &id, None, root, sender);
        }
        _ => plan_in_background(m, items, sender),
    }
}

/// Several ways of installation: let the user pick which to remove.
fn choose_units(app: &Arc<App>, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let d = adw::AlertDialog::new(
        Some(&trf("Uninstall “{}”", &[&app.name])),
        Some(&tr("It is installed in several ways. Choose what to remove.")),
    );
    let list = ui::boxed_list();
    let mut checks = Vec::new();
    for inst in &app.installs {
        let check = gtk::CheckButton::builder().active(!matches!(inst.source, Source::Shortcut { .. })).build();
        let row = adw::ActionRow::builder()
            .title(format::escape(&source_short(&inst.source)))
            .subtitle(format::escape(&format!(
                "{} · {}",
                inst.unit_name,
                inst.size.map(format_size).unwrap_or_default()
            )))
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        list.append(&row);
        checks.push((inst.key.to_string(), check));
    }
    d.set_extra_child(Some(&list));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("next", &tr("Continue"));
    d.set_response_appearance("next", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("cancel"));
    let s = sender.clone();
    let id = app.id.clone();
    d.connect_response(None, move |_, r| {
        if r == "next" {
            let keys: Vec<String> = checks.iter().filter(|(_, c)| c.is_active()).map(|(k, _)| k.clone()).collect();
            if !keys.is_empty() {
                s.input(Msg::UninstallUnits(id.clone(), keys));
            }
        }
    });
    d.present(Some(root));
}

pub fn plan_units(m: &mut AppModel, app_id: &str, keys: &[String], sender: &ComponentSender<AppModel>) {
    let Some(app) = m.apps.iter().find(|a| a.id == app_id).cloned() else { return };
    let units = app.installs.iter().filter(|i| keys.iter().any(|k| k == i.key.as_str())).cloned().collect();
    plan_in_background(m, vec![(app, units)], sender);
}

fn pacman_names(units: &[Installation]) -> Vec<String> {
    units.iter().filter(|u| u.source.is_pacman()).map(|u| u.unit_name.clone()).collect()
}

/// Why one application of a bulk removal cannot go with the others, if it cannot.
fn skip_reason(plan: &RemovalPlan, selected_pkgs: &HashSet<String>) -> Option<String> {
    if let Some((pkg, b)) = plan.blocked.first() {
        return Some(block_text(pkg, b));
    }
    // Needed only by packages that are being removed as well: fine, the whole plan handles it.
    let outsiders = plan.needed_by.iter().any(|(_, needers)| needers.iter().any(|n| !selected_pkgs.contains(n)));
    if outsiders { blocked_reason(plan) } else { None }
}

fn plan_in_background(m: &AppModel, items: Vec<(Arc<App>, Vec<Installation>)>, sender: &ComponentSender<AppModel>) {
    let (sys, rules, data) = (m.sys.clone(), m.rules.clone(), m.data.clone());
    let chosen: HashSet<&str> = items.iter().map(|(a, _)| a.id.as_str()).collect();
    let others: Vec<Arc<App>> = m.apps.iter().filter(|a| !chosen.contains(a.id.as_str())).cloned().collect();
    sender.spawn_oneshot_command(move || {
        let prot = protection(&sys, &rules, &data);
        let mut handle = plan::open_local("/", "/var/lib/pacman").map_err(|e| e.to_string());
        let mut items = items;
        let mut skipped: Vec<(Arc<App>, String)> = Vec::new();

        // A protected application must not hold back the others: plan each one alone first.
        if items.len() > 1
            && let Ok(h) = handle.as_mut()
        {
            let all: HashSet<String> = items.iter().flat_map(|(_, u)| pacman_names(u)).collect();
            items.retain(|(app, units)| {
                let pkgs = pacman_names(units);
                if pkgs.is_empty() {
                    return true;
                }
                // An error here shows up again in the combined plan below.
                match plan::plan(h, &pkgs, true, Some(&prot)).ok().and_then(|p| skip_reason(&p, &all)) {
                    Some(reason) => {
                        skipped.push((app.clone(), reason));
                        false
                    }
                    None => true,
                }
            });
        }

        let mut pkgs: Vec<String> = Vec::new();
        for (_, units) in &items {
            for n in pacman_names(units) {
                if !pkgs.contains(&n) {
                    pkgs.push(n);
                }
            }
        }
        let pacman = (!pkgs.is_empty())
            .then(|| handle.and_then(|mut h| plan::plan(&mut h, &pkgs, true, Some(&prot)).map_err(|e| e.to_string())));

        // Leftovers only for applications that go away completely.
        let others: Vec<&App> = others.iter().map(|a| a.as_ref()).collect();
        let mut leftovers: Vec<Leftover> = Vec::new();
        for (app, units) in &items {
            if units.len() == app.installs.len() {
                for l in leftovers::find(&sys, app, &others) {
                    if !leftovers.iter().any(|x| x.path == l.path) {
                        leftovers.push(l);
                    }
                }
            }
        }
        let orphans_before = if pkgs.is_empty() { Vec::new() } else { orphan_names() };
        Cmd::Planned(Box::new(Planned {
            items,
            home: sys.home.clone(),
            pacman,
            leftovers,
            need_agent: !prot.polkit_agent,
            orphans_before,
            skipped,
        }))
    });
}

fn blocked_reason(plan: &RemovalPlan) -> Option<String> {
    if let Some((pkg, b)) = plan.blocked.first() {
        return Some(block_text(pkg, b));
    }
    if let Some((pkg, needers)) = plan.needed_by.first() {
        let mut n = needers.clone();
        n.sort();
        n.dedup();
        return Some(trf(
            "{} is needed by: {}. Remove those first, or keep it.",
            &[pkg, &n.iter().take(5).cloned().collect::<Vec<_>>().join(", ")],
        ));
    }
    None
}

/// The confirmation dialog (or an explanation why it cannot be removed).
pub fn confirm(p: Planned, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let name = p.subject();
    if p.items.is_empty() {
        let d = adw::AlertDialog::new(
            Some(&tr("These applications cannot be uninstalled")),
            Some(&tr("The system needs them, or other programs depend on them.")),
        );
        let list = ui::boxed_list();
        for (app, reason) in &p.skipped {
            list.append(&ui::warning_row(&trf("“{}”: {}", &[&app.name, reason])));
        }
        d.set_extra_child(Some(&list));
        d.add_response("close", &tr("Close"));
        d.present(Some(root));
        return;
    }
    if let Some(Err(e)) = &p.pacman {
        info_dialog(root, &trf("Cannot check “{}”", &[&name]), e);
        return;
    }
    if let Some(Ok(plan)) = &p.pacman
        && let Some(reason) = blocked_reason(plan)
    {
        info_dialog(root, &trf("“{}” cannot be uninstalled", &[&name]), &reason);
        return;
    }

    let mut steps: Vec<Step> = Vec::new();
    let mut size: u64 = 0;
    let mut warnings: Vec<String> = Vec::new();
    let extra = ui::vbox(SPACE_S);
    let what = ui::boxed_list();

    for (app, reason) in &p.skipped {
        warnings.push(trf("“{}” stays installed: {}", &[&app.name, reason]));
    }
    if p.items.len() > 1 {
        let exp = adw::ExpanderRow::builder()
            .title(tr("Applications"))
            .subtitle(format::escape(&p.items.len().to_string()))
            .build();
        for (app, _) in &p.items {
            let mut sources: Vec<String> = app.installs.iter().map(|i| source_short(&i.source)).collect();
            sources.dedup();
            exp.add_row(&ui::row_with_size(&app.name, &sources.join(" + "), app.total_size()));
        }
        what.append(&exp);
    }

    if let Some(Ok(plan)) = &p.pacman {
        size += plan.total_size;
        steps.push(Step::Pacman { pkgs: plan.targets.clone(), fingerprint: plan.fingerprint.clone(), nosave: false });
        let extra_pkgs: Vec<&PlannedPkg> = plan.extra().collect();
        let subtitle = if extra_pkgs.is_empty() {
            trf("{} · {}", &[&plan.removed.len().to_string(), &format_size(plan.total_size)])
        } else {
            trn(
                "{} component that nothing else needs will be removed too.",
                "{} components that nothing else needs will be removed too.",
                extra_pkgs.len() as u64,
            )
        };
        let exp = adw::ExpanderRow::builder().title(tr("Packages")).subtitle(format::escape(&subtitle)).build();
        for pkg in &plan.removed {
            exp.add_row(&ui::row_with_size(&pkg.name, &pkg.version, Some(pkg.size)));
        }
        what.append(&exp);
        for (pkg, w) in &plan.warnings {
            warnings.push(warning_text(pkg, w));
        }
        if !plan.optional_for.is_empty() {
            let users: Vec<String> = plan.optional_for.iter().flat_map(|(_, u)| u.clone()).collect();
            warnings.push(warning_text("", &appreestr_core::protect::Warning::OptionalFor(users)));
        }
    }
    let home = p.home.clone();
    for u in p.units() {
        match &u.source {
            Source::Flatpak { scope, flatpak_ref, .. } => {
                size += u.size.unwrap_or(0);
                steps.push(match scope {
                    FlatpakScope::User => Step::FlatpakUser { flatpak_ref: flatpak_ref.clone() },
                    FlatpakScope::System => Step::FlatpakSystem { flatpak_ref: flatpak_ref.clone() },
                });
                what.append(&ui::row_with_size("Flatpak", flatpak_ref, u.size));
            }
            Source::Snap { .. } => {
                size += u.size.unwrap_or(0);
                steps.push(Step::Snap { name: u.unit_name.clone() });
                what.append(&ui::row_with_size("Snap", &u.unit_name, u.size));
            }
            Source::AppImage | Source::Manual => {
                size += u.size.unwrap_or(0);
                let mut user_paths: Vec<PathBuf> = Vec::new();
                for loc in &u.locations {
                    if loc.starts_with(&home) {
                        user_paths.push(loc.clone());
                    } else {
                        steps.push(Step::RootPath { path: loc.clone() });
                    }
                    what.append(&ui::row_with_size(&format::path(loc), &source_short(&u.source), None));
                }
                // Launchers in the user's menu that belong to it.
                for e in &u.desktop_entries {
                    if e.path.starts_with(&home) && e.path.extension().is_some_and(|x| x == "desktop") {
                        user_paths.push(e.path.clone());
                    }
                }
                if let Some(icon) = &u.icon_path {
                    user_paths.push(icon.clone());
                }
                if !user_paths.is_empty() {
                    steps.push(Step::Trash { paths: user_paths });
                }
            }
            _ => {}
        }
    }

    // Leftovers: settings, data and cache in the home folder. Off until the user switches it on.
    let left_checks: Rc<RefCell<Vec<(PathBuf, gtk::CheckButton)>>> = Rc::new(RefCell::new(Vec::new()));
    let left_exp: Option<adw::ExpanderRow> = (!p.leftovers.is_empty()).then(|| {
        let total: u64 = p.leftovers.iter().filter(|l| l.confidence == Confidence::High).filter_map(|l| l.size).sum();
        let exp = adw::ExpanderRow::builder()
            .title(tr("Settings, data and cache"))
            .subtitle(format::escape(&trf("{} · moved to the trash, so they can be restored", &[&format_size(total)])))
            .show_enable_switch(true)
            .enable_expansion(false)
            .build();
        for l in &p.leftovers {
            let c = gtk::CheckButton::builder().active(l.confidence == Confidence::High).build();
            let sub = match l.confidence {
                Confidence::High => l.size.map(format_size).unwrap_or_else(|| tr("large")),
                Confidence::Low => {
                    format!("{} · {}", l.size.map(format_size).unwrap_or_default(), tr("possibly unrelated"))
                }
            };
            let r = adw::ActionRow::builder()
                .title(format::escape(&format::path(&l.path)))
                .subtitle(format::escape(&sub))
                .activatable_widget(&c)
                .build();
            r.add_prefix(&c);
            exp.add_row(&r);
            left_checks.borrow_mut().push((l.path.clone(), c));
        }
        what.append(&exp);
        exp
    });

    if !warnings.is_empty() {
        let list = ui::boxed_list();
        for w in &warnings {
            list.append(&ui::warning_row(w));
        }
        extra.append(&list);
    }
    extra.append(&what);

    let text = trf("This frees {}.", &[&format_size(size)]);
    let heading = match p.items.as_slice() {
        [_] => trf("Uninstall “{}”?", &[&name]),
        items => trn("Uninstall {} application?", "Uninstall {} applications?", items.len() as u64),
    };
    let d = adw::AlertDialog::new(Some(&heading), Some(&text));
    d.set_extra_child(Some(&extra));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("remove", &tr("Uninstall"));
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");

    let s = sender.clone();
    let had_pacman = p.pacman.is_some();
    let had_flatpak = p.units().find_map(|u| match u.source {
        Source::Flatpak { scope, .. } => Some(scope),
        _ => None,
    });
    let need_agent = p.need_agent;
    let app_ids: Vec<String> = p.items.iter().map(|(a, _)| a.id.clone()).collect();
    let orphans_before = p.orphans_before.clone();
    let count = p.items.len() as u64;
    d.connect_response(None, move |_, r| {
        if r != "remove" {
            return;
        }
        let mut steps = steps.clone();
        let checks = left_checks.borrow();
        if left_exp.as_ref().is_some_and(|e| e.enables_expansion()) {
            let paths: Vec<PathBuf> = checks.iter().filter(|(_, c)| c.is_active()).map(|(p, _)| p.clone()).collect();
            if !paths.is_empty() {
                steps.push(Step::Trash { paths });
            }
        }
        let mut req = JobRequest::new(Job { title: name.clone(), steps, need_agent });
        if count > 1 {
            req.heading = trn("Uninstalling {} application", "Uninstalling {} applications", count);
            req.done_text = trn("{} application was uninstalled", "{} applications were uninstalled", count);
        }
        req.app_ids = app_ids.clone();
        req.had_pacman = had_pacman;
        req.had_flatpak = had_flatpak;
        req.orphans_before = orphans_before.clone();
        s.input(Msg::StartJob(Box::new(req)));
    });
    d.present(Some(root));
}

pub struct JobRequest {
    pub job: Job,
    /// Applications that disappear when the job succeeds.
    pub app_ids: Vec<String>,
    pub had_pacman: bool,
    pub had_flatpak: Option<FlatpakScope>,
    pub orphans_before: Vec<String>,
    /// Title of the progress dialog.
    pub heading: String,
    /// Shown in a toast when the job succeeds.
    pub done_text: String,
}

impl JobRequest {
    /// A job on one named thing; set the other fields when there is more to say.
    pub fn new(job: Job) -> Self {
        Self {
            heading: trf("Uninstalling “{}”", &[&job.title]),
            done_text: trf("“{}” was uninstalled", &[&job.title]),
            job,
            app_ids: Vec::new(),
            had_pacman: false,
            had_flatpak: None,
            orphans_before: Vec::new(),
        }
    }
}

impl std::fmt::Debug for JobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JobRequest({})", self.job.title)
    }
}

/// Show the progress dialog and start the worker.
pub fn start(m: &mut AppModel, req: JobRequest, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let spinner = adw::Spinner::builder().width_request(48).height_request(48).build();
    let status = gtk::Label::builder().label(tr("Preparing…")).wrap(true).justify(gtk::Justification::Center).build();
    status.add_css_class("title-4");
    let log = gtk::TextBuffer::new(None);
    let view = gtk::TextView::builder()
        .buffer(&log)
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(SPACE_XS)
        .right_margin(SPACE_XS)
        .top_margin(SPACE_XS)
        .bottom_margin(SPACE_XS)
        .build();
    let scrolled = gtk::ScrolledWindow::builder().child(&view).min_content_height(160).build();
    scrolled.add_css_class("card");
    let expander = gtk::Expander::builder().label(tr("Details")).child(&scrolled).build();
    let close = ui::pill(&tr("Close"), ui::Kind::Normal);
    close.set_visible(false);
    let content = ui::vbox(SPACE_M);
    content.set_margin_top(SPACE_L);
    content.set_margin_bottom(SPACE_L);
    content.set_margin_start(SPACE_L);
    content.set_margin_end(SPACE_L);
    content.append(&spinner);
    content.append(&status);
    content.append(&expander);
    content.append(&close);
    // No way to close it while the job runs; the title buttons come back after a failure.
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let dialog =
        adw::Dialog::builder().title(&req.heading).content_width(DIALOG_WIDTH).can_close(false).child(&toolbar).build();
    let d2 = dialog.clone();
    close.connect_clicked(move |_| {
        d2.set_can_close(true);
        d2.close();
    });
    dialog.present(Some(root));

    m.progress = Some(Progress {
        dialog,
        spinner,
        status,
        log,
        expander,
        close,
        header,
        app_ids: req.app_ids,
        done_text: req.done_text,
        had_pacman: req.had_pacman,
        had_flatpak: req.had_flatpak,
        orphans_before: req.orphans_before,
    });
    let job = req.job;
    sender.spawn_command(move |out| {
        let out = Arc::new(out);
        removal::run(job, move |e| {
            let _ = out.send(Cmd::Removal(e));
        });
    });
}

/// Events from the worker.
pub fn on_event(
    m: &mut AppModel,
    e: Event,
    root: &adw::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
    toasts: &adw::ToastOverlay,
) {
    let Some(p) = &m.progress else { return };
    match e {
        Event::Status(s) => p.status.set_label(&s),
        Event::Log(line) => {
            let mut end = p.log.end_iter();
            p.log.insert(&mut end, &format!("{line}\n"));
        }
        Event::Password { message, reply } => ask_password(root, &message, reply),
        Event::Done(Ok(())) => {
            let p = m.progress.take().unwrap();
            p.dialog.set_can_close(true);
            p.dialog.close();
            toasts.add_toast(adw::Toast::new(&p.done_text));
            sender.input(Msg::Removed(p.app_ids.clone()));
            follow_up(m, p, sender);
        }
        Event::Done(Err(msg)) => {
            let p = m.progress.take().unwrap();
            p.spinner.set_visible(false);
            p.status.set_label(&msg);
            p.expander.set_expanded(p.log.char_count() > 0);
            p.close.set_visible(true);
            p.header.set_show_end_title_buttons(true);
            p.dialog.set_can_close(true);
            sender.input(Msg::Refresh);
        }
    }
}

/// After a successful removal: look for components nothing needs any more.
fn follow_up(m: &mut AppModel, p: Progress, sender: &ComponentSender<AppModel>) {
    let _ = m;
    let before = p.orphans_before;
    let had_pacman = p.had_pacman;
    let flatpak = p.had_flatpak;
    sender.spawn_oneshot_command(move || {
        let new_orphans = if had_pacman {
            plan::open_local("/", "/var/lib/pacman")
                .map(|h| plan::orphans(&h).into_iter().filter(|o| !before.contains(&o.name)).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Cmd::FollowUp { orphans: new_orphans, flatpak }
    });
}

pub fn ask_password(root: &adw::ApplicationWindow, message: &str, reply: std::sync::mpsc::Sender<Option<String>>) {
    let d = adw::AlertDialog::new(Some(&tr("Authentication Required")), Some(message));
    let entry = adw::PasswordEntryRow::builder().title(tr("Password")).build();
    let list = ui::boxed_list();
    list.append(&entry);
    d.set_extra_child(Some(&list));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("ok", &tr("Authenticate"));
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let reply = Rc::new(RefCell::new(Some(reply)));
    let (e2, r2) = (entry.clone(), reply.clone());
    d.connect_response(None, move |_, r| {
        if let Some(tx) = r2.borrow_mut().take() {
            let pw = (r == "ok").then(|| e2.text().to_string());
            e2.set_text("");
            let _ = tx.send(pw);
        }
    });
    let d2 = d.clone();
    entry.connect_entry_activated(move |_| d2.emit_by_name::<()>("response", &[&"ok"]));
    d.present(Some(root));
    entry.grab_focus();
}

/// Offer to remove components that became unnecessary.
pub fn offer_cleanup(
    orphans: Vec<PlannedPkg>,
    flatpak: Option<FlatpakScope>,
    toasts: &adw::ToastOverlay,
    sender: &ComponentSender<AppModel>,
) {
    if !orphans.is_empty() {
        let size: u64 = orphans.iter().map(|o| o.size).sum();
        let t = adw::Toast::builder()
            .title(
                trn("{} unused component left", "{} unused components left", orphans.len() as u64)
                    + &format!(" · {}", format_size(size)),
            )
            .button_label(tr("Review"))
            .timeout(10)
            .build();
        let s = sender.clone();
        let names: Vec<String> = orphans.into_iter().map(|o| o.name).collect();
        t.connect_button_clicked(move |_| s.input(Msg::CleanPackages(names.clone())));
        toasts.add_toast(t);
    }
    if let Some(scope) = flatpak {
        let t = adw::Toast::builder()
            .title(tr("Flatpak runtimes may be unused now"))
            .button_label(tr("Clean Up"))
            .timeout(10)
            .build();
        let s = sender.clone();
        t.connect_button_clicked(move |_| s.input(Msg::CleanFlatpak(scope)));
        toasts.add_toast(t);
    }
}

/// Plan the removal of a list of packages (unused components) and confirm it.
pub fn clean_packages(m: &AppModel, names: Vec<String>, sender: &ComponentSender<AppModel>) {
    let (sys, rules, data) = (m.sys.clone(), m.rules.clone(), m.data.clone());
    sender.spawn_oneshot_command(move || {
        let prot = protection(&sys, &rules, &data);
        let plan = plan::open_local("/", "/var/lib/pacman")
            .and_then(|mut h| plan::plan(&mut h, &names, true, Some(&prot)))
            .map_err(|e| e.to_string());
        Cmd::CleanPlanned(Box::new((plan, !prot.polkit_agent)))
    });
}

pub fn confirm_clean(
    plan: Result<RemovalPlan, String>,
    need_agent: bool,
    root: &adw::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
) {
    let plan = match plan {
        Ok(p) => p,
        Err(e) => return info_dialog(root, &tr("Cannot check unused components"), &e),
    };
    if let Some(reason) = blocked_reason(&plan) {
        return info_dialog(root, &tr("Unused components cannot be removed"), &reason);
    }
    let d = adw::AlertDialog::new(
        Some(&tr("Remove unused components?")),
        Some(&trf("Nothing uses them any more. This frees {}.", &[&format_size(plan.total_size)])),
    );
    let lb = ui::boxed_list();
    let exp = adw::ExpanderRow::builder().title(tr("Packages")).subtitle(plan.removed.len().to_string()).build();
    for p in &plan.removed {
        exp.add_row(&ui::row_with_size(&p.name, &p.version, Some(p.size)));
    }
    lb.append(&exp);
    d.set_extra_child(Some(&lb));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("remove", &tr("Remove"));
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_close_response("cancel");
    let s = sender.clone();
    d.connect_response(None, move |_, r| {
        if r == "remove" {
            s.input(Msg::StartJob(Box::new(JobRequest::new(Job {
                title: tr("Unused components"),
                steps: vec![Step::Pacman {
                    pkgs: plan.targets.clone(),
                    fingerprint: plan.fingerprint.clone(),
                    nosave: false,
                }],
                need_agent,
            }))));
        }
    });
    d.present(Some(root));
}

/// Details page: show what goes away together with the app, and whether it is protected.
pub fn on_details_shown(m: &mut AppModel, sender: &ComponentSender<AppModel>) {
    let Some(d) = &m.details else { return };
    let Some(app) = m.apps.iter().find(|a| a.id == d.app_id).cloned() else { return };
    let pkgs: Vec<String> = app.installs.iter().filter(|u| u.source.is_pacman()).map(|u| u.unit_name.clone()).collect();
    if pkgs.is_empty() {
        return;
    }
    let (sys, rules, data) = (m.sys.clone(), m.rules.clone(), m.data.clone());
    let id = app.id.clone();
    sender.spawn_oneshot_command(move || {
        let prot = protection(&sys, &rules, &data);
        let plan = plan::open_local("/", "/var/lib/pacman")
            .and_then(|mut h| plan::plan(&mut h, &pkgs, true, Some(&prot)))
            .map_err(|e| e.to_string());
        Cmd::DetailsPlan(id, Box::new(plan))
    });
}

pub fn fill_details(m: &AppModel, app_id: &str, plan: Result<RemovalPlan, String>) {
    let Some(d) = &m.details else { return };
    if d.app_id != app_id {
        return;
    }
    let plan = match plan {
        Ok(p) => p,
        Err(e) => {
            d.deps_group.set_description(Some(&e));
            return;
        }
    };
    if let Some(reason) = blocked_reason(&plan) {
        d.banner.set_title(&reason);
        d.banner.set_revealed(true);
    }
    let extra: Vec<&PlannedPkg> = plan.extra().collect();
    if extra.is_empty() {
        d.deps_group.set_description(Some(&tr("Nothing else: it brought no components of its own.")));
        return;
    }
    let size: u64 = extra.iter().map(|p| p.size).sum();
    d.deps_group.set_description(Some(&tr("Removed together with it, because nothing else needs them.")));
    let exp = adw::ExpanderRow::builder()
        .title(trn("{} component", "{} components", extra.len() as u64))
        .subtitle(format_size(size))
        .build();
    for p in extra {
        exp.add_row(&ui::row_with_size(&p.name, &p.version, Some(p.size)));
    }
    d.deps_group.add(&exp);
}
