//! Runs every source, classifies and merges the results.

use crate::classify::{self, Context};
use crate::config::Settings;
use crate::desktop::DesktopIndex;
use crate::merge::{self, Classified};
use crate::model::{App, Installation, UserMark};
use crate::protect::{self, Protection};
use crate::rules::Rules;
use crate::sources::pacman::{OwnerIndex, PacmanData};
use crate::sources::{appimage, flatpak, manual, pacman, snap};
use crate::sysroot::SysRoot;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct ScanResult {
    pub apps: Vec<App>,
    pub owners: OwnerIndex,
    /// pacman package → installed packages it depends on.
    pub depends: HashMap<String, Vec<String>>,
    pub hold_pkgs: Vec<String>,
    pub kernels: Vec<String>,
    pub timings: Vec<(&'static str, Duration)>,
    pub warnings: Vec<String>,
}

pub struct ScanInput<'a> {
    pub sys: &'a SysRoot,
    pub rules: &'a Rules,
    pub settings: &'a Settings,
    pub marks: &'a HashMap<String, UserMark>,
}

fn timed<T>(timings: &mut Vec<(&'static str, Duration)>, name: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    timings.push((name, t.elapsed()));
    r
}

pub fn scan(input: &ScanInput) -> ScanResult {
    let sys = input.sys;
    let mut res = ScanResult::default();
    let total = Instant::now();
    let desktop = timed(&mut res.timings, "desktop", || DesktopIndex::scan(sys));

    let (pac, fp, ai, sn) = std::thread::scope(|s| {
        let pac = s.spawn(|| {
            let t = Instant::now();
            (pacman::scan(sys, &desktop, input.rules), t.elapsed())
        });
        let fp = s.spawn(|| {
            let t = Instant::now();
            (flatpak::scan(sys, &desktop), t.elapsed())
        });
        let ai = s.spawn(|| {
            let t = Instant::now();
            (appimage::scan(sys, &desktop, &input.settings.appimage_dirs), t.elapsed())
        });
        let sn = s.spawn(|| {
            let t = Instant::now();
            (snap::scan(sys, &desktop), t.elapsed())
        });
        (
            pac.join().expect("pacman scan panicked"),
            fp.join().expect("flatpak scan panicked"),
            ai.join().expect("appimage scan panicked"),
            sn.join().expect("snap scan panicked"),
        )
    });
    let ((pac, pac_t), (fp, fp_t), (ai, ai_t), (sn, sn_t)) = (pac, fp, ai, sn);
    res.timings.push(("pacman", pac_t));
    res.timings.push(("flatpak", fp_t));
    res.timings.push(("appimage", ai_t));
    res.timings.push(("snap", sn_t));

    let pac = pac.unwrap_or_else(|e| {
        res.warnings.push(format!("pacman: {e:#}"));
        PacmanData::default()
    });

    let mut units: Vec<Installation> = pac.units;
    units.extend(fp);
    units.extend(ai);
    units.extend(sn);

    let man = timed(&mut res.timings, "manual", || {
        let claimed: HashSet<PathBuf> =
            units.iter().flat_map(|u| u.desktop_entries.iter().map(|e| e.path.clone())).collect();
        manual::scan(sys, &desktop, &pac.owners, &claimed)
    });
    // User launchers that start a packaged program belong to that package.
    for (pkg, entry) in man.links {
        if let Some(u) = units.iter_mut().find(|u| u.source.is_pacman() && u.unit_name == pkg)
            && !u.desktop_entries.iter().any(|e| e.id == entry.id)
        {
            u.desktop_entries.push(entry);
        }
    }
    units.extend(man.units);

    res.apps = timed(&mut res.timings, "classify", || classify_and_merge(units, input.rules, input.marks));
    res.owners = pac.owners;
    res.depends = pac.depends;
    res.hold_pkgs = pac.hold_pkgs;
    res.kernels = pac.kernels;
    res.timings.push(("total", total.elapsed()));
    res
}

pub fn classify_and_merge(units: Vec<Installation>, rules: &Rules, marks: &HashMap<String, UserMark>) -> Vec<App> {
    let sets: HashSet<String> = units.iter().filter(|u| classify::is_set(u)).map(|u| u.unit_name.clone()).collect();
    let ctx = Context { rules, sets: &sets };
    let classified = units
        .into_iter()
        .map(|unit| {
            let (visibility, reasons) = classify::classify(&unit, &ctx);
            Classified { unit, visibility, reasons }
        })
        .collect();
    merge::merge(classified, marks)
}

impl ScanResult {
    /// Protection as of now (running processes change, so compute it when needed).
    pub fn protection(&self, sys: &SysRoot, rules: &Rules) -> Protection {
        let processes = protect::processes(sys);
        protect::analyze(&protect::Input {
            sys,
            rules,
            owners: &self.owners,
            depends: &self.depends,
            hold_pkgs: &self.hold_pkgs,
            kernels: &self.kernels,
            processes: &processes,
        })
    }
}
