//! Refresh the list when something is installed or removed outside AppReestr.

use crate::window::{AppModel, Msg};
use appreestr_core::sysroot::SysRoot;
use notify::{RecursiveMode, Watcher};
use relm4::ComponentSender;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Changes settle for this long before a rescan (pacman touches many files per transaction).
const DEBOUNCE: Duration = Duration::from_secs(2);

pub fn watch(
    sys: &SysRoot,
    appimage_dirs: &[String],
    sender: ComponentSender<AppModel>,
) -> Option<notify::RecommendedWatcher> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && !matches!(ev.kind, notify::EventKind::Access(_))
        {
            let _ = tx.send(());
        }
    })
    .ok()?;

    let mut paths: Vec<PathBuf> = vec![
        sys.path("var/lib/pacman/local"),
        sys.path("var/lib/flatpak/app"),
        sys.data_home.join("flatpak/app"),
        sys.path("var/lib/snapd/desktop/applications"),
        sys.data_home.join("applications"),
        sys.path("opt"),
        sys.path("usr/local/bin"),
    ];
    paths.extend(appimage_dirs.iter().map(|d| if d.starts_with('/') { PathBuf::from(d) } else { sys.home.join(d) }));
    for p in paths.iter().filter(|p| p.is_dir()) {
        if let Err(e) = watcher.watch(p, RecursiveMode::NonRecursive) {
            tracing::debug!("cannot watch {}: {e}", p.display());
        }
    }

    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Wait until changes stop for a moment.
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            sender.input(Msg::ExternalChange);
        }
    });
    Some(watcher)
}
