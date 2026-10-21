//! The details page of an application ("card").

use crate::{format, icons};
use appreestr_core::i18n::{tr, trf};
use appreestr_core::model::{App, Installation, Source, Visibility};
use appreestr_core::reasons::{describe, format_size, source_long, source_short};
use appreestr_core::sources::pacman_log::{InstallMethod, install_method};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

/// Widgets the window fills in later (plans are computed in the background).
pub struct DetailsPage {
    pub app_id: String,
    pub page: adw::NavigationPage,
    /// "Installed along with it" group, filled when the removal plan is known.
    pub deps_group: adw::PreferencesGroup,
    /// Shown when removal is blocked or needs a warning.
    pub banner: adw::Banner,
}

fn row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(format::escape(title))
        .subtitle(format::escape(value))
        .subtitle_selectable(true)
        .build();
    r.add_css_class("property");
    r
}

fn action_button(icon: &str, tooltip: &str, action: &str, target: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .action_name(action)
        .action_target(&glib::Variant::from(target))
        .build();
    b.add_css_class("flat");
    b
}

pub fn method_text(inst: &Installation) -> Option<String> {
    let cmd = inst.install_command.as_deref()?;
    let with_system = inst.pkg.as_ref().is_some_and(|p| p.installed_with_system);
    if with_system {
        return Some(tr("together with the system"));
    }
    Some(match install_method(cmd, &inst.unit_name, with_system) {
        InstallMethod::Installer => tr("together with the system"),
        InstallMethod::Repo { requested: true, .. } => tr("installed on request"),
        InstallMethod::Repo { requested: false, targets } if !targets.is_empty() => {
            trf("came with {}", &[&targets.iter().take(3).cloned().collect::<Vec<_>>().join(", ")])
        }
        InstallMethod::LocalFile { helper: Some(h) } => trf("built with {}", &[&h]),
        InstallMethod::LocalFile { helper: None } => tr("from a package file"),
        _ => return None,
    })
}

fn install_group(inst: &Installation, title: &str, show_uninstall: bool) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(format::escape(title)).build();
    if show_uninstall {
        let b = gtk::Button::builder()
            .label(tr("Uninstall"))
            .valign(gtk::Align::Center)
            .action_name("win.uninstall-unit")
            .action_target(&glib::Variant::from(inst.key.as_str()))
            .build();
        b.add_css_class("destructive-action");
        g.set_header_suffix(Some(&b));
    }
    g.add(&row(&tr("Source"), &source_long(&inst.source)));
    if let Some(v) = &inst.version {
        g.add(&row(&tr("Version"), v));
    }
    if let Some(s) = inst.size {
        g.add(&row(&tr("Size"), &format_size(s)));
    }
    if let Some(t) = inst.installed_at {
        let mut v = format::date(t);
        if let Some(m) = method_text(inst) {
            v = format!("{v} · {m}");
        }
        g.add(&row(&tr("Installed"), &v));
    }
    if let Some(u) = inst.updated_at.filter(|u| Some(*u) != inst.installed_at) {
        g.add(&row(&tr("Updated"), &format::date(u)));
    }

    let tech = adw::ExpanderRow::builder().title(tr("Technical details")).build();
    let (kind, name) = match &inst.source {
        s if s.is_pacman() => (tr("Package"), inst.unit_name.clone()),
        Source::Flatpak { flatpak_ref, .. } => (tr("Flatpak reference"), flatpak_ref.clone()),
        Source::Snap { .. } => (tr("Snap"), inst.unit_name.clone()),
        _ => (tr("File"), inst.unit_name.clone()),
    };
    tech.add_row(&row(&kind, &name));
    if let Source::Repo { repo, .. } = &inst.source {
        tech.add_row(&row(&tr("Repository"), repo));
    }
    if let Some(p) = &inst.pkg {
        if let Some(packager) = &p.packager {
            tech.add_row(&row(&tr("Packager"), packager));
        }
        if let Some(b) = p.build_date {
            tech.add_row(&row(&tr("Built"), &format::date(b)));
        }
        if !p.depends.is_empty() {
            tech.add_row(&row(&tr("Depends on"), &p.depends.join(", ")));
        }
        if !p.required_by.is_empty() {
            tech.add_row(&row(&tr("Required by"), &p.required_by.join(", ")));
        }
    }
    if let Some(cmd) = &inst.install_command {
        tech.add_row(&row(&tr("Install command"), cmd));
    }
    if let Some(url) = &inst.url {
        let r = row(&tr("Website"), url);
        let b = gtk::Button::builder()
            .icon_name("adw-external-link-symbolic")
            .tooltip_text(tr("Open website"))
            .valign(gtk::Align::Center)
            .build();
        b.add_css_class("flat");
        let url = url.clone();
        b.connect_clicked(move |b| {
            gtk::UriLauncher::new(&url).launch(b.root().and_downcast_ref::<gtk::Window>(), gio_none(), |_| {});
        });
        r.add_suffix(&b);
        tech.add_row(&r);
    }
    g.add(&tech);
    g
}

