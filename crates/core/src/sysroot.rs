//! Where to look: the filesystem root and the user's home, replaceable in tests.

use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SysRoot {
    /// Filesystem root (`/` on the host, a fixture directory in tests).
    pub root: PathBuf,
    pub home: PathBuf,
    pub data_home: PathBuf,
    pub config_home: PathBuf,
    pub cache_home: PathBuf,
    /// `$XDG_DATA_DIRS`, already joined with `root`.
    pub data_dirs: Vec<PathBuf>,
    /// Locale used for `Name[xx]` lookups, e.g. `ru_RU`.
    pub locale: Option<String>,
}

impl SysRoot {
    pub fn host() -> Self {
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let xdg = |var: &str, fallback: &str| {
            env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(fallback))
        };
        let data_home = xdg("XDG_DATA_HOME", ".local/share");
        let mut data_dirs: Vec<PathBuf> = env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        // Flatpak and Snap export dirs are not always in XDG_DATA_DIRS (profile.d not sourced
        // by the session); add them so their launchers and icons are found.
        for extra in [
            data_home.join("flatpak/exports/share"),
            PathBuf::from("/var/lib/flatpak/exports/share"),
            PathBuf::from("/var/lib/snapd/desktop"),
        ] {
            if !data_dirs.contains(&extra) {
                data_dirs.push(extra);
            }
        }
        Self {
            root: PathBuf::from("/"),
            config_home: xdg("XDG_CONFIG_HOME", ".config"),
            cache_home: xdg("XDG_CACHE_HOME", ".cache"),
            data_home,
            home,
            data_dirs,
            locale: current_locale(),
        }
    }

    /// A sysroot entirely inside `dir` (for tests): `dir/home/user` is the home directory.
    pub fn fixture(dir: &Path) -> Self {
        let home = dir.join("home/user");
        Self {
            root: dir.to_path_buf(),
            data_home: home.join(".local/share"),
            config_home: home.join(".config"),
            cache_home: home.join(".cache"),
            data_dirs: vec![dir.join("usr/local/share"), dir.join("usr/share")],
            home,
            locale: None,
        }
    }

    /// Resolve an absolute host path inside this root.
    pub fn path(&self, abs: impl AsRef<Path>) -> PathBuf {
        let abs = abs.as_ref();
        self.root.join(abs.strip_prefix("/").unwrap_or(abs))
    }

    /// Strip the root back off a path (for display and for package ownership lookups).
    pub fn host_path(&self, p: &Path) -> PathBuf {
        match p.strip_prefix(&self.root) {
            Ok(rel) => Path::new("/").join(rel),
            Err(_) => p.to_path_buf(),
        }
    }

    pub fn is_host(&self) -> bool {
        self.root == Path::new("/")
    }
}

fn current_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| env::var(v).ok())
        .find(|v| !v.is_empty())
        .map(|v| v.split(['.', '@']).next().unwrap_or("").to_string())
        .filter(|v| v != "C" && v != "POSIX" && !v.is_empty())
}
