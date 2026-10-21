//! One row of the application list.

use crate::{format, icons};
use appreestr_core::i18n::tr;
use appreestr_core::model::{App, Visibility};
use appreestr_core::reasons::{format_size, source_short};
use relm4::gtk::{self, gio, glib, prelude::*};
use relm4::typed_view::list::RelmListItem;
use std::sync::Arc;

pub struct AppRow {
    pub app: Arc<App>,
}

pub struct RowWidgets {
    icon: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    size: gtk::Label,
    date: gtk::Label,
    menu: gtk::MenuButton,
}

fn label(classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// Row menu: actions on the window, targeted at the app id.
pub fn row_menu(app: &App) -> gio::Menu {
    let app_id = app.id.as_str();
    let menu = gio::Menu::new();
    let target = glib::Variant::from(app_id);
    let item = |label: &str, action: &str| {
        let i = gio::MenuItem::new(Some(label), None);
        i.set_action_and_target_value(Some(action), Some(&target));
        i
    };
    menu.append_item(&item(&tr("Details"), "win.details"));
    menu.append_item(&item(&tr("Open Location"), "win.open-location"));
    let marks = gio::Menu::new();
    match (app.user_mark, app.visibility) {
        (Some(_), _) => marks.append_item(&item(&tr("Undo My Choice"), "win.unmark")),
        (None, Visibility::App) => marks.append_item(&item(&tr("Hide From List"), "win.mark-hidden")),
        (None, _) => marks.append_item(&item(&tr("Show as Application"), "win.mark-app")),
    }
    menu.append_section(None, &marks);
    let danger = gio::Menu::new();
    danger.append_item(&item(&tr("Uninstall…"), "win.uninstall"));
    menu.append_section(None, &danger);
    menu
}

impl RelmListItem for AppRow {
    type Root = gtk::Box;
    type Widgets = RowWidgets;

    fn setup(_item: &gtk::ListItem) -> (gtk::Box, RowWidgets) {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(6)
            .margin_end(6)
            .build();
        let icon = gtk::Image::builder().pixel_size(32).build();
        icon.add_css_class("icon-dropshadow");
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        let title = label(&["heading"]);
        let subtitle = label(&["dim-label", "caption"]);
        text.append(&title);
        text.append(&subtitle);
        let meta = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        let size = label(&["numeric"]);
        size.set_xalign(1.0);
        let date = label(&["dim-label", "caption", "numeric"]);
        date.set_xalign(1.0);
        meta.append(&size);
        meta.append(&date);
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Actions"))
            .build();
        menu.add_css_class("flat");
        root.append(&icon);
        root.append(&text);
        root.append(&meta);
        root.append(&menu);
        (root, RowWidgets { icon, title, subtitle, size, date, menu })
    }

    fn bind(&mut self, w: &mut RowWidgets, _root: &mut gtk::Box) {
        let app = &self.app;
        icons::set(&w.icon, app);
        w.title.set_label(&app.name);
        let mut sources: Vec<String> = app.installs.iter().map(|i| source_short(&i.source)).collect();
        sources.dedup();
        let mut sub = sources.join(" + ");
        if let Some(v) = &app.primary().version {
            sub.push_str(" · ");
            sub.push_str(v);
        }
        w.subtitle.set_label(&sub);
        w.size.set_label(&app.total_size().map(format_size).unwrap_or_default());
        w.date.set_label(&app.installed_at().map(format::date).unwrap_or_default());
        w.menu.set_menu_model(Some(&row_menu(app)));
    }
}
