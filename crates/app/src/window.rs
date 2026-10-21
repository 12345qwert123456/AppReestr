//! The main window: the list, search, filters, and navigation to details.

use crate::details::{self, DetailsPage};
use crate::row::AppRow;
use appreestr_core::cache;
use appreestr_core::config::{ConfigStore, Overrides, Settings, SortKey};
use appreestr_core::i18n::{tr, trf, trn};
use appreestr_core::listing::{self, ListQuery};
use appreestr_core::model::{App, FlatpakScope, SourceFilter, UserMark};
use appreestr_core::plan::{PlannedPkg, RemovalPlan};
use appreestr_core::reasons::{filter_label, format_size};
use appreestr_core::rules::Rules;
use appreestr_core::scan::{ScanInput, ScanResult, scan};
use appreestr_core::sources::pacman::OwnerIndex;
use appreestr_core::sysroot::SysRoot;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};
use relm4::prelude::*;
use relm4::typed_view::list::TypedListView;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Facts from the last full scan needed for protection and planning.
#[derive(Debug, Default)]
pub struct ScanData {
    pub owners: OwnerIndex,
    pub depends: HashMap<String, Vec<String>>,
    pub hold_pkgs: Vec<String>,
    pub kernels: Vec<String>,
}

pub struct AppModel {
    pub sys: SysRoot,
    pub rules: Rules,
    pub store: ConfigStore,
    pub settings: Settings,
    pub overrides: Overrides,
    pub apps: Vec<Arc<App>>,
    pub data: Arc<ScanData>,
    pub query: ListQuery,
    pub scanning: bool,
    pub scanned_once: bool,
    list: TypedListView<AppRow, gtk::SingleSelection>,
    visible: Vec<usize>,
    pub details: Option<DetailsPage>,
    pub progress: Option<crate::uninstall::Progress>,
    rescan_pending: bool,
    _watcher: Option<notify::RecommendedWatcher>,
}

#[derive(Debug)]
pub enum Msg {
    Search(String),
    Filter(Option<SourceFilter>),
    FilterIndex(usize),
    Sort(SortKey),
    ShowSystem(bool),
    ShowConsole(bool),
    Refresh,
    Activate(u32),
    Details(String),
    OpenLocation(String),
    OpenPath(String),
    Uninstall(String),
    UninstallUnit(String),
    UninstallSelected,
    UninstallUnits(String, Vec<String>),
    StartJob(Box<crate::uninstall::JobRequest>),
    Removed(String),
    CleanPackages(Vec<String>),
    CleanFlatpak(FlatpakScope),
    DetailsClosed,
    Mark(String, Option<UserMark>),
    UnitMark(String, Option<UserMark>),
    ExternalChange,
    Preferences,
    SetTheme(appreestr_core::config::Theme),
    SetAppImageDirs(Vec<String>),
    FocusSearch,
    FocusList,
    Back,
    About,
    Shortcuts,
}

pub enum Cmd {
    Scanned(Box<ScanResult>),
    Planned(Box<crate::uninstall::Planned>),
    DetailsPlan(String, Box<Result<RemovalPlan, String>>),
    Removal(crate::removal::Event),
    FollowUp { orphans: Vec<PlannedPkg>, flatpak: Option<FlatpakScope> },
    CleanPlanned(Box<(Result<RemovalPlan, String>, bool)>),
}

// Kept short: relm4 logs command outputs, and a full scan result is megabytes of text.
impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cmd::Scanned(r) => write!(f, "Scanned({} apps)", r.apps.len()),
            Cmd::Planned(p) => write!(f, "Planned({})", p.app.id),
            Cmd::DetailsPlan(id, _) => write!(f, "DetailsPlan({id})"),
            Cmd::Removal(e) => write!(f, "Removal({e:?})"),
            Cmd::FollowUp { orphans, .. } => write!(f, "FollowUp({} orphans)", orphans.len()),
            Cmd::CleanPlanned(_) => write!(f, "CleanPlanned"),
        }
    }
}

