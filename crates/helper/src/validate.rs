//! Input validation for the privileged helper. Everything here must reject by default.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// pacman package names: `[a-z0-9@._+-]`, not starting with `-` or `.`.
pub fn package_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with(['-', '.'])
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

/// `app/org.example.App/x86_64/stable` or `runtime/…`.
pub fn flatpak_ref(s: &str) -> bool {
    let parts: Vec<&str> = s.split('/').collect();
    let ok = |p: &str| {
        !p.is_empty()
            && !p.starts_with(['-', '.'])
            && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    parts.len() == 4 && (parts[0] == "app" || parts[0] == "runtime") && parts[1..].iter().all(|p| ok(p))
}

pub fn snap_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && !s.starts_with('-')
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Directories whose direct children may be removed as manual installations.
const ALLOWED_PARENTS: &[&str] = &["/opt", "/usr/local/bin", "/usr/local/share/applications", "/usr/local/lib"];

/// A manual installation path: an existing direct child of an allowed directory, given in
/// canonical form (no symlinks, no `..`), on the same filesystem as its parent.
pub fn removable_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("path must be absolute".into());
    }
    let canonical = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    if canonical != path {
        return Err("path must be canonical (no symlinks or ..)".into());
    }
    let parent = path.parent().ok_or("no parent")?;
    if !ALLOWED_PARENTS.iter().any(|a| Path::new(a) == parent) {
        return Err(format!("only direct children of {} can be removed", ALLOWED_PARENTS.join(", ")));
    }
    let (md, pmd) = (
        std::fs::symlink_metadata(path).map_err(|e| e.to_string())?,
        std::fs::metadata(parent).map_err(|e| e.to_string())?,
    );
    if md.dev() != pmd.dev() {
        return Err("path is a mount point".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(package_name("firefox"));
        assert!(package_name("lib32-gcc-libs"));
        assert!(package_name("gtk+3"));
        assert!(package_name("ai.storyteller.pdfcraft"));
        assert!(!package_name("-Rdd"));
        assert!(!package_name("--cascade"));
        assert!(!package_name("foo bar"));
        assert!(!package_name("foo;rm"));
        assert!(!package_name("../x"));
        assert!(!package_name(""));
    }

    #[test]
    fn refs() {
        assert!(flatpak_ref("app/org.mozilla.firefox/x86_64/stable"));
        assert!(flatpak_ref("runtime/org.gnome.Platform/x86_64/47"));
        assert!(!flatpak_ref("app/org.x/x86_64"));
        assert!(!flatpak_ref("app/../x86_64/stable"));
        assert!(!flatpak_ref("app/--system/x86_64/stable"));
        assert!(!flatpak_ref("other/a/b/c"));
        assert!(snap_name("firefox"));
        assert!(!snap_name("--purge"));
    }

    #[test]
    fn paths() {
        assert!(removable_path(Path::new("relative")).is_err());
        assert!(removable_path(Path::new("/usr/bin")).is_err());
        assert!(removable_path(Path::new("/opt/../etc")).is_err());
        assert!(removable_path(Path::new("/opt")).is_err());
        assert!(removable_path(Path::new("/opt/definitely-missing-dir")).is_err());
        if let Some(e) = std::fs::read_dir("/opt").ok().and_then(|mut d| d.next()).and_then(|e| e.ok()) {
            let p = e.path();
            if p.canonicalize().ok().as_deref() == Some(p.as_path()) {
                assert!(removable_path(&p).is_ok());
            }
        }
    }
}
