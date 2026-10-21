//! Shared layout metrics and widget factories, so every screen uses the same spacing, sizes and styles.
//!
//! Spacing follows the GNOME 6 px grid. Text styles are libadwaita's classes only (`dimmed`,
//! `caption`, `heading`, `title-*`, `numeric`, `property`); colours come from the theme.

use crate::format;
use appreestr_core::reasons::format_size;
use relm4::adw::{self, prelude::*};
use relm4::gtk;

/// Between lines of text, chips, an icon and its label.
pub const SPACE_XS: i32 = 6;
/// Inside a row, between buttons, content inset from the window edge.
pub const SPACE_S: i32 = 12;
/// Between blocks inside a dialog.
pub const SPACE_M: i32 = 18;
/// Between groups; vertical margins of pages and dialogs.
pub const SPACE_L: i32 = 24;
/// Around the header of the details page.
pub const SPACE_XL: i32 = 36;

/// Maximum width of the application list and the filter bar above it.
pub const LIST_WIDTH: i32 = 860;
pub const ICON_ROW: i32 = 32;
pub const ICON_HERO: i32 = 128;
pub const DIALOG_WIDTH: i32 = 460;

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(spacing).build()
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(spacing).build()
}

/// A start-aligned, ellipsized label with the given style classes.
pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// A boxed list without selection, for rows inside dialogs and groups.
pub fn boxed_list() -> gtk::ListBox {
    let l = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    l.add_css_class("boxed-list");
    l
}

/// "Title above, value below" row with a selectable value.
pub fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(format::escape(title))
        .subtitle(format::escape(value))
        .subtitle_selectable(true)
        .build();
    r.add_css_class("property");
    r
}

/// A row with a dimmed size at the end.
pub fn row_with_size(title: &str, subtitle: &str, size: Option<u64>) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(format::escape(title)).subtitle(format::escape(subtitle)).build();
    if let Some(s) = size {
        r.add_suffix(&label(&format_size(s), &["dimmed", "numeric"]));
    }
    r
}

/// A row with a warning icon in front.
pub fn warning_row(text: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(format::escape(text)).build();
    let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
    icon.add_css_class("warning");
    r.add_prefix(&icon);
    r
}

/// A flat icon-only button; the tooltip doubles as the accessible label.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).valign(gtk::Align::Center).build();
    b.add_css_class("flat");
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

/// An icon at the end of an activatable row, hinting at what activation does.
pub fn row_hint(icon: &str) -> gtk::Image {
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("dimmed");
    i
}

#[derive(Clone, Copy)]
pub enum Kind {
    Normal,
    Suggested,
    Destructive,
}

/// The main action of a screen: a centred pill button.
pub fn pill(text: &str, kind: Kind) -> gtk::Button {
    let b = gtk::Button::builder().label(text).halign(gtk::Align::Center).build();
    b.add_css_class("pill");
    match kind {
        Kind::Normal => {}
        Kind::Suggested => b.add_css_class("suggested-action"),
        Kind::Destructive => b.add_css_class("destructive-action"),
    }
    b
}