pub struct Widgets {
    search: gtk::SearchEntry,
    stack: gtk::Stack,
    chips: Vec<(Option<SourceFilter>, gtk::ToggleButton)>,
    summary: gtk::Label,
    nav: adw::NavigationView,
    pub toasts: adw::ToastOverlay,
    list_view: gtk::ListView,
    actions: gio::SimpleActionGroup,
    refresh_spinner: adw::Spinner,
}

impl AppModel {
    fn app(&self, id: &str) -> Option<Arc<App>> {
        self.apps.iter().find(|a| a.id == id).cloned()
    }

    pub fn app_by_unit(&self, key: &str) -> Option<Arc<App>> {
        self.apps.iter().find(|a| a.installs.iter().any(|i| i.key.as_str() == key)).cloned()
    }

    fn save_settings(&mut self) {
        self.settings.show_system = self.query.show_system;
        self.settings.show_console = self.query.show_console;
        self.settings.sort = self.query.sort;
        if let Err(e) = self.store.save_settings(&self.settings) {
            tracing::warn!("saving settings: {e}");
        }
    }

    fn start_scan(&mut self, sender: &ComponentSender<Self>, w: &Widgets) {
        if self.scanning {
            self.rescan_pending = true;
            return;
        }
        self.scanning = true;
        w.refresh_spinner.set_visible(true);
        let sys = self.sys.clone();
        let rules = self.rules.clone();
        let settings = self.settings.clone();
        let marks = self.overrides.as_map();
        sender.spawn_oneshot_command(move || {
            let res = scan(&ScanInput { sys: &sys, rules: &rules, settings: &settings, marks: &marks });
            if let Err(e) = cache::save(&cache::snapshot_path(&sys.cache_home), &res.apps) {
                tracing::warn!("saving snapshot: {e}");
            }
            Cmd::Scanned(Box::new(res))
        });
    }

    fn refresh_list(&mut self, w: &Widgets) {
        self.visible = listing::select(&self.apps, &self.query);
        self.list.clear();
        self.list.extend_from_iter(self.visible.iter().map(|&i| AppRow { app: self.apps[i].clone() }));

        let counts = listing::counts(&self.apps, self.query.show_system, self.query.show_console);
        let total: usize =
            self.apps.iter().filter(|a| a.is_listed(self.query.show_system, self.query.show_console)).count();
        for (f, chip) in &w.chips {
            let (label, n) = match f {
                None => (tr("All"), total),
                Some(f) => (filter_label(*f), counts.get(f).copied().unwrap_or(0)),
            };
            chip.set_label(&format!("{label}  {n}"));
            chip.set_visible(f.is_none() || n > 0 || self.query.filter == *f);
            if chip.is_active() != (self.query.filter == *f) {
                chip.set_active(self.query.filter == *f);
            }
        }

        let size: u64 = self.visible.iter().filter_map(|&i| self.apps[i].total_size()).sum();
        let n = self.visible.len() as u64;
        w.summary.set_label(&format!("{} · {}", trn("{} application", "{} applications", n), format_size(size)));

        let page = if self.apps.is_empty() && !self.scanned_once {
            "loading"
        } else if self.visible.is_empty() {
            "empty"
        } else {
            "list"
        };
        w.stack.set_visible_child_name(page);
    }

    fn sync_actions(&self, w: &Widgets) {
        let set = |name: &str, v: glib::Variant| {
            if let Some(a) = w.actions.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                a.set_state(&v);
            }
        };
        set("show-system", self.query.show_system.to_variant());
        set("show-console", self.query.show_console.to_variant());
        set("sort", sort_name(self.query.sort).to_variant());
    }

    fn show_details(&mut self, id: &str, w: &Widgets, sender: &ComponentSender<Self>) {
        let Some(app) = self.app(id) else { return };
        let page = details::build(&app);
        if w.nav.visible_page().and_then(|p| p.tag()).as_deref() == Some("details") {
            w.nav.pop();
        }
        w.nav.push(&page.page);
        self.details = Some(page);
        crate::uninstall::on_details_shown(self, sender);
    }

    fn open_location(&self, id: &str, root: &adw::ApplicationWindow) {
        let Some(app) = self.app(id) else { return };
        if let Some(loc) = app.installs.iter().flat_map(|i| i.locations.iter()).next() {
            open_path(loc.clone(), root);
        }
    }
}

