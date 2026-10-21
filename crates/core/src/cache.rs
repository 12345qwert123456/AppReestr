//! Snapshot of the last scan, shown instantly at startup while a fresh scan runs.

use crate::model::App;
use crate::sysroot::as_user;
use std::path::{Path, PathBuf};

/// Bump when the model changes incompatibly.
const MAGIC: &[u8; 12] = b"APPREESTR\x00\x00\x04";

pub fn snapshot_path(cache_home: &Path) -> PathBuf {
    cache_home.join("appreestr/snapshot.bin")
}

pub fn save(path: &Path, apps: &[App]) -> std::io::Result<()> {
    let body = postcard::to_stdvec(apps).map_err(std::io::Error::other)?;
    let mut data = Vec::with_capacity(MAGIC.len() + body.len());
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&body);
    as_user(|| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(tmp, path)
    })
}

pub fn load(path: &Path) -> Option<Vec<App>> {
    let data = std::fs::read(path).ok()?;
    let body = data.strip_prefix(MAGIC.as_slice())?;
    postcard::from_bytes(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Installation, Source, UnitKey, Visibility};

    #[test]
    fn roundtrip() {
        let app = App {
            id: "a".into(),
            name: "A".into(),
            icon: Some("a".into()),
            summary: None,
            installs: vec![Installation::new(UnitKey::new("appimage", "/x"), Source::AppImage, "x")],
            visibility: Visibility::App,
            reasons: vec![],
            user_mark: None,
        };
        let p = std::env::temp_dir().join(format!("appreestr-snap-{}.bin", std::process::id()));
        save(&p, std::slice::from_ref(&app)).unwrap();
        assert_eq!(load(&p).unwrap(), vec![app]);
        std::fs::write(&p, b"garbage").unwrap();
        assert!(load(&p).is_none());
        let _ = std::fs::remove_file(p);
    }
}
