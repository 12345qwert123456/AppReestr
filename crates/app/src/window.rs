//! The main window: the list, search, filters, and navigation to details.

use crate::details::{self, DetailsPage};
use crate::row::{AppRow, Selection};
use crate::ui::{self, LIST_WIDTH, SPACE_S, SPACE_XS};
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
    /// Applications ticked in selection mode.
    select: Selection,
    /// A privileged copy is being started; wait for it.
    elevating: bool,
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
    ShowAddons(bool),
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
    Removed(Vec<String>),
    /// Enter or leave selection mode.
    SelectMode(bool),
    /// Tick everything that is listed, or untick it when it is all ticked already.
    SelectAll,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    SelectApps(Vec<String>),
    SelectionChanged,
    UninstallMany,
    AskElevate,
    Elevate,
    /// The Danger Zone: choose any installed package to remove by force.
    DangerZone,
    ForcePlan(String),
    CleanPackages(Vec<String>),
    CleanFlatpak(FlatpakScope),
    DetailsClosed,
    Mark(String, Option<UserMark>),
    UnitMark(String, Option<UserMark>),
    ExternalChange,
    Preferences,
    SetTheme(appreestr_core::config::Theme),
    SetTransparency(u8),
    SetAppImageDirs(Vec<String>),
    FocusSearch,
    ClearSearch,
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
    Elevate(crate::removal::Event),
    DangerListing(Box<crate::danger::Listing>),
    ForcePlanned(Box<crate::danger::Planned>),
}

// Kept short: relm4 logs command outputs, and a full scan result is megabytes of text.
impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cmd::Scanned(r) => write!(f, "Scanned({} apps)", r.apps.len()),
            Cmd::Planned(p) => write!(f, "Planned({} apps)", p.items.len()),
            Cmd::DetailsPlan(id, _) => write!(f, "DetailsPlan({id})"),
            Cmd::Removal(e) => write!(f, "Removal({e:?})"),
            Cmd::FollowUp { orphans, .. } => write!(f, "FollowUp({} orphans)", orphans.len()),
            Cmd::CleanPlanned(_) => write!(f, "CleanPlanned"),
            Cmd::Elevate(e) => write!(f, "Elevate({e:?})"),
            Cmd::DangerListing(l) => write!(f, "DangerListing({} packages)", l.packages.len()),
            Cmd::ForcePlanned(p) => write!(f, "ForcePlanned({})", p.name),
        }
    }
}

pub struct Widgets {
    search: gtk::SearchEntry,
    stack: gtk::Stack,
    chips: Vec<(Option<SourceFilter>, gtk::ToggleButton)>,
    summary: gtk::Label,
    toolbar: adw::ToolbarView,
    select_toggle: gtk::ToggleButton,
    select_all: gtk::Button,
    uninstall_selected: gtk::Button,
    admin_button: Option<gtk::Button>,
    empty: adw::StatusPage,
    empty_button: gtk::Button,
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
        self.settings.show_addons = self.query.show_addons;
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
        let select = self.select.clone();
        self.list.extend_from_iter(
            self.visible.iter().map(|&i| AppRow { app: self.apps[i].clone(), select: select.clone() }),
        );

        let counts = listing::counts(&self.apps, self.query.shown());
        let total: usize = self.apps.iter().filter(|a| a.is_listed(self.query.shown())).count();
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

        self.update_summary(w);

