//! Flatpak applications and runtimes (user and system installations).
//!
//! Metadata comes from `flatpak list` (the stable public interface); launchers, metainfo and
//! install dates come from the deploy directories on disk. Without the `flatpak` binary the
//! source is simply empty.

use crate::appstream;
use crate::desktop::{self, DesktopIndex};
use crate::model::{FlatpakScope, Installation, Source, UnitKey};
use crate::sysroot::SysRoot;
use std::path::{Path, PathBuf};
use std::process::Command;

const COLUMNS: &str = "application,name,version,branch,arch,origin,installation,ref,size";

#[derive(Debug, Clone, PartialEq)]
pub struct ListRow {
    pub app_id: String,
    pub name: String,
    pub version: Option<String>,
    pub branch: String,
    pub arch: String,
    pub origin: Option<String>,
    pub installation: String,
    pub flatpak_ref: String,
    pub size: Option<u64>,
}

/// Parse `flatpak list --columns=…` output (tab separated when not a terminal).
pub fn parse_list(out: &str) -> Vec<ListRow> {
    out.lines()
        .filter_map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() < 9 {
                return None;
            }
            let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
            Some(ListRow {
                app_id: c[0].trim().to_string(),
                name: c[1].trim().to_string(),
                version: opt(c[2]),
                branch: c[3].trim().to_string(),
                arch: c[4].trim().to_string(),
                origin: opt(c[5]),
                installation: c[6].trim().to_string(),
                flatpak_ref: c[7].trim().to_string(),
                size: parse_size(c[8]),
            })
        })
        .collect()
}

/// `1.2 GB`, `350.4 MB`, `12 kB` (LC_ALL=C output; decimal units).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().replace('\u{a0}', " ");
    let (num, unit) = s.split_once(' ').unwrap_or((&s, "B"));
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let mult = match unit.trim() {
        "B" | "bytes" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((n * mult) as u64)
}

pub fn available() -> bool {
    which("flatpak").is_some()
}

pub fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(bin)).find(|p| p.is_file())
}

fn run_list(kind: &str) -> Vec<ListRow> {
    let out = Command::new("flatpak").args(["list", kind, &format!("--columns={COLUMNS}")]).env("LC_ALL", "C").output();
    match out {
        Ok(o) if o.status.success() => parse_list(&String::from_utf8_lossy(&o.stdout)),
        Ok(o) => {
            tracing::warn!("flatpak list failed: {}", String::from_utf8_lossy(&o.stderr));
            Vec::new()
        }
        Err(e) => {
            tracing::warn!("cannot run flatpak: {e}");
            Vec::new()
        }
    }
}

fn install_root(sys: &SysRoot, scope: FlatpakScope) -> PathBuf {
    match scope {
        FlatpakScope::User => sys.data_home.join("flatpak"),
        FlatpakScope::System => sys.path("var/lib/flatpak"),
    }
}

pub fn scan(sys: &SysRoot, desktop: &DesktopIndex) -> Vec<Installation> {
    if !sys.is_host() || !available() {
        return Vec::new();
    }
    let mut units = Vec::new();
    for (kind, runtime) in [("--app", false), ("--runtime", true)] {
        for row in run_list(kind) {
            units.push(build_unit(sys, desktop, row, runtime));
        }
    }
    units
}

pub fn build_unit(sys: &SysRoot, desktop: &DesktopIndex, row: ListRow, runtime: bool) -> Installation {
    let scope = if row.installation == "user" { FlatpakScope::User } else { FlatpakScope::System };
    let scope_name = if scope == FlatpakScope::User { "user" } else { "system" };
    let key = UnitKey::new("flatpak", format!("{scope_name}:{}", row.flatpak_ref));
    let source = Source::Flatpak { scope, remote: row.origin.clone(), flatpak_ref: row.flatpak_ref.clone() };
    let mut unit = Installation::new(key, source, &row.app_id);
    unit.version = row.version.clone();
    unit.size = row.size;
    unit.is_runtime = runtime;

    let base = install_root(sys, scope).join(if runtime { "runtime" } else { "app" }).join(&row.app_id);
    let deploy = base.join(&row.arch).join(&row.branch).join("active");
    unit.installed_at = file_time(&base, true);
    unit.updated_at = file_time(&deploy, false);
    unit.locations = vec![deploy.clone()];
    let data = sys.home.join(".var/app").join(&row.app_id);
    if !runtime && data.is_dir() {
        unit.locations.push(data);
    }

    if !runtime {
        let exports = install_root(sys, scope).join("exports/share/applications");
        let mut entries: Vec<_> = desktop
            .all()
            .iter()
            .filter(|e| e.path.starts_with(&exports))
            .filter(|e| e.id_stem() == row.app_id || e.id_stem().starts_with(&format!("{}.", row.app_id)))
            .filter_map(|e| desktop.effective_for_path(&e.path).cloned())
            .collect();
        if entries.is_empty() {
            // Not exported (e.g. index not refreshed yet): read it from the deploy dir.
            let p = deploy.join("export/share/applications").join(format!("{}.desktop", row.app_id));
            if let Ok(text) = std::fs::read_to_string(&p)
                && let Some(e) = desktop::parse(&text, &format!("{}.desktop", row.app_id), &p, sys.locale.as_deref())
            {
                entries.push(e);
            }
        }
        unit.desktop_entries = entries;
        let mi = deploy.join("files/share/metainfo");
        for name in [format!("{}.metainfo.xml", row.app_id), format!("{}.appdata.xml", row.app_id)] {
            if let Ok(text) = std::fs::read_to_string(mi.join(&name)) {
                unit.metainfo = appstream::parse(&text, sys.locale.as_deref());
                break;
            }
        }
    }
    if unit.metainfo.is_none() && unit.desktop_entries.is_empty() && !row.name.is_empty() {
        unit.description = Some(row.name.clone());
    }
    unit
}

fn file_time(p: &Path, prefer_created: bool) -> Option<i64> {
    let md = std::fs::symlink_metadata(p).ok()?;
    let t = if prefer_created { md.created().or_else(|_| md.modified()) } else { md.modified() };
    t.ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_output() {
        let out = "org.mozilla.firefox\tFirefox\t131.0\tstable\tx86_64\tflathub\tsystem\tapp/org.mozilla.firefox/x86_64/stable\t312.4 MB\n\
                   org.gnome.Platform\tGNOME Application Platform version 47\t\t47\tx86_64\tflathub\tuser\truntime/org.gnome.Platform/x86_64/47\t1.1 GB\n";
        let rows = parse_list(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].app_id, "org.mozilla.firefox");
        assert_eq!(rows[0].size, Some(312_400_000));
        assert_eq!(rows[1].version, None);
        assert_eq!(rows[1].installation, "user");
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("12 kB"), Some(12_000));
        assert_eq!(parse_size("1,5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("?"), None);
    }
}
