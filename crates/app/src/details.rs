//! The details page of an application ("card").

use crate::ui::{self, ICON_HERO, SPACE_XL, SPACE_XS};
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
            .label(tr("Uninstall…"))
            .valign(gtk::Align::Center)
            .action_name("win.uninstall-unit")
            .action_target(&glib::Variant::from(inst.key.as_str()))
            .build();
        b.add_css_class("flat");
        b.add_css_class("error");
        g.set_header_suffix(Some(&b));
    }
    g.add(&ui::property_row(&tr("Source"), &source_long(&inst.source)));
    if let Some(v) = &inst.version {
        g.add(&ui::property_row(&tr("Version"), v));
    }
    if let Some(s) = inst.size {
        g.add(&ui::property_row(&tr("Size"), &format_size(s)));
    }
    if let Some(t) = inst.installed_at {
        let mut v = format::date(t);
        if let Some(m) = method_text(inst) {
            v = format!("{v} · {m}");
        }
        g.add(&ui::property_row(&tr("Installed"), &v));
    }
    if let Some(u) = inst.updated_at.filter(|u| Some(*u) != inst.installed_at) {
        g.add(&ui::property_row(&tr("Updated"), &format::date(u)));
    }

    let tech = adw::ExpanderRow::builder().title(tr("Technical details")).build();
    let (kind, name) = match &inst.source {
        s if s.is_pacman() => (tr("Package"), inst.unit_name.clone()),
        Source::Flatpak { flatpak_ref, .. } => (tr("Flatpak reference"), flatpak_ref.clone()),
        Source::Snap { .. } => (tr("Snap"), inst.unit_name.clone()),
        _ => (tr("File"), inst.unit_name.clone()),
    };
    tech.add_row(&ui::property_row(&kind, &name));
    if let Source::Repo { repo, .. } = &inst.source {
        tech.add_row(&ui::property_row(&tr("Repository"), repo));
    }
    if let Some(p) = &inst.pkg {
        if let Some(packager) = &p.packager {
            tech.add_row(&ui::property_row(&tr("Packager"), packager));
        }
        if let Some(b) = p.build_date {
            tech.add_row(&ui::property_row(&tr("Built"), &format::date(b)));
        }
        if !p.depends.is_empty() {
            tech.add_row(&ui::property_row(&tr("Depends on"), &p.depends.join(", ")));
        }
        if !p.required_by.is_empty() {
            tech.add_row(&ui::property_row(&tr("Required by"), &p.required_by.join(", ")));
        }
    }
    if let Some(cmd) = &inst.install_command {
        tech.add_row(&ui::property_row(&tr("Install command"), cmd));
    }
    if let Some(url) = &inst.url {
        let r = ui::property_row(&tr("Website"), url);
        r.set_activatable(true);
        r.add_suffix(&ui::row_hint("adw-external-link-symbolic"));
        let url = url.clone();
        r.connect_activated(move |r| {
            gtk::UriLauncher::new(&url).launch(r.root().and_downcast_ref::<gtk::Window>(), gio_none(), |_| {});
        });
        tech.add_row(&r);
    }
    g.add(&tech);
    g
}

fn gio_none() -> Option<&'static relm4::gtk::gio::Cancellable> {
    None
}

pub fn build(app: &App) -> DetailsPage {
    let page_content = adw::PreferencesPage::new();

    // Hero: icon, name, summary, source and the main action.
    let hero = ui::vbox(SPACE_XS);
    hero.set_margin_top(SPACE_XL / 2);
    let icon = gtk::Image::builder().pixel_size(ICON_HERO).build();
    icons::set(&icon, app);
    icon.set_pixel_size(ICON_HERO);
    icon.add_css_class("icon-dropshadow");
    let title = gtk::Label::builder().label(&app.name).wrap(true).justify(gtk::Justification::Center).build();
    title.add_css_class("title-1");
    hero.append(&icon);
    hero.append(&title);
    if let Some(s) = &app.summary {
        let l =
            gtk::Label::builder().label(s).wrap(true).justify(gtk::Justification::Center).max_width_chars(48).build();
        l.add_css_class("dimmed");
        hero.append(&l);
    }
    let mut sources: Vec<String> = app.installs.iter().map(|i| source_short(&i.source)).collect();
    sources.dedup();
    let src = gtk::Label::new(Some(&sources.join(" + ")));
    src.add_css_class("caption");
    src.add_css_class("dimmed");
    hero.append(&src);

    let uninstall = ui::pill(&tr("Uninstall…"), ui::Kind::Destructive);
    uninstall.set_margin_top(ui::SPACE_S);
    uninstall.set_action_name(Some("win.uninstall"));
    uninstall.set_action_target_value(Some(&glib::Variant::from(app.id.as_str())));
    hero.append(&uninstall);
    let hero_group = adw::PreferencesGroup::new();
    hero_group.add(&hero);
    page_content.add(&hero_group);

    let banner = adw::Banner::builder().revealed(false).build();

    // One group per way of installation.
    let multiple = app.installs.len() > 1;
    for (n, inst) in app.installs.iter().enumerate() {
        let title = if multiple {
            trf("Installation {}: {}", &[&(n + 1).to_string(), &source_short(&inst.source)])
        } else {
            tr("Installation")
        };
        let g = install_group(inst, &title, multiple);
        if multiple && n == 0 {
            g.set_description(Some(&format::escape(&trf(
                "Installed in {} ways. Each one can be removed separately.",
                &[&app.installs.len().to_string()],
            ))));
        }
        page_content.add(&g);
    }

    // Files: a row opens the folder.
    let locations: Vec<_> = app.installs.iter().flat_map(|i| i.locations.iter()).collect();
    if !locations.is_empty() {
        let g = adw::PreferencesGroup::builder().title(tr("Files")).build();
        for loc in locations.iter().take(8) {
            let r = adw::ActionRow::builder().title(format::escape(&format::path(loc))).build();
            r.add_css_class("property");
            r.add_suffix(&ui::row_hint("folder-open-symbolic"));
            r.set_tooltip_text(Some(&tr("Open in file manager")));
            r.set_action_name(Some("win.open-path"));
            r.set_action_target_value(Some(&glib::Variant::from(loc.to_string_lossy().as_ref())));
            g.add(&r);
        }
        page_content.add(&g);
    }

    // Dependencies, filled later.
    let deps_group = adw::PreferencesGroup::builder()
        .title(tr("Installed along with it"))
        .description(tr("Calculating…"))
        .visible(app.installs.iter().any(|i| i.source.is_pacman()))
        .build();
    page_content.add(&deps_group);

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
        page_content.add(&g);
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
    page_content.add(&why);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_css_class("translucent");
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&page_content));
    let page = adw::NavigationPage::builder().title(&app.name).tag("details").child(&toolbar).build();
    DetailsPage { app_id: app.id.clone(), page, deps_group, banner }
}