pub fn open_path(path: PathBuf, root: &impl IsA<gtk::Window>) {
    let file = gio::File::for_path(&path);
    let launcher = gtk::FileLauncher::new(Some(&file));
    if path.is_dir() {
        launcher.launch(Some(root), None::<&gio::Cancellable>, |r| {
            if let Err(e) = r {
                tracing::warn!("open folder: {e}");
            }
        });
    } else {
        launcher.open_containing_folder(Some(root), None::<&gio::Cancellable>, |r| {
            if let Err(e) = r {
                tracing::warn!("open containing folder: {e}");
            }
        });
    }
}

fn sort_name(k: SortKey) -> &'static str {
    match k {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Date => "date",
        SortKey::Source => "source",
    }
}

fn sort_from(s: &str) -> SortKey {
    match s {
        "size" => SortKey::Size,
        "date" => SortKey::Date,
        "source" => SortKey::Source,
        _ => SortKey::Name,
    }
}

const FILTERS: [Option<SourceFilter>; 8] = [
    None,
    Some(SourceFilter::Official),
    Some(SourceFilter::ThirdPartyRepo),
    Some(SourceFilter::Aur),
    Some(SourceFilter::Flatpak),
    Some(SourceFilter::AppImage),
    Some(SourceFilter::Snap),
    Some(SourceFilter::Manual),
];

fn build_actions(sender: &ComponentSender<AppModel>, q: &ListQuery) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let string_action = |name: &str, f: fn(String) -> Msg| {
        let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let s = sender.clone();
        a.connect_activate(move |_, p| {
            if let Some(v) = p.and_then(|p| p.get::<String>()) {
                s.input(f(v));
            }
        });
        group.add_action(&a);
    };
    string_action("details", Msg::Details);
    string_action("open-location", Msg::OpenLocation);
    string_action("open-path", Msg::OpenPath);
    string_action("uninstall", Msg::Uninstall);
    string_action("uninstall-unit", Msg::UninstallUnit);
    string_action("mark-app", |id| Msg::Mark(id, Some(UserMark::App)));
    string_action("mark-hidden", |id| Msg::Mark(id, Some(UserMark::Hidden)));
    string_action("unmark", |id| Msg::Mark(id, None));

    let toggle = |name: &str, init: bool, f: fn(bool) -> Msg| {
        let a = gio::SimpleAction::new_stateful(name, None, &init.to_variant());
        let s = sender.clone();
        a.connect_activate(move |a, _| {
            let v = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            a.set_state(&v.to_variant());
            s.input(f(v));
        });
        group.add_action(&a);
    };
    toggle("show-system", q.show_system, Msg::ShowSystem);
    toggle("show-console", q.show_console, Msg::ShowConsole);

    let sort = gio::SimpleAction::new_stateful("sort", Some(glib::VariantTy::STRING), &sort_name(q.sort).to_variant());
    let s = sender.clone();
    sort.connect_activate(move |a, p| {
        if let Some(v) = p.and_then(|p| p.get::<String>()) {
            a.set_state(&v.to_variant());
            s.input(Msg::Sort(sort_from(&v)));
        }
    });
    group.add_action(&sort);

    let plain = |name: &str, msg: fn() -> Msg| {
        let a = gio::SimpleAction::new(name, None);
        let s = sender.clone();
        a.connect_activate(move |_, _| s.input(msg()));
        group.add_action(&a);
    };
    plain("refresh", || Msg::Refresh);
    plain("search", || Msg::FocusSearch);
    plain("about", || Msg::About);
    plain("preferences", || Msg::Preferences);
    plain("shortcuts", || Msg::Shortcuts);
    plain("back", || Msg::Back);
    plain("uninstall-selected", || Msg::UninstallSelected);
    for i in 0..FILTERS.len() {
        let a = gio::SimpleAction::new(&format!("filter-{}", i + 1), None);
        let s = sender.clone();
        a.connect_activate(move |_, _| s.input(Msg::FilterIndex(i)));
        group.add_action(&a);
    }
    group
}

