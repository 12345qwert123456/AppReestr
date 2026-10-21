//! About and keyboard shortcuts dialogs.

use appreestr_core::APP_ID;
use appreestr_core::i18n::tr;
use relm4::adw::{self, prelude::*};

pub fn about(parent: &adw::ApplicationWindow) {
    let dialog = adw::AboutDialog::builder()
        .application_name("AppReestr")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .website(env!("CARGO_PKG_REPOSITORY"))
        .license_type(relm4::gtk::License::Gpl30)
        .comments(tr("All installed applications in one list: where they came from and how to remove them safely."))
        .build();
    dialog.present(Some(parent));
}

pub fn shortcuts(parent: &adw::ApplicationWindow) {
    let dialog = adw::ShortcutsDialog::new();
    let sections: [(String, Vec<(String, &str)>); 3] = [
        (
            tr("List"),
            vec![
                (tr("Search"), "<Ctrl>f"),
                (tr("Open details"), "Return"),
                (tr("Uninstall the selected application"), "Delete"),
                (tr("Back or clear the search"), "Escape"),
                (tr("Refresh the list"), "F5"),
            ],
        ),
        (
            tr("Filters"),
            vec![
                (tr("Show system components"), "<Ctrl>h"),
                (tr("Show command-line tools"), "<Ctrl>j"),
                (tr("Show add-ons"), "<Ctrl>k"),
                (tr("All sources"), "<Alt>1"),
                (tr("Filter by source"), "<Alt>2"),
            ],
        ),
        (
            tr("General"),
            vec![
                (tr("Keyboard shortcuts"), "<Ctrl>question"),
                (tr("Preferences"), "<Ctrl>comma"),
                (tr("Close"), "<Ctrl>q"),
            ],
        ),
    ];
    for (title, items) in sections {
        let section = adw::ShortcutsSection::new(Some(&title));
        for (label, accel) in items {
            section.add(adw::ShortcutsItem::new(&label, accel));
        }
        dialog.add(section);
    }
    dialog.present(Some(parent));
}