        let page = if self.apps.is_empty() && !self.scanned_once {
            "loading"
        } else if self.visible.is_empty() {
            "empty"
        } else {
            "list"
        };
        if page == "empty" {
            self.update_empty(w);
        }
        w.stack.set_visible_child_name(page);
    }

    /// Under the filters: what is listed, or in selection mode what is ticked.
    fn update_summary(&self, w: &Widgets) {
        if self.select.active() {
            let ids = self.select.ids();
            let size: u64 = self.apps.iter().filter(|a| ids.contains(&a.id)).filter_map(|a| a.total_size()).sum();
            let text = if ids.is_empty() {
                tr("Select applications to uninstall")
            } else {
                format!("{} · {}", trn("{} selected", "{} selected", ids.len() as u64), format_size(size))
            };
            w.summary.set_label(&text);
            w.uninstall_selected.set_sensitive(!ids.is_empty());
            let all = self.all_listed_selected();
            w.select_all.set_label(&if all { tr("Select None") } else { tr("Select All") });
            return;
        }
        let size: u64 = self.visible.iter().filter_map(|&i| self.apps[i].total_size()).sum();
        let n = self.visible.len() as u64;
        w.summary.set_label(&format!("{} · {}", trn("{} application", "{} applications", n), format_size(size)));
    }

    /// Every removable application that is listed now is ticked.
    fn all_listed_selected(&self) -> bool {
        let mut any = false;
        for &i in &self.visible {
            if self.apps[i].is_removable() {
                any = true;
                if !self.select.contains(&self.apps[i].id) {
                    return false;
                }
            }
        }
        any
    }

    /// The "nothing to show" page explains why and offers the way out.
    fn update_empty(&self, w: &Widgets) {
        let (title, description, button) = if !self.query.text.is_empty() {
            (
                trf("No Results for “{}”", &[&self.query.text]),
                tr("Check the spelling or try a different search."),
                Some((tr("Clear Search"), "win.clear-search")),
            )
        } else if !self.query.show_system {
            (
                tr("Nothing Found"),
                tr("Only applications are shown. System components are hidden."),
                Some((tr("Show System Components"), "win.show-system")),
            )
        } else if !self.query.show_console {
            (
                tr("Nothing Found"),
                tr("Only applications are shown. Command-line tools are hidden."),
                Some((tr("Show Command-Line Tools"), "win.show-console")),
            )
        } else if !self.query.show_addons {
            (
                tr("Nothing Found"),
                tr("Only applications are shown. Add-ons are hidden."),
                Some((tr("Show Add-ons"), "win.show-addons")),
            )
        } else {
            (tr("Nothing Found"), tr("No installed applications were found."), None)
        };
        w.empty.set_title(&title);
        w.empty.set_description(Some(&description));
        w.empty_button.set_visible(button.is_some());
        if let Some((label, action)) = button {
            w.empty_button.set_label(&label);
            w.empty_button.set_action_name(Some(action));
        }
    }

    fn sync_actions(&self, w: &Widgets) {
        let set = |name: &str, v: glib::Variant| {
            if let Some(a) = w.actions.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                a.set_state(&v);
            }
        };
        set("show-system", self.query.show_system.to_variant());
        set("show-console", self.query.show_console.to_variant());
        set("show-addons", self.query.show_addons.to_variant());
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
    string_action("force-remove", Msg::ForcePlan);
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
    toggle("show-addons", q.show_addons, Msg::ShowAddons);

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
    plain("clear-search", || Msg::ClearSearch);
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
        ("win.show-addons", &["<Ctrl>k"]),
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
    view.append(Some(&tr("Show Add-ons")), Some("win.show-addons"));
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
    /// Whether to tell the process that started us when the window is up.
    type Init = bool;
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

    fn init(notify_ready: bool, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let is_root = crate::elevate::is_root();
        if notify_ready {
            let sent = std::cell::Cell::new(false);
            root.connect_map(move |_| {
                if !sent.replace(true) {
                    crate::elevate::notify_ready();
                }
            });
        }
        let sys = SysRoot::host();
        crate::icons::setup(&sys);
        let store = ConfigStore::new(&sys);
        let rules = Rules::load(&store.rules_file());
        let settings = store.settings();
        let overrides = store.overrides();
        let query = ListQuery {
            show_system: settings.show_system,
            show_console: settings.show_console,
            show_addons: settings.show_addons,
            sort: settings.sort,
            ..Default::default()
        };
        crate::theme::apply(&settings);

        // Instant start: show the last snapshot while the fresh scan runs.
        let apps: Vec<Arc<App>> =
            cache::load(&cache::snapshot_path(&sys.cache_home)).unwrap_or_default().into_iter().map(Arc::new).collect();

        let list: TypedListView<AppRow, gtk::SingleSelection> = TypedListView::new();
        let list_view = list.view.clone();
        list_view.set_single_click_activate(true);
        list_view.add_css_class("navigation-sidebar");
        list_view.set_margin_top(SPACE_XS);
        list_view.set_margin_bottom(SPACE_S);
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
        let header = adw::HeaderBar::builder().title_widget(&adw::WindowTitle::new("AppReestr", "")).build();
        let refresh_spinner = adw::Spinner::builder().visible(false).tooltip_text(tr("Updating the list…")).build();
        let select_toggle = gtk::ToggleButton::builder()
            .icon_name("selection-mode-symbolic")
            .tooltip_text(tr("Select Applications"))
            .build();
        select_toggle.update_property(&[gtk::accessible::Property::Label(&tr("Select Applications"))]);
        let s = sender.clone();
        select_toggle.connect_toggled(move |b| s.input(Msg::SelectMode(b.is_active())));
        header.pack_start(&select_toggle);
        // Without administrator rights every removal asks for a password; this restarts with them.
        let admin_button = (!is_root).then(|| {
            let b = ui::icon_button("changes-prevent-symbolic", &tr("Restart as Administrator…"));
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(Msg::AskElevate));
            header.pack_start(&b);
            b
        });
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
        let chips_box = ui::hbox(SPACE_XS);
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
        let summary = ui::label("", &["dimmed", "caption"]);
        summary.set_margin_start(SPACE_S);
        summary.set_margin_bottom(SPACE_XS);
        // Search, filters and the summary share the list's clamp and the same inset from its edges.
        let filters = ui::vbox(SPACE_S);
        filters.set_margin_start(SPACE_XS);
        filters.set_margin_end(SPACE_XS);
        filters.set_margin_top(SPACE_S);
        filters.append(&search);
        filters.append(&chips_scroll);
        filters.append(&summary);
        // The same clamp as the list below, so the left edges line up.
        let subheader = adw::Clamp::builder().maximum_size(LIST_WIDTH).child(&filters).build();

        // Content: loading / empty / list.
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        let loading = adw::StatusPage::builder()
            .title(tr("Looking for installed applications…"))
            .paintable(&adw::SpinnerPaintable::new(None::<&gtk::Widget>))
            .build();
        stack.add_named(&loading, Some("loading"));
        let empty_button = ui::pill("", ui::Kind::Suggested);
        let empty = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title(tr("Nothing Found"))
            .child(&empty_button)
            .build();
        stack.add_named(&empty, Some("empty"));
        let clamp = adw::ClampScrollable::builder().maximum_size(LIST_WIDTH).child(&list_view).build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        stack.add_named(&scrolled, Some("list"));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_css_class("translucent");
        root.add_css_class("translucent");
        toolbar.add_top_bar(&header);
        let admin_banner = adw::Banner::builder()
            .title(tr("Administrator mode: removing does not ask for a password."))
            .revealed(is_root)
            .build();
        toolbar.add_top_bar(&admin_banner);
        toolbar.add_top_bar(&subheader);
        toolbar.set_content(Some(&stack));
        // Selection mode: the action that applies to everything ticked.
        let select_all = gtk::Button::builder().label(tr("Select All")).build();
        select_all.add_css_class("flat");
        let s = sender.clone();
        select_all.connect_clicked(move |_| s.input(Msg::SelectAll));
        let uninstall_selected = ui::pill(&tr("Uninstall…"), ui::Kind::Destructive);
        uninstall_selected.set_sensitive(false);
        let s = sender.clone();
        uninstall_selected.connect_clicked(move |_| s.input(Msg::UninstallMany));
        let select_bar = gtk::ActionBar::new();
        select_bar.pack_start(&select_all);
        select_bar.pack_end(&uninstall_selected);
        toolbar.add_bottom_bar(&select_bar);
        toolbar.set_reveal_bottom_bars(false);
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
            select: Selection::new(),
            elevating: false,
            rescan_pending: false,
            _watcher: None,
        };
        model._watcher = crate::watcher::watch(&model.sys, &model.settings.appimage_dirs, sender.clone());
        let s = sender.clone();
        model.select.on_change(move || s.input(Msg::SelectionChanged));
        let widgets = Widgets {
            search,
            stack,
            chips,
            summary,
            toolbar,
            select_toggle,
            select_all,
            uninstall_selected,
            admin_button,
            empty,
            empty_button,
            nav,
            toasts,
            list_view,
            actions,
            refresh_spinner,
        };
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
            Msg::ShowAddons(v) => {
                self.query.show_addons = v;
                self.save_settings();
                self.refresh_list(w);
            }
            Msg::Refresh => self.start_scan(&sender, w),
            Msg::Activate(pos) => {
                if let Some(&i) = self.visible.get(pos as usize) {
                    let app = self.apps[i].clone();
                    if self.select.active() {
                        if app.is_removable() {
                            self.select.toggle(&app.id);
                        }
                    } else {
                        self.show_details(&app.id.clone(), w, &sender);
                    }
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
            Msg::UninstallSelected if self.select.active() => {
                if self.select.len() > 0 {
                    sender.input(Msg::UninstallMany);
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
            Msg::Removed(ids) => {
                if self.details.as_ref().is_some_and(|d| ids.contains(&d.app_id))
                    && w.nav.navigation_stack().n_items() > 1
                {
                    w.nav.pop();
                    self.details = None;
                }
                if !ids.is_empty() {
                    sender.input(Msg::SelectMode(false));
                }
                self.start_scan(&sender, w);
            }
            Msg::SelectMode(on) => {
                self.select.set_active(on);
                if w.select_toggle.is_active() != on {
                    w.select_toggle.set_active(on);
                }
            }
            Msg::SelectAll => {
                let listed: Vec<String> = self
                    .visible
                    .iter()
                    .map(|&i| &self.apps[i])
                    .filter(|a| a.is_removable())
                    .map(|a| a.id.clone())
                    .collect();
                if self.all_listed_selected() {
                    self.select.replace(self.select.ids().into_iter().filter(|id| !listed.contains(id)));
                } else {
                    self.select.replace(self.select.ids().into_iter().chain(listed));
                }
            }
            Msg::SelectApps(ids) => {
                sender.input(Msg::SelectMode(true));
                self.select.set_active(true);
                self.select.replace(ids);
            }
            Msg::SelectionChanged => {
                w.toolbar.set_reveal_bottom_bars(self.select.active());
                self.update_summary(w);
            }
            Msg::UninstallMany => {
                let ids = self.select.ids();
                crate::uninstall::request_many(self, &ids, root, &sender);
            }
            Msg::AskElevate => {
                if self.elevating || self.progress.is_some() {
                    return;
                }
                let d = adw::AlertDialog::new(
                    Some(&tr("Restart as Administrator?")),
                    Some(&tr(
                        "AppReestr reopens with full system access. The password is asked once; removals then do not ask for it.",
                    )),
                );
                d.add_response("cancel", &tr("Cancel"));
                d.add_response("restart", &tr("Restart"));
                d.set_response_appearance("restart", adw::ResponseAppearance::Suggested);
                d.set_default_response(Some("cancel"));
                d.set_close_response("cancel");
                let s = sender.clone();
                d.connect_response(None, move |_, r| {
                    if r == "restart" {
                        s.input(Msg::Elevate);
                    }
                });
                d.present(Some(root));
            }
            Msg::Elevate => {
                self.elevating = true;
                if let Some(b) = &w.admin_button {
                    b.set_sensitive(false);
                }
                sender.spawn_command(move |out| {
                    let out = Arc::new(out);
                    crate::removal::launch_elevated(move |e| {
                        let _ = out.send(Cmd::Elevate(e));
                    });
                });
            }
            Msg::DangerZone => crate::danger::load(self, &sender),
            Msg::ForcePlan(name) => crate::danger::plan_package(self, name, &sender),
            Msg::CleanPackages(names) => crate::uninstall::clean_packages(self, names, &sender),
            Msg::CleanFlatpak(scope) => {
                let need_agent = scope == FlatpakScope::System;
                crate::uninstall::start(
                    self,
                    crate::uninstall::JobRequest::new(crate::removal::Job {
                        title: tr("Unused Flatpak runtimes"),
                        steps: vec![crate::removal::Step::FlatpakUnused { system: scope == FlatpakScope::System }],
                        need_agent,
                    }),
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
                crate::theme::apply(&self.settings);
                self.save_settings();
            }
            Msg::SetTransparency(v) => {
                self.settings.transparency = v;
                crate::theme::apply(&self.settings);
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
            Msg::ClearSearch => w.search.set_text(""),
            Msg::FocusList => {
                if !self.visible.is_empty() {
                    w.list_view.scroll_to(0, gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT, None);
                }
            }
            Msg::Back => {
                if w.nav.navigation_stack().n_items() > 1 {
                    w.nav.pop();
                } else if self.select.active() {
                    sender.input(Msg::SelectMode(false));
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
                // Ticked applications that are gone (just uninstalled, say) are forgotten.
                let ticked = self.select.ids();
                self.select.replace(ticked.into_iter().filter(|id| self.apps.iter().any(|a| &a.id == id)));
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
            Cmd::Elevate(e) => match e {
                crate::removal::Event::Password { message, reply } => {
                    crate::uninstall::ask_password(root, &message, reply)
                }
                // The privileged copy is up: this one has nothing left to do.
                crate::removal::Event::Done(Ok(())) => relm4::main_application().quit(),
                crate::removal::Event::Done(Err(msg)) => {
                    self.elevating = false;
                    if let Some(b) = &w.admin_button {
                        b.set_sensitive(true);
                    }
                    w.toasts.add_toast(adw::Toast::new(&msg));
                }
                _ => {}
            },
            Cmd::DangerListing(l) => crate::danger::picker(*l, root, &sender),
            Cmd::ForcePlanned(p) => crate::danger::confirm(*p, root, &sender),
        }
    }
}
