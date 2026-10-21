//! What must never be removed, and what deserves a warning.
//!
//! Blocked: everything the "critical roots" depend on — the base system, kernels, boot loader,
//! the running desktop session, the display manager, pacman itself (and pacman's HoldPkg).
//! Warned: running programs, enabled services, desktop components (bars, notifications…).

use crate::rules::{Rules, any_glob};
use crate::sources::pacman::OwnerIndex;
use crate::sysroot::SysRoot;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    /// Matches the critical list (`base`, `pacman`, boot loaders…).
    Essential(String),
    Kernel(String),
    HoldPkg(String),
    /// The compositor or desktop shell this session runs on.
    Session(String),
    DisplayManager(String),
    PolkitAgent(String),
    /// Needed by a blocked root.
    NeededBy {
        root: String,
        reason: Box<Block>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Warning {
    Running(Vec<String>),
    DesktopComponent(String),
    Service(Vec<String>),
    OptionalFor(Vec<String>),
}

#[derive(Clone, Debug, Default)]
pub struct Protection {
    pub blocked: HashMap<String, Block>,
    /// Package → names of its running processes.
    pub running: HashMap<String, Vec<String>>,
    pub desktop_components: HashMap<String, String>,
    /// Package → enabled systemd units.
    pub services: HashMap<String, Vec<String>>,
    /// A polkit authentication agent is running in the session.
    pub polkit_agent: bool,
}

/// Built-in agents of desktop shells (no separate agent process).
const SHELLS_WITH_AGENT: &[&str] =
    &["gnome-shell", "cinnamon", "budgie-polkit-dialog", "io.elementary.desktop.agent-polkit", "deepin-polkit-agent"];

#[derive(Debug, Clone)]
pub struct Process {
    pub comm: String,
    pub exe: Option<PathBuf>,
}

/// Processes visible to us (our own user's, and root's names without executables).
pub fn processes(sys: &SysRoot) -> Vec<Process> {
    let Ok(rd) = std::fs::read_dir(sys.path("proc")) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| {
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?.trim().to_string();
            let exe = std::fs::read_link(e.path().join("exe")).ok().map(|p| {
                // A replaced binary shows up as "/usr/bin/x (deleted)".
                PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)").to_string())
            });
            Some(Process { comm, exe })
        })
        .collect()
}

fn unit_owner(owners: &OwnerIndex, sys: &SysRoot, link: &Path) -> Option<(String, String)> {
    let target = std::fs::read_link(link).ok()?;
    let target = if target.is_absolute() { target } else { link.parent()?.join(target) };
    let host = sys.host_path(&target);
    let unit = host.file_name()?.to_string_lossy().into_owned();
    let pkg = owners.owner(&host.to_string_lossy())?;
    Some((pkg.to_string(), unit))
}

fn enabled_services(sys: &SysRoot, owners: &OwnerIndex) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for base in ["etc/systemd/system", "etc/systemd/user"] {
        let Ok(rd) = std::fs::read_dir(sys.path(base)) else { continue };
        for d in rd.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".wants") && !name.ends_with(".requires") {
                continue;
            }
            let Ok(links) = std::fs::read_dir(d.path()) else { continue };
            for l in links.flatten() {
                if let Some((pkg, unit)) = unit_owner(owners, sys, &l.path()) {
                    let e = map.entry(pkg).or_default();
                    if !e.contains(&unit) {
                        e.push(unit);
                    }
                }
            }
        }
    }
    map
}

pub struct Input<'a> {
    pub sys: &'a SysRoot,
    pub rules: &'a Rules,
    pub owners: &'a OwnerIndex,
    pub depends: &'a HashMap<String, Vec<String>>,
    pub hold_pkgs: &'a [String],
    pub kernels: &'a [String],
    pub processes: &'a [Process],
}