fn set_accels() {
    let app = relm4::main_application();
    let accels: &[(&str, &[&str])] = &[
        ("win.search", &["<Ctrl>f"]),
        ("win.refresh", &["F5", "<Ctrl>r"]),
        ("win.show-system", &["<Ctrl>h"]),
        ("win.show-console", &["<Ctrl>j"]),
        ("win.shortcuts", &["<Ctrl>question"]),
        ("win.preferences", &["<Ctrl>comma"]),
        ("win.back", &["<Alt>Left"]),
        ("window.close", &["<Ctrl>w", "<Ctrl>q"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
    for i in 1..=FILTERS.len() {
        app.set_accels_for_action(&format!("win.filter-{i}"), &[&format!("<Alt>{i}")]);
    }
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let view = gio::Menu::new();
    view.append(Some(&tr("Show System Components")), Some("win.show-system"));
    view.append(Some(&tr("Show Command-Line Tools")), Some("win.show-console"));
    menu.append_section(None, &view);
    let other = gio::Menu::new();
    other.append(Some(&tr("Refresh")), Some("win.refresh"));
    other.append(Some(&tr("Preferences")), Some("win.preferences"));
    other.append(Some(&tr("Keyboard Shortcuts")), Some("win.shortcuts"));
    other.append(Some(&tr("About AppReestr")), Some("win.about"));
    menu.append_section(None, &other);
    menu
}

fn sort_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for (label, key) in
        [(tr("Name"), "name"), (tr("Size"), "size"), (tr("Installation Date"), "date"), (tr("Source"), "source")]
    {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("win.sort"), Some(&key.to_variant()));
        menu.append_item(&item);
    }
    menu
}

impl Component for AppModel {
    type CommandOutput = Cmd;
    type Input = Msg;
    type Output = ();
    type Init = ();
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title("AppReestr")
            .default_width(920)
            .default_height(720)
            .width_request(360)
            .height_request(400)
            .build()
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let sys = SysRoot::host();
        crate::icons::setup(&sys);
        let store = ConfigStore::new(&sys);
        let rules = Rules::load(&store.rules_file());
        let settings = store.settings();
        let overrides = store.overrides();
        let query = ListQuery {
            show_system: settings.show_system,
            show_console: settings.show_console,
            sort: settings.sort,
            ..Default::default()
        };
        crate::theme::apply(settings.theme);

        // Instant start: show the last snapshot while the fresh scan runs.
        let apps: Vec<Arc<App>> =
            cache::load(&cache::snapshot_path(&sys.cache_home)).unwrap_or_default().into_iter().map(Arc::new).collect();

        let list: TypedListView<AppRow, gtk::SingleSelection> = TypedListView::new();
        let list_view = list.view.clone();
        list_view.set_single_click_activate(true);
        list_view.add_css_class("navigation-sidebar");
        list_view.update_property(&[gtk::accessible::Property::Label(&tr("Installed applications"))]);
        let s = sender.clone();
        list_view.connect_activate(move |_, pos| s.input(Msg::Activate(pos)));
        let key = gtk::EventControllerKey::new();
        let s = sender.clone();
        key.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Delete || k == gtk::gdk::Key::KP_Delete {
                s.input(Msg::UninstallSelected);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        list_view.add_controller(key);

        // Header bar: search in the middle, sort and main menu on the right.
        let search = gtk::SearchEntry::builder().placeholder_text(tr("Search applications")).hexpand(true).build();
        search.set_key_capture_widget(Some(&root));
        let s = sender.clone();
        search.connect_search_changed(move |e| s.input(Msg::Search(e.text().to_string())));
        let s = sender.clone();
        search.connect_activate(move |_| s.input(Msg::FocusList));
        let skey = gtk::EventControllerKey::new();
        let s = sender.clone();
        skey.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Down {
                s.input(Msg::FocusList);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        search.add_controller(skey);
        let search_clamp = adw::Clamp::builder().maximum_size(480).child(&search).build();

        let header = adw::HeaderBar::builder().title_widget(&search_clamp).build();
        let refresh_spinner = adw::Spinner::builder().visible(false).tooltip_text(tr("Updating the list…")).build();
        header.pack_start(&refresh_spinner);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu())
            .tooltip_text(tr("Main Menu"))
            .primary(true)
            .build();
        let sort_button = gtk::MenuButton::builder()
            .icon_name("view-sort-descending-symbolic")
            .menu_model(&sort_menu())
            .tooltip_text(tr("Sort"))
            .build();
        header.pack_end(&menu_button);
        header.pack_end(&sort_button);

        // Source filter chips.
        let chips_box = gtk::Box::builder().spacing(6).margin_start(12).margin_end(12).margin_bottom(6).build();
        let mut chips = Vec::new();
        let mut first: Option<gtk::ToggleButton> = None;
        for f in FILTERS {
            let chip = gtk::ToggleButton::builder().active(f.is_none()).build();
            chip.add_css_class("pill");
            chip.add_css_class("chip");
            if let Some(fst) = &first {
                chip.set_group(Some(fst));
            } else {
                first = Some(chip.clone());
            }
            let s = sender.clone();
            chip.connect_toggled(move |c| {
                if c.is_active() {
                    s.input(Msg::Filter(f));
                }
            });
            chips_box.append(&chip);
            chips.push((f, chip));
        }
        let chips_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .child(&chips_box)
            .build();
        let summary = gtk::Label::builder().xalign(0.0).margin_start(18).margin_bottom(6).build();
        summary.add_css_class("dim-label");
        summary.add_css_class("caption");
        let subheader = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        subheader.append(&chips_scroll);
        subheader.append(&summary);

        // Content: loading / empty / list.
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        let loading = adw::StatusPage::builder()
            .title(tr("Looking for installed applications…"))
            .paintable(&adw::SpinnerPaintable::new(None::<&gtk::Widget>))
            .build();
        stack.add_named(&loading, Some("loading"));
        let empty = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title(tr("Nothing Found"))
            .description(tr(
                "Try a different search, or show system components and command-line tools from the main menu.",
            ))
            .build();
        stack.add_named(&empty, Some("empty"));
        let clamp = adw::ClampScrollable::builder().maximum_size(900).child(&list_view).build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        stack.add_named(&scrolled, Some("list"));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&subheader);
        toolbar.set_content(Some(&stack));
        let list_page =
            adw::NavigationPage::builder().title(tr("Installed Applications")).tag("list").child(&toolbar).build();
        let nav = adw::NavigationView::new();
        nav.add(&list_page);
        let s = sender.clone();
        nav.connect_popped(move |_, page| {
            if page.tag().as_deref() == Some("details") {
                s.input(Msg::DetailsClosed);
            }
        });
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));
        root.set_content(Some(&toasts));

        let actions = build_actions(&sender, &query);
        root.insert_action_group("win", Some(&actions));
        set_accels();
        let esc = gtk::ShortcutController::new();
        esc.set_scope(gtk::ShortcutScope::Managed);
        esc.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("Escape"),
            Some(gtk::NamedAction::new("win.back")),
        ));
        root.add_controller(esc);

        let mut model = AppModel {
            sys,
            rules,
            store,
            settings,
            overrides,
            apps,
            data: Arc::new(ScanData::default()),
            query,
            scanning: false,
            scanned_once: false,
            list,
            visible: Vec::new(),
            details: None,
            progress: None,
            rescan_pending: false,
            _watcher: None,
        };
        model._watcher = crate::watcher::watch(&model.sys, &model.settings.appimage_dirs, sender.clone());
        let widgets = Widgets { search, stack, chips, summary, nav, toasts, list_view, actions, refresh_spinner };
        model.refresh_list(&widgets);
        model.start_scan(&sender, &widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(&mut self, w: &mut Widgets, msg: Msg, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            Msg::Search(text) => {
                self.query.text = text;
                self.refresh_list(w);
            }
            Msg::Filter(f) => {
                if self.query.filter != f {
                    self.query.filter = f;
                    self.refresh_list(w);
                }
            }
            Msg::FilterIndex(i) => {
                if let Some((f, chip)) = w.chips.get(i)
                    && chip.is_visible()
                {
                    self.query.filter = *f;
                    self.refresh_list(w);
                }
            }
            Msg::Sort(k) => {
                self.query.sort = k;
                self.save_settings();
                self.refresh_list(w);
            }
            Msg::ShowSystem(v) => {
                self.query.show_system = v;
                self.save_settings();
                self.refresh_list(w);
            }
            Msg::ShowConsole(v) => {
                self.query.show_console = v;
                self.save_settings();
                self.refresh_list(w);
            }
            Msg::Refresh => self.start_scan(&sender, w),
            Msg::Activate(pos) => {
                if let Some(&i) = self.visible.get(pos as usize) {
                    let id = self.apps[i].id.clone();
                    self.show_details(&id, w, &sender);
                }
            }
            Msg::Details(id) => self.show_details(&id, w, &sender),
            Msg::OpenLocation(id) => self.open_location(&id, root),
            Msg::OpenPath(p) => open_path(PathBuf::from(p), root),
            Msg::Uninstall(id) => crate::uninstall::request(self, &id, None, root, &sender),
            Msg::UninstallUnit(key) => {
                if let Some(app) = self.app_by_unit(&key) {
                    crate::uninstall::request(self, &app.id.clone(), Some(key), root, &sender);
                }
            }
            Msg::UninstallSelected => {
                let pos = self.list.selection_model.selected();
                if let Some(&i) = self.visible.get(pos as usize) {
                    let id = self.apps[i].id.clone();
                    crate::uninstall::request(self, &id, None, root, &sender);
                }
            }
            Msg::UninstallUnits(id, keys) => crate::uninstall::plan_units(self, &id, &keys, &sender),
            Msg::StartJob(req) => crate::uninstall::start(self, *req, root, &sender),
            Msg::Removed(id) => {
                if self.details.as_ref().is_some_and(|d| d.app_id == id) && w.nav.navigation_stack().n_items() > 1 {
                    w.nav.pop();
                    self.details = None;
                }
                self.start_scan(&sender, w);
            }
            Msg::CleanPackages(names) => crate::uninstall::clean_packages(self, names, &sender),
            Msg::CleanFlatpak(scope) => {
                let need_agent = scope == FlatpakScope::System;
                crate::uninstall::start(
                    self,
                    crate::uninstall::JobRequest {
                        job: crate::removal::Job {
                            title: tr("Unused Flatpak runtimes"),
                            steps: vec![crate::removal::Step::FlatpakUnused { system: scope == FlatpakScope::System }],
                            need_agent,
                        },
                        app_id: String::new(),
                        had_pacman: false,
                        had_flatpak: None,
                        orphans_before: Vec::new(),
                    },
                    root,
                    &sender,
                );
            }
            Msg::DetailsClosed => self.details = None,
            Msg::Mark(id, mark) => {
                let Some(app) = self.app(&id) else { return };
                // Marks are stored per installation, so they survive merges and renames.
                let key = app.primary().key.to_string();
                let previous = app.installs.iter().find_map(|i| self.overrides.marks.get(i.key.as_str()).copied());
                for i in &app.installs {
                    self.overrides.marks.remove(i.key.as_str());
                }
                if let Some(m) = mark {
                    self.overrides.marks.insert(key.clone(), m);
                }
                if let Err(e) = self.store.save_overrides(&self.overrides) {
                    tracing::warn!("saving overrides: {e}");
                }
                let text = match mark {
                    Some(UserMark::Hidden) => trf("“{}” is hidden. It stays under system components.", &[&app.name]),
                    Some(UserMark::App) => trf("“{}” is now shown as an application", &[&app.name]),
                    None => trf("“{}” is classified automatically again", &[&app.name]),
                };
                let toast = adw::Toast::builder().title(text).button_label(tr("Undo")).build();
                let s = sender.clone();
                toast.connect_button_clicked(move |_| s.input(Msg::UnitMark(key.clone(), previous)));
                w.toasts.add_toast(toast);
                self.start_scan(&sender, w);
            }
            Msg::UnitMark(key, mark) => {
                match mark {
                    Some(m) => self.overrides.marks.insert(key, m),
                    None => self.overrides.marks.remove(&key),
                };
                if let Err(e) = self.store.save_overrides(&self.overrides) {
                    tracing::warn!("saving overrides: {e}");
                }
                self.start_scan(&sender, w);
            }
            Msg::Preferences => crate::prefs::show(&self.settings, root, &sender),
            Msg::SetTheme(t) => {
                self.settings.theme = t;
                crate::theme::apply(t);
                self.save_settings();
            }
            Msg::SetAppImageDirs(dirs) => {
                self.settings.appimage_dirs = dirs;
                self.save_settings();
                self._watcher = crate::watcher::watch(&self.sys, &self.settings.appimage_dirs, sender.clone());
                self.start_scan(&sender, w);
            }
            Msg::ExternalChange => {
                if self.progress.is_none() {
                    self.start_scan(&sender, w);
                }
            }
            Msg::FocusSearch => {
                w.search.grab_focus();
            }
            Msg::FocusList => {
                if !self.visible.is_empty() {
                    w.list_view.scroll_to(0, gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT, None);
                }
            }
            Msg::Back => {
                if w.nav.navigation_stack().n_items() > 1 {
                    w.nav.pop();
                } else if !w.search.text().is_empty() {
                    w.search.set_text("");
                }
            }
            Msg::About => crate::dialogs::about(root),
            Msg::Shortcuts => crate::dialogs::shortcuts(root),
        }
        self.sync_actions(w);
    }

    fn update_cmd_with_view(&mut self, w: &mut Widgets, cmd: Cmd, sender: ComponentSender<Self>, root: &Self::Root) {
        match cmd {
            Cmd::Scanned(res) => {
                self.scanning = false;
                #[cfg(debug_assertions)]
                if !self.scanned_once {
                    crate::devtools::on_first_scan(root, &sender);
                }
                self.scanned_once = true;
                w.refresh_spinner.set_visible(false);
                for warn in &res.warnings {
                    tracing::warn!("{warn}");
                }
                if let Some((_, t)) = res.timings.iter().find(|(n, _)| *n == "total") {
                    tracing::info!("scan finished in {:.0} ms", t.as_secs_f64() * 1000.0);
                }
                let res = *res;
                self.apps = res.apps.into_iter().map(Arc::new).collect();
                self.data = Arc::new(ScanData {
                    owners: res.owners,
                    depends: res.depends,
                    hold_pkgs: res.hold_pkgs,
                    kernels: res.kernels,
                });
                self.refresh_list(w);
                // Re-open the details of the shown app with fresh data, or leave if it is gone.
                if let Some(d) = self.details.take() {
                    if self.app(&d.app_id).is_some() {
                        self.show_details(&d.app_id, w, &sender);
                    } else if w.nav.navigation_stack().n_items() > 1 {
                        w.nav.pop();
                    }
                }
                if !res.warnings.is_empty() {
                    w.toasts.add_toast(adw::Toast::new(&trf(
                        "Some sources could not be read: {}",
                        &[&res.warnings.join("; ")],
                    )));
                }
                if std::mem::take(&mut self.rescan_pending) {
                    self.start_scan(&sender, w);
                }
            }
            Cmd::Planned(p) => crate::uninstall::confirm(*p, root, &sender),
            Cmd::DetailsPlan(id, plan) => crate::uninstall::fill_details(self, &id, *plan),
            Cmd::Removal(e) => crate::uninstall::on_event(self, e, root, &sender, &w.toasts),
            Cmd::FollowUp { orphans, flatpak } => crate::uninstall::offer_cleanup(orphans, flatpak, &w.toasts, &sender),
            Cmd::CleanPlanned(b) => {
                let (plan, need_agent) = *b;
                crate::uninstall::confirm_clean(plan, need_agent, root, &sender);
            }
        }
    }
}