fn gio_none() -> Option<&'static relm4::gtk::gio::Cancellable> {
    None
}

pub fn build(app: &App) -> DetailsPage {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Header: icon, name, summary, uninstall.
    let header = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    let icon = gtk::Image::builder().pixel_size(96).build();
    icons::set(&icon, app);
    icon.set_pixel_size(96);
    icon.add_css_class("icon-dropshadow");
    let title = gtk::Label::builder().label(&app.name).wrap(true).justify(gtk::Justification::Center).build();
    title.add_css_class("title-1");
    header.append(&icon);
    header.append(&title);
    if let Some(s) = &app.summary {
        let l = gtk::Label::builder().label(s).wrap(true).justify(gtk::Justification::Center).build();
        l.add_css_class("dim-label");
        header.append(&l);
    }
    let mut sources: Vec<String> = app.installs.iter().map(|i| source_short(&i.source)).collect();
    sources.dedup();
    let src = gtk::Label::new(Some(&sources.join(" + ")));
    src.add_css_class("caption");
    src.add_css_class("dim-label");
    header.append(&src);

    let uninstall = gtk::Button::builder()
        .label(tr("Uninstall"))
        .halign(gtk::Align::Center)
        .margin_top(12)
        .action_name("win.uninstall")
        .action_target(&glib::Variant::from(app.id.as_str()))
        .build();
    uninstall.add_css_class("destructive-action");
    uninstall.add_css_class("pill");
    header.append(&uninstall);
    content.append(&header);

    let banner = adw::Banner::builder().revealed(false).build();

    // One group per way of installation.
    let multiple = app.installs.len() > 1;
    if multiple {
        let note = gtk::Label::builder()
            .label(trf("Installed in {} ways. Each one can be removed separately.", &[&app.installs.len().to_string()]))
            .wrap(true)
            .build();
        note.add_css_class("dim-label");
        content.append(&note);
    }
    for (n, inst) in app.installs.iter().enumerate() {
        let title = if multiple {
            trf("Installation {}: {}", &[&(n + 1).to_string(), &source_short(&inst.source)])
        } else {
            tr("Installation")
        };
        content.append(&install_group(inst, &title, multiple));
    }

    // Files.
    let locations: Vec<_> = app.installs.iter().flat_map(|i| i.locations.iter()).collect();
    if !locations.is_empty() {
        let g = adw::PreferencesGroup::builder().title(tr("Files")).build();
        for loc in locations.iter().take(8) {
            let r = adw::ActionRow::builder().title(format::escape(&format::path(loc))).build();
            r.add_css_class("property");
            let s = loc.to_string_lossy();
            r.add_suffix(&action_button("folder-open-symbolic", &tr("Open in file manager"), "win.open-path", &s));
            g.add(&r);
        }
        content.append(&g);
    }

    // Dependencies, filled later.
    let deps_group = adw::PreferencesGroup::builder()
        .title(tr("Installed along with it"))
        .description(tr("Calculating…"))
        .visible(app.installs.iter().any(|i| i.source.is_pacman()))
        .build();
    content.append(&deps_group);

    // Launchers.
    let entries: Vec<_> = app.installs.iter().flat_map(|i| i.desktop_entries.iter()).collect();
    if !entries.is_empty() {
        let g = adw::PreferencesGroup::builder().title(tr("Launchers")).build();
        for e in entries {
            let subtitle = if e.is_visible() { e.id.clone() } else { format!("{} · {}", e.id, tr("hidden")) };
            let r =
                adw::ActionRow::builder().title(format::escape(&e.name)).subtitle(format::escape(&subtitle)).build();
            g.add(&r);
        }
        content.append(&g);
    }

    // Why it is (not) listed.
    let why = adw::PreferencesGroup::builder().title(tr("Why is it listed here?")).build();
    let (label, action) = match (app.user_mark, app.visibility) {
        (Some(_), _) => (tr("Undo My Choice"), "win.unmark"),
        (None, Visibility::App) => (tr("Hide From List"), "win.mark-hidden"),
        (None, _) => (tr("Show as Application"), "win.mark-app"),
    };
    let mark = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .action_name(action)
        .action_target(&glib::Variant::from(app.id.as_str()))
        .build();
    mark.add_css_class("flat");
    why.set_header_suffix(Some(&mark));
    for r in &app.reasons {
        why.add(&adw::ActionRow::builder().title(format::escape(&describe(r))).build());
    }
    content.append(&why);

    let clamp = adw::Clamp::builder().maximum_size(720).child(&content).build();
    let scrolled =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    body.append(&banner);
    body.append(&scrolled);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&body));
    let page = adw::NavigationPage::builder().title(&app.name).tag("details").child(&toolbar).build();
    DetailsPage { app_id: app.id.clone(), page, deps_group, banner }
}
