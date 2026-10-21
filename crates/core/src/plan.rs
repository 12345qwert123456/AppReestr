//! Removal planning with libalpm, without root and without touching the system: the same
//! transaction pacman would run (`pacman -Rs`), prepared with `NO_LOCK` and released.

use crate::protect::{Block, Protection, Warning};
use alpm::{Alpm, PackageReason, PrepareData, TransFlag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedPkg {
    pub name: String,
    pub version: String,
    pub size: u64,
    pub explicit: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemovalPlan {
    pub targets: Vec<String>,
    /// Everything the transaction removes (targets and their no-longer-needed dependencies).
    pub removed: Vec<PlannedPkg>,
    pub total_size: u64,
    /// Removal is impossible: (package being removed, packages that need it).
    pub needed_by: Vec<(String, Vec<String>)>,
    /// Protected packages in the removal set.
    pub blocked: Vec<(String, Block)>,
    pub warnings: Vec<(String, Warning)>,
    /// Installed packages that use something being removed as an optional dependency.
    pub optional_for: Vec<(String, Vec<String>)>,
    /// Fingerprint of `removed`, checked again by the privileged helper.
    pub fingerprint: String,
}

impl RemovalPlan {
    pub fn can_remove(&self) -> bool {
        self.needed_by.is_empty() && self.blocked.is_empty() && !self.removed.is_empty()
    }

    /// Packages removed besides the requested ones.
    pub fn extra(&self) -> impl Iterator<Item = &PlannedPkg> {
        self.removed.iter().filter(|p| !self.targets.contains(&p.name))
    }
}

/// FNV-1a over the sorted names: stable across builds and processes.
pub fn fingerprint<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let sorted: BTreeSet<&str> = names.into_iter().collect();
    let mut h: u64 = 0xcbf29ce484222325;
    for n in sorted {
        for b in n.bytes().chain(std::iter::once(b'\n')) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("package {0} is not installed")]
    NotInstalled(String),
    #[error("libalpm: {0}")]
    Alpm(String),
}

pub fn open_local(root: &str, dbpath: &str) -> Result<Alpm, PlanError> {
    Alpm::new(root, dbpath).map_err(|e| PlanError::Alpm(e.to_string()))
}

/// Simulate `pacman -R[s]` for `targets`.
pub fn plan(
    handle: &mut Alpm,
    targets: &[String],
    recursive: bool,
    protection: Option<&Protection>,
) -> Result<RemovalPlan, PlanError> {
    let mut flags = TransFlag::NO_LOCK;
    if recursive {
        flags |= TransFlag::RECURSE;
    }
    let mut out = RemovalPlan { targets: targets.to_vec(), ..Default::default() };
    handle.trans_init(flags).map_err(|e| PlanError::Alpm(e.to_string()))?;
    let result = (|| {
        for t in targets {
            let pkg = handle.localdb().pkg(t.as_str()).map_err(|_| PlanError::NotInstalled(t.clone()))?;
            handle.trans_remove_pkg(pkg).map_err(|e| PlanError::Alpm(e.to_string()))?;
        }
        // Turn the error into owned data first: it borrows the handle.
        let prepared: Result<(), Result<Vec<(String, String)>, String>> =
            handle.trans_prepare().map_err(|e| match e.data() {
                Some(PrepareData::UnsatisfiedDeps(list)) => Ok(list
                    .iter()
                    .map(|m| (m.causing_pkg().unwrap_or(m.depend().name()).to_string(), m.target().to_string()))
                    .collect()),
                _ => Err(e.to_string()),
            });
        match prepared {
            Ok(()) => {
                for p in handle.trans_remove() {
                    out.removed.push(PlannedPkg {
                        name: p.name().to_string(),
                        version: p.version().as_str().to_string(),
                        size: u64::try_from(p.isize()).unwrap_or(0),
                        explicit: p.reason() == PackageReason::Explicit,
                    });
                }
                Ok(())
            }
            Err(Ok(missing)) => {
                for (removing, needer) in missing {
                    match out.needed_by.iter_mut().find(|(r, _)| *r == removing) {
                        Some((_, v)) => {
                            if !v.contains(&needer) {
                                v.push(needer)
                            }
                        }
                        None => out.needed_by.push((removing, vec![needer])),
                    }
                }
                Ok(())
            }
            Err(Err(msg)) => Err(PlanError::Alpm(msg)),
        }
    })();
    let _ = handle.trans_release();
    result?;

    out.removed.sort_by(|a, b| a.name.cmp(&b.name));
    out.total_size = out.removed.iter().map(|p| p.size).sum();
    out.fingerprint = fingerprint(out.removed.iter().map(|p| p.name.as_str()));

    // Optional users that stay installed.
    let removed_names: BTreeSet<&str> = out.removed.iter().map(|p| p.name.as_str()).collect();
    for p in &out.removed {
        if let Ok(pkg) = handle.localdb().pkg(p.name.as_str()) {
            let users: Vec<String> =
                pkg.optional_for().iter().filter(|u| !removed_names.contains(*u)).map(String::from).collect();
            if !users.is_empty() {
                out.optional_for.push((p.name.clone(), users));
            }
        }
    }

    if let Some(prot) = protection {
        let names: Vec<&str> = if out.removed.is_empty() {
            targets.iter().map(String::as_str).collect()
        } else {
            removed_names.iter().copied().collect()
        };
        for n in names {
            if let Some(b) = prot.block(n) {
                out.blocked.push((n.to_string(), b.clone()));
            }
            for w in prot.warnings(n) {
                out.warnings.push((n.to_string(), w));
            }
        }
    }
    Ok(out)
}

/// Orphans: dependencies nothing needs any more (`pacman -Qdt`).
pub fn orphans(handle: &Alpm) -> Vec<PlannedPkg> {
    let mut v: Vec<PlannedPkg> = handle
        .localdb()
        .pkgs()
        .iter()
        .filter(|p| p.reason() == PackageReason::Depend)
        .filter(|p| p.required_by().is_empty() && p.optional_for().is_empty())
        .map(|p| PlannedPkg {
            name: p.name().to_string(),
            version: p.version().as_str().to_string(),
            size: u64::try_from(p.isize()).unwrap_or(0),
            explicit: false,
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_order_independent() {
        assert_eq!(fingerprint(["b", "a"]), fingerprint(["a", "b"]));
        assert_ne!(fingerprint(["a"]), fingerprint(["a", "b"]));
    }

    /// Runs against the real pacman database when present (read-only).
    #[test]
    fn plans_on_host_without_root() {
        if !std::path::Path::new("/var/lib/pacman/local").is_dir() {
            return;
        }
        let mut h = open_local("/", "/var/lib/pacman").unwrap();
        // glibc is needed by everything: removal must be refused with dependents listed.
        let p = plan(&mut h, &["glibc".to_string()], true, None).unwrap();
        assert!(!p.can_remove());
        assert!(!p.needed_by.is_empty());
        assert!(matches!(plan(&mut h, &["no-such-package-x".into()], true, None), Err(PlanError::NotInstalled(_))));
    }
}