pub fn analyze(inp: &Input) -> Protection {
    let rules = &inp.rules.protect;
    let mut p = Protection::default();
    let mut roots: Vec<(String, Block)> = Vec::new();

    for name in inp.depends.keys() {
        if let Some(pat) = any_glob(&rules.critical, name) {
            roots.push((name.clone(), Block::Essential(pat)));
        }
    }
    for k in inp.kernels {
        roots.push((k.clone(), Block::Kernel(k.clone())));
    }
    for h in inp.hold_pkgs {
        if inp.depends.contains_key(h) {
            roots.push((h.clone(), Block::HoldPkg(h.clone())));
        }
    }

    // Running processes → owning packages.
    for proc in inp.processes {
        let exe_name = proc.exe.as_ref().and_then(|e| e.file_name()).map(|f| f.to_string_lossy().into_owned());
        let names = [Some(proc.comm.as_str()), exe_name.as_deref()];
        let owner = proc.exe.as_ref().and_then(|e| inp.owners.owner(&e.to_string_lossy())).map(String::from);
        let matches = |list: &[String]| names.iter().flatten().any(|n| list.iter().any(|l| l == n));
        if matches(&rules.polkit_agents) || names.iter().flatten().any(|n| SHELLS_WITH_AGENT.contains(n)) {
            p.polkit_agent = true;
        }
        let Some(owner) = owner else { continue };
        let label = exe_name.clone().unwrap_or_else(|| proc.comm.clone());
        if matches(&rules.session_processes) {
            roots.push((owner.clone(), Block::Session(label.clone())));
        } else if matches(&rules.polkit_agents) {
            roots.push((owner.clone(), Block::PolkitAgent(label.clone())));
        } else if matches(&rules.desktop_components) {
            p.desktop_components.insert(owner.clone(), label.clone());
        }
        let r = p.running.entry(owner).or_default();
        if !r.contains(&label) {
            r.push(label);
        }
    }

    // Display manager.
    if let Some((pkg, unit)) =
        unit_owner(inp.owners, inp.sys, &inp.sys.path("etc/systemd/system/display-manager.service"))
    {
        roots.push((pkg, Block::DisplayManager(unit.trim_end_matches(".service").to_string())));
    }
    p.services = enabled_services(inp.sys, inp.owners);

    // Closure over dependencies, remembering which root pulled each package in.
    let mut queue: VecDeque<String> = VecDeque::new();
    for (pkg, reason) in roots {
        if !p.blocked.contains_key(&pkg) {
            p.blocked.insert(pkg.clone(), reason);
            queue.push_back(pkg);
        }
    }
    while let Some(pkg) = queue.pop_front() {
        let reason = p.blocked[&pkg].clone();
        let root_reason = match &reason {
            Block::NeededBy { .. } => reason.clone(),
            other => Block::NeededBy { root: pkg.clone(), reason: Box::new(other.clone()) },
        };
        for dep in inp.depends.get(&pkg).into_iter().flatten() {
            if !p.blocked.contains_key(dep) {
                p.blocked.insert(dep.clone(), root_reason.clone());
                queue.push_back(dep.clone());
            }
        }
    }
    p
}

impl Protection {
    pub fn block(&self, pkg: &str) -> Option<&Block> {
        self.blocked.get(pkg)
    }

    pub fn warnings(&self, pkg: &str) -> Vec<Warning> {
        let mut w = Vec::new();
        if let Some(r) = self.running.get(pkg) {
            w.push(Warning::Running(r.clone()));
        }
        if let Some(c) = self.desktop_components.get(pkg) {
            w.push(Warning::DesktopComponent(c.clone()));
        }
        if let Some(s) = self.services.get(pkg) {
            w.push(Warning::Service(s.clone()));
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect()
    }

    #[test]
    fn closure_and_session() {
        let dir = std::env::temp_dir().join(format!("appreestr-prot-{}", std::process::id()));
        let sys = SysRoot::fixture(&dir);
        let rules = Rules::bundled();
        let depends = deps(&[
            ("base", &["glibc", "bash"]),
            ("glibc", &[]),
            ("bash", &["glibc"]),
            ("hyprland", &["aquamarine", "glibc"]),
            ("aquamarine", &[]),
            ("firefox", &["glibc", "nss"]),
            ("nss", &[]),
            ("waybar", &[]),
            ("linux-zen", &[]),
        ]);
        let mut owners = OwnerIndex::default();
        owners.insert_file("usr/bin/Hyprland", "hyprland");
        owners.insert_file("usr/bin/waybar", "waybar");
        owners.insert_file("usr/bin/firefox", "firefox");
        let processes = vec![
            Process { comm: "Hyprland".into(), exe: Some("/usr/bin/Hyprland".into()) },
            Process { comm: "waybar".into(), exe: Some("/usr/bin/waybar".into()) },
            Process { comm: "firefox".into(), exe: Some("/usr/bin/firefox".into()) },
        ];
        let p = analyze(&Input {
            sys: &sys,
            rules: &rules,
            owners: &owners,
            depends: &depends,
            hold_pkgs: &["glibc".to_string()],
            kernels: &["linux-zen".to_string()],
            processes: &processes,
        });
        assert!(matches!(p.block("base"), Some(Block::Essential(_))));
        assert!(matches!(p.block("bash"), Some(Block::NeededBy { .. })));
        assert!(matches!(p.block("hyprland"), Some(Block::Session(_))));
        assert!(matches!(p.block("aquamarine"), Some(Block::NeededBy { root, .. }) if root == "hyprland"));
        assert!(matches!(p.block("linux-zen"), Some(Block::Kernel(_))));
        assert!(p.block("firefox").is_none());
        assert!(p.block("nss").is_none());
        assert!(p.block("waybar").is_none());
        assert!(matches!(p.warnings("waybar")[..], [Warning::Running(_), Warning::DesktopComponent(_)]));
        assert!(matches!(p.warnings("firefox")[..], [Warning::Running(_)]));
        assert!(!p.polkit_agent);
    }
}
