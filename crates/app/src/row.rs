//! One row of the application list.

use crate::ui::{self, ICON_ROW, SPACE_S, SPACE_XS};
use crate::{format, icons};
use appreestr_core::i18n::{tr, trf};
use appreestr_core::model::{App, Visibility};
use appreestr_core::reasons::{format_size, source_short};
use relm4::gtk::{self, gio, glib, prelude::*};
use relm4::typed_view::list::RelmListItem;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

/// Selection mode: which applications are ticked. Shared by the window and the rows; the rows
/// that are on screen are updated in place, so the list keeps its scroll position.
#[derive(Clone)]
pub struct Selection(Rc<SelectionInner>);

struct SelectionInner {
    active: Cell<bool>,
    ids: RefCell<HashSet<String>>,
    /// Set while the state is pushed into the check buttons, so their signal is ignored.
    syncing: Cell<bool>,
    /// The check buttons of the rows that are on screen, with the application they show.
    bound: RefCell<Vec<(gtk::CheckButton, String)>>,
    changed: RefCell<Option<Box<dyn Fn()>>>,
}

impl Selection {
    pub fn new() -> Self {
        Selection(Rc::new(SelectionInner {
            active: Cell::new(false),
            ids: RefCell::new(HashSet::new()),
            syncing: Cell::new(false),
            bound: RefCell::new(Vec::new()),
            changed: RefCell::new(None),
        }))
    }

    /// Called after every change of the ticked set.
    pub fn on_change(&self, f: impl Fn() + 'static) {
        *self.0.changed.borrow_mut() = Some(Box::new(f));
    }

    fn notify(&self) {
        if let Some(f) = &*self.0.changed.borrow() {
            f();
        }
    }

    pub fn active(&self) -> bool {
        self.0.active.get()
    }

    /// Enter or leave selection mode; leaving forgets what was ticked.
    pub fn set_active(&self, on: bool) {
        if self.0.active.replace(on) == on {
            return;
        }
        if !on {
            self.0.ids.borrow_mut().clear();
        }
        self.push_to_rows();
        self.notify();
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.ids.borrow().contains(id)
    }

    pub fn len(&self) -> usize {
        self.0.ids.borrow().len()
    }

    pub fn ids(&self) -> Vec<String> {
        self.0.ids.borrow().iter().cloned().collect()
    }

    pub fn toggle(&self, id: &str) {
        self.set(id, !self.contains(id));
    }

    pub fn set(&self, id: &str, on: bool) {
        let changed =
            if on { self.0.ids.borrow_mut().insert(id.to_string()) } else { self.0.ids.borrow_mut().remove(id) };
        if changed {
            self.push_to_rows();
            self.notify();
        }
    }

    /// Replace the ticked set (select all, select none, or drop applications that are gone).
    pub fn replace(&self, ids: impl IntoIterator<Item = String>) {
        let new: HashSet<String> = ids.into_iter().collect();
        if *self.0.ids.borrow() != new {
            *self.0.ids.borrow_mut() = new;
            self.push_to_rows();
            self.notify();
        }
    }

    fn push_to_rows(&self) {
        self.0.syncing.set(true);
        for (check, id) in self.0.bound.borrow().iter() {
            check.set_visible(self.active());
            check.set_active(self.contains(id));
        }
        self.0.syncing.set(false);
    }

    fn bind(&self, check: &gtk::CheckButton, id: &str) {
        let mut bound = self.0.bound.borrow_mut();
        bound.retain(|(c, _)| c != check);
        bound.push((check.clone(), id.to_string()));
        drop(bound);
        self.0.syncing.set(true);
        check.set_visible(self.active());
        check.set_active(self.contains(id));
        self.0.syncing.set(false);
    }

    fn unbind(&self, check: &gtk::CheckButton) {
        self.0.bound.borrow_mut().retain(|(c, _)| c != check);
    }

    /// The user ticked a row's check button.
    fn ticked(&self, id: &str, on: bool) {
        if self.0.syncing.get() {
            return;
        }
        let changed =
            if on { self.0.ids.borrow_mut().insert(id.to_string()) } else { self.0.ids.borrow_mut().remove(id) };
        if changed {
            self.notify();
        }
    }
}

pub struct AppRow {
    pub app: Arc<App>,
    pub select: Selection,
}

pub struct RowWidgets {
    check: gtk::CheckButton,
    ticked: Option<glib::SignalHandlerId>,
    icon: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    size: gtk::Label,
    date: gtk::Label,
    menu: gtk::MenuButton,
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
        let root = ui::hbox(SPACE_S);
        root.set_margin_top(SPACE_XS);
        root.set_margin_bottom(SPACE_XS);
        let check = gtk::CheckButton::builder().valign(gtk::Align::Center).visible(false).build();
        let icon = gtk::Image::builder().pixel_size(ICON_ROW).build();
        icon.add_css_class("icon-dropshadow");
        let text = ui::vbox(0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let title = ui::label("", &["heading"]);
        let subtitle = ui::label("", &["dimmed", "caption"]);
        text.append(&title);
        text.append(&subtitle);
        let meta = ui::vbox(0);
        meta.set_valign(gtk::Align::Center);
        let size = ui::label("", &["numeric"]);
        size.set_xalign(1.0);
        let date = ui::label("", &["dimmed", "caption", "numeric"]);
        date.set_xalign(1.0);
        meta.append(&size);
        meta.append(&date);
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Actions"))
            .build();
        menu.add_css_class("flat");
        menu.update_property(&[gtk::accessible::Property::Label(&tr("Actions"))]);
        root.append(&check);
        root.append(&icon);
        root.append(&text);
        root.append(&meta);
        root.append(&menu);
        (root, RowWidgets { check, ticked: None, icon, title, subtitle, size, date, menu })
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
        w.menu.update_property(&[gtk::accessible::Property::Label(&trf("Actions for “{}”", &[&app.name]))]);

        w.check.set_sensitive(app.is_removable());
        w.check.update_property(&[gtk::accessible::Property::Label(&trf("Select “{}”", &[&app.name]))]);
        self.select.bind(&w.check, &app.id);
        let (select, id) = (self.select.clone(), app.id.clone());
        w.ticked = Some(w.check.connect_toggled(move |c| select.ticked(&id, c.is_active())));
    }

    fn unbind(&mut self, w: &mut RowWidgets, _root: &mut gtk::Box) {
        if let Some(h) = w.ticked.take() {
            w.check.disconnect(h);
        }
        self.select.unbind(&w.check);
    }
}
