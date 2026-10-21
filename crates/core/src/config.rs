//! User settings and manual marks, stored as TOML in `~/.config/appreestr/`.

use crate::model::UserMark;
use crate::sysroot::{SysRoot, as_user};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Date,
    Source,
}

pub const MAX_TRANSPARENCY: u8 = 90;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_system: bool,
    pub show_console: bool,
    pub sort: SortKey,
    pub sort_descending: bool,
    pub theme: Theme,
    /// Window background transparency in percent (0 = opaque, at most `MAX_TRANSPARENCY`).
    pub transparency: u8,
    /// Directories searched for AppImages (relative paths are relative to $HOME).
    pub appimage_dirs: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_system: false,
            show_console: false,
            sort: SortKey::Name,
            sort_descending: false,
            theme: Theme::System,
            transparency: 0,
            appimage_dirs: ["Applications", "AppImages", "Apps", ".local/bin", "Downloads", "Desktop", "bin", "opt"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    /// Unit key → the user's mark.
    pub marks: BTreeMap<String, UserMark>,
}

impl Overrides {
    pub fn as_map(&self) -> HashMap<String, UserMark> {
        self.marks.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }
}

pub struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    pub fn new(sys: &SysRoot) -> Self {
        Self { dir: sys.config_home.join("appreestr") }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn rules_file(&self) -> PathBuf {
        self.dir.join("rules.toml")
    }

    fn load<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> T {
        match std::fs::read_to_string(self.dir.join(name)) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("ignoring broken {name}: {e}");
                T::default()
            }),
            Err(_) => T::default(),
        }
    }

    fn save<T: Serialize>(&self, name: &str, value: &T) -> std::io::Result<()> {
        let text = toml::to_string_pretty(value).map_err(std::io::Error::other)?;
        as_user(|| {
            std::fs::create_dir_all(&self.dir)?;
            let tmp = self.dir.join(format!(".{name}.tmp"));
            std::fs::write(&tmp, text)?;
            std::fs::rename(tmp, self.dir.join(name))
        })
    }

    pub fn settings(&self) -> Settings {
        self.load("config.toml")
    }

    pub fn save_settings(&self, s: &Settings) -> std::io::Result<()> {
        self.save("config.toml", s)
    }

    pub fn overrides(&self) -> Overrides {
        self.load("overrides.toml")
    }

    pub fn save_overrides(&self, o: &Overrides) -> std::io::Result<()> {
        self.save("overrides.toml", o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("appreestr-cfg-{}", std::process::id()));
        let sys = SysRoot::fixture(&dir);
        let store = ConfigStore::new(&sys);
        let mut o = Overrides::default();
        o.marks.insert("pacman:avahi".into(), UserMark::Hidden);
        store.save_overrides(&o).unwrap();
        assert_eq!(store.overrides(), o);
        assert_eq!(store.settings(), Settings::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
