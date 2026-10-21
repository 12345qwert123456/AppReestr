//! Preferences: appearance and where to look for AppImage files.

use crate::window::{AppModel, Msg};
use crate::{format, ui};
use appreestr_core::config::{MAX_TRANSPARENCY, Settings, Theme};
use appreestr_core::i18n::{self, tr};
use relm4::ComponentSender;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

pub fn show(settings: &Settings, root: &adw::ApplicationWindow, sender: &ComponentSender<AppModel>) {
    let dialog = adw::PreferencesDialog::new();
    let page = adw::PreferencesPage::new();

    let look = adw::PreferencesGroup::builder().title(tr("Appearance")).build();
    let themes = gtk::StringList::new(&[&tr("Follow the system"), &tr("Light"), &tr("Dark")]);
    let theme = adw::ComboRow::builder().title(tr("Style")).model(&themes).build();
    theme.set_selected(match settings.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    let s = sender.clone();
    theme.connect_selected_notify(move |r| {
        let t = match r.selected() {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        };
        s.input(Msg::SetTheme(t));
    });
    look.add(&theme);
    let transparency = adw::SpinRow::builder()
        .title(tr("Window Transparency"))
        .subtitle(tr("Percent. Needs a compositor that supports transparency."))
        .adjustment(&gtk::Adjustment::new(
            f64::from(settings.transparency),
            0.0,
            f64::from(MAX_TRANSPARENCY),
            5.0,
            10.0,
            0.0,
        ))
        .build();
    let s = sender.clone();
    transparency.connect_value_notify(move |r| s.input(Msg::SetTransparency(r.value() as u8)));
    look.add(&transparency);
    page.add(&look);

    let languages = i18n::available(&crate::localedir());
    let names: Vec<&str> = languages.iter().map(|l| l.name.as_str()).collect();
    let language = adw::ComboRow::builder().title(tr("Language")).model(&gtk::StringList::new(&names)).build();
    language.set_selected(languages.iter().position(|l| l.code == settings.language).unwrap_or(0) as u32);
    let (s, d) = (sender.clone(), dialog.clone());
    let pending: RefCell<Option<adw::Toast>> = RefCell::new(None);
    language.connect_selected_notify(move |r| {
        let Some(chosen) = languages.get(r.selected() as usize) else { return };
        s.input(Msg::SetLanguage(chosen.code.clone()));
        if let Some(old) = pending.take() {
            old.dismiss();
        }
        // Going back to the language in use needs no restart.
        if chosen.code != i18n::active() {
            let toast = adw::Toast::builder()
                .title(tr("Restart to apply the language."))
                .button_label(tr("Restart"))
                .timeout(0)
                .build();
            toast.connect_button_clicked(|_| crate::restart());
            d.add_toast(toast.clone());
            pending.replace(Some(toast));
        }
    });
    let lang_group = adw::PreferencesGroup::new();
    lang_group.add(&language);
    page.add(&lang_group);

    let folders = adw::PreferencesGroup::builder()
        .title(tr("AppImage Folders"))
        .description(tr(
            "AppImage files are looked for in these folders. Files started from the menu are found anywhere.",
        ))
        .build();
    let dirs = Rc::new(RefCell::new(settings.appimage_dirs.clone()));
    let list = ui::boxed_list();
    folders.add(&list);
    let add = adw::ButtonRow::builder().title(tr("Add Folder…")).start_icon_name("list-add-symbolic").build();
    page.add(&folders);

    fn refill(
        list: &gtk::ListBox,
        add: &adw::ButtonRow,
        dirs: &Rc<RefCell<Vec<String>>>,
        sender: &ComponentSender<AppModel>,
    ) {
        list.remove_all();
        for d in dirs.borrow().iter() {
            let shown = if d.starts_with('/') { format::path(Path::new(d)) } else { format!("~/{d}") };
            let row = adw::ActionRow::builder().title(format::escape(&shown)).build();
            let rm = ui::icon_button("list-remove-symbolic", &tr("Stop looking here"));
            let (l, a, ds, s, d) = (list.clone(), add.clone(), dirs.clone(), sender.clone(), d.clone());
            rm.connect_clicked(move |_| {
                ds.borrow_mut().retain(|x| x != &d);
                s.input(Msg::SetAppImageDirs(ds.borrow().clone()));
                refill(&l, &a, &ds, &s);
            });
            row.add_suffix(&rm);
            list.append(&row);
        }
        list.append(add);
    }
    refill(&list, &add, &dirs, sender);

    let (l, a, ds, s, r) = (list.clone(), add.clone(), dirs.clone(), sender.clone(), root.clone());
    add.connect_activated(move |_| {
        let fd = gtk::FileDialog::builder().title(tr("Choose a Folder")).modal(true).build();
        let (l, a, ds, s) = (l.clone(), a.clone(), ds.clone(), s.clone());
        fd.select_folder(Some(&r), None::<&gio::Cancellable>, move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else { return };
            let home = glib::home_dir();
            let value = match path.strip_prefix(&home) {
                Ok(rel) => rel.to_string_lossy().into_owned(),
                Err(_) => path.to_string_lossy().into_owned(),
            };
            if !ds.borrow().contains(&value) {
                ds.borrow_mut().push(value);
                s.input(Msg::SetAppImageDirs(ds.borrow().clone()));
                refill(&l, &a, &ds, &s);
            }
        });
    });

    // For when an ordinary uninstall is refused.
    let danger = adw::PreferencesGroup::builder()
        .title(tr("Danger Zone"))
        .description(tr("For when an ordinary uninstall is refused."))
        .build();
    let force = adw::ActionRow::builder()
        .title(tr("Force Remove a Package…"))
        .subtitle(tr("Ignores dependencies and protection. The system may stop working."))
        .activatable(true)
        .build();
    force.add_suffix(&ui::row_hint("go-next-symbolic"));
    let (s, d) = (sender.clone(), dialog.clone());
    force.connect_activated(move |_| {
        d.close();
        s.input(Msg::DangerZone);
    });
    danger.add(&force);
    page.add(&danger);

    dialog.add(&page);
    dialog.present(Some(root));
}
