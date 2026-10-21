//! AppImage files. Metadata (launcher and icon) is read from the embedded SquashFS image
//! without executing the file, and cached by path, size and modification time.

use crate::desktop::{self, DesktopIndex};
use crate::model::{DesktopEntry, Installation, Source, UnitKey};
use crate::sysroot::SysRoot;
use backhand::{FilesystemReader, InnerNode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// Directories searched one level deeper (apps kept in their own folder).
const NESTED_DIRS: &[&str] = &["Applications", "AppImages", "Apps"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Type1,
    Type2,
}

/// Recognise an AppImage by its magic bytes (`AI\x01`/`AI\x02` at offset 8 of an ELF file).
pub fn detect(header: &[u8]) -> Option<Kind> {
    if header.len() < 11 || &header[0..4] != b"\x7fELF" || &header[8..10] != b"AI" {
        return None;
    }
    match header[10] {
        1 => Some(Kind::Type1),
        2 => Some(Kind::Type2),
        _ => None,
    }
}

/// End of the ELF runtime = start of the SquashFS image (end of the section header table).
pub fn squashfs_offset(header: &[u8]) -> Option<u64> {
    if header.len() < 64 {
        return None;
    }
    let le = header[5] == 1;
    let u16_at = |o: usize| {
        let b = [header[o], header[o + 1]];
        if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
    };
    let u32_at = |o: usize| {
        let b: [u8; 4] = header[o..o + 4].try_into().unwrap();
        if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
    };
    let u64_at = |o: usize| {
        let b: [u8; 8] = header[o..o + 8].try_into().unwrap();
        if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) }
    };
    match header[4] {
        2 => Some(u64_at(0x28) + u64::from(u16_at(0x3A)) * u64::from(u16_at(0x3C))),
        1 => Some(u64::from(u32_at(0x20)) + u64::from(u16_at(0x2E)) * u64::from(u16_at(0x30))),
        _ => None,
    }
}

/// Metadata extracted from an AppImage, cached between scans.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub size: u64,
    pub mtime: i64,
    pub desktop: Option<String>,
    pub desktop_name: Option<String>,
    pub icon: Option<PathBuf>,
}

/// Read the root `.desktop` file and the icon from the embedded image.
pub fn extract(path: &Path, icon_dir: &Path) -> Option<(Option<String>, Option<String>, Option<PathBuf>)> {
    let mut header = [0u8; 64];
    File::open(path).ok()?.read_exact(&mut header).ok()?;
    if detect(&header)? != Kind::Type2 {
        return Some((None, None, None));
    }
    let offset = squashfs_offset(&header)?;
    let reader = BufReader::new(File::open(path).ok()?);
    let fs = FilesystemReader::from_reader_with_offset(reader, offset).ok()?;

    let read = |want: &Path| -> Option<Vec<u8>> {
        let mut target = want.to_path_buf();
        for _ in 0..4 {
            let node = fs.files().find(|n| n.fullpath == target)?;
            match &node.inner {
                InnerNode::File(f) => {
                    let mut buf = Vec::new();
                    fs.file(f).reader().read_to_end(&mut buf).ok()?;
                    return Some(buf);
                }
                InnerNode::Symlink(s) => {
                    target = if s.link.is_absolute() { s.link.clone() } else { Path::new("/").join(&s.link) };
                }
                _ => return None,
            }
        }
        None
    };

    let desktop_node = fs.files().find(|n| {
        n.fullpath.parent() == Some(Path::new("/")) && n.fullpath.extension().is_some_and(|e| e == "desktop")
    });
    let desktop_name = desktop_node.and_then(|n| n.fullpath.file_name()).map(|f| f.to_string_lossy().into_owned());
    let desktop_text = desktop_node
        .map(|n| n.fullpath.clone())
        .and_then(|p| read(&p))
        .map(|b| String::from_utf8_lossy(&b).into_owned());

    let icon_name =
        desktop_text.as_deref().and_then(|t| desktop::parse(t, "x.desktop", path, None)).and_then(|e| e.icon);
    let mut icon = None;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(name) = &icon_name {
        for ext in ["png", "svg", "svgz", "xpm"] {
            candidates.push(Path::new("/").join(format!("{name}.{ext}")));
        }
    }
    candidates.push(PathBuf::from("/.DirIcon"));
    for c in candidates {
        if let Some(bytes) = read(&c) {
            let ext = if bytes.starts_with(b"\x89PNG") {
                "png"
            } else if bytes.starts_with(b"<?xml") || bytes.starts_with(b"<svg") {
                "svg"
            } else {
                continue;
            };
            let mut h = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut h);
            let file = icon_dir.join(format!("appimage-{:016x}.{ext}", h.finish()));
            if std::fs::create_dir_all(icon_dir).is_ok() && std::fs::write(&file, &bytes).is_ok() {
                icon = Some(file);
            }
            break;
        }
    }
    Some((desktop_text, desktop_name, icon))
}

/// `Obsidian-1.6.7-x86_64.AppImage` → ("Obsidian", Some("1.6.7")).
pub fn name_and_version(file_name: &str) -> (String, Option<String>) {
    let stem = file_name
        .strip_suffix(".AppImage")
        .or_else(|| file_name.strip_suffix(".appimage"))
        .or_else(|| file_name.strip_suffix(".Appimage"))
        .unwrap_or(file_name);
    let tokens: Vec<&str> = stem.split(['-', '_']).collect();
    let is_ver = |t: &str| {
        t.chars().next().is_some_and(|c| c.is_ascii_digit())
            || (t.starts_with('v') && t[1..].starts_with(|c: char| c.is_ascii_digit()))
    };
    let is_arch = |t: &str| {
        matches!(
            t.to_lowercase().as_str(),
            "x86" | "64" | "x86_64" | "amd64" | "x64" | "linux" | "aarch64" | "arm64" | "i386" | "appimage"
        )
    };
    match tokens.iter().position(|t| is_ver(t)) {
        Some(0) | None => (stem.replace(['-', '_'], " ").trim().to_string(), None),
        Some(i) => {
            let name = tokens[..i].iter().filter(|t| !is_arch(t)).cloned().collect::<Vec<_>>().join(" ");
            let ver: Vec<&str> = tokens[i..]
                .iter()
                .take_while(|t| !is_arch(t) && t.chars().any(|c| c.is_ascii_digit()))
                .cloned()
                .collect();
            (name, Some(ver.join("-").trim_start_matches('v').to_string()).filter(|v| !v.is_empty()))
        }
    }
}

fn cache_path(sys: &SysRoot) -> PathBuf {
    sys.cache_home.join("appreestr/appimages.json")
}

fn mtime(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_appimage(path: &Path) -> bool {
    let mut h = [0u8; 16];
    File::open(path).and_then(|mut f| f.read_exact(&mut h)).is_ok() && detect(&h).is_some()
}

pub fn scan_dirs(sys: &SysRoot, dirs: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for d in dirs {
        let dir = if Path::new(d).is_absolute() { sys.path(d) } else { sys.home.join(d) };
        let nested = NESTED_DIRS.contains(&d.as_str());
        collect(&dir, nested, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn collect(dir: &Path, nested: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if nested {
                collect(&p, false, out);
            }
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            e.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        };
        if (name.ends_with(".appimage") || executable) && is_appimage(&p) {
            out.push(p);
        }
    }
}

/// AppImage files found in the configured dirs and referenced by launchers.
pub fn scan(sys: &SysRoot, desktop: &DesktopIndex, dirs: &[String]) -> Vec<Installation> {
    let mut files = scan_dirs(sys, dirs);
    // Launchers created by AppImageLauncher, Gear Lever or by hand point at the file.
    let mut integration: HashMap<PathBuf, Vec<DesktopEntry>> = HashMap::new();
    for e in desktop.effective() {
        let Some(prog) = e.exec.as_deref().and_then(desktop::exec_program) else { continue };
        let prog = PathBuf::from(prog);
        if prog.is_absolute() && (prog.to_string_lossy().to_lowercase().ends_with(".appimage") || is_appimage(&prog)) {
            if !files.contains(&prog) && prog.is_file() {
                files.push(prog.clone());
            }
            integration.entry(prog).or_default().push(e.clone());
        }
    }

    let cache_file = cache_path(sys);
    let mut cache: BTreeMap<PathBuf, Meta> =
        std::fs::read(&cache_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let icon_dir = sys.cache_home.join("appreestr/icons");
    let mut changed = false;
    let mut units = Vec::new();

    for path in files {
        let Ok(md) = std::fs::metadata(&path) else { continue };
        let (size, mt) = (md.len(), mtime(&md));
        let meta = match cache.get(&path) {
            Some(m) if m.size == size && m.mtime == mt => m.clone(),
            _ => {
                let (desktop_text, desktop_name, icon) = extract(&path, &icon_dir).unwrap_or_default();
                let m = Meta { size, mtime: mt, desktop: desktop_text, desktop_name, icon };
                cache.insert(path.clone(), m.clone());
                changed = true;
                m
            }
        };

        let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let (fallback_name, file_version) = name_and_version(&file_name);
        let mut unit =
            Installation::new(UnitKey::new("appimage", path.to_string_lossy()), Source::AppImage, &file_name);
        unit.size = Some(size);
        unit.installed_at = md
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .or(Some(mt));
        unit.updated_at = Some(mt);
        unit.locations = vec![path.clone()];
        unit.icon_path = meta.icon.clone().filter(|p| p.is_file());

        let embedded = meta.desktop.as_deref().and_then(|t| {
            let id = meta.desktop_name.clone().unwrap_or_else(|| "appimage.desktop".into());
            desktop::parse(t, &id, &path, sys.locale.as_deref())
        });
        unit.version = embedded
            .as_ref()
            .and(meta.desktop.as_deref())
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("X-AppImage-Version=")).map(|v| v.trim().to_string()))
            .or(file_version);
        let mut entries = integration.remove(&path).unwrap_or_default();
        if entries.is_empty() {
            if let Some(mut e) = embedded {
                // Not integrated into the menu, but it is still an application.
                e.no_display = false;
                e.hidden = false;
                entries.push(e);
            } else {
                entries.push(DesktopEntry {
                    id: format!("{file_name}.desktop"),
                    path: path.clone(),
                    name: fallback_name,
                    is_application: true,
                    ..Default::default()
                });
            }
        }
        unit.desktop_entries = entries;
        units.push(unit);
    }

    if changed {
        cache.retain(|p, _| p.exists());
        if let Some(dir) = cache_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(&cache) {
            let _ = std::fs::write(&cache_file, json);
        }
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use backhand::{FilesystemWriter, NodeHeader};
    use std::io::{Cursor, Write};

    fn fake_elf(squashfs_at: u64) -> Vec<u8> {
        let mut h = vec![0u8; squashfs_at as usize];
        h[0..4].copy_from_slice(b"\x7fELF");
        h[4] = 2; // 64-bit
        h[5] = 1; // little endian
        h[8..11].copy_from_slice(b"AI\x02");
        // e_shoff = 128, e_shentsize = 64, e_shnum = (squashfs_at - 128) / 64
        h[0x28..0x30].copy_from_slice(&128u64.to_le_bytes());
        h[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes());
        h[0x3C..0x3E].copy_from_slice(&(((squashfs_at - 128) / 64) as u16).to_le_bytes());
        h
    }

    #[test]
    fn reads_embedded_metadata() {
        let dir = std::env::temp_dir().join(format!("appreestr-ai-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut fs = FilesystemWriter::default();
        let hdr = NodeHeader::default();
        let desktop =
            "[Desktop Entry]\nType=Application\nName=Test App\nIcon=testapp\nExec=AppRun\nX-AppImage-Version=2.5\n";
        fs.push_file(Cursor::new(desktop.as_bytes()), "testapp.desktop", hdr).unwrap();
        fs.push_file(Cursor::new(b"\x89PNG\r\n\x1a\nfake".to_vec()), "testapp.png", hdr).unwrap();
        fs.push_symlink("testapp.png", ".DirIcon", hdr).unwrap();
        let mut image = Cursor::new(Vec::new());
        fs.write(&mut image).unwrap();

        let path = dir.join("Test_App-2.5-x86_64.AppImage");
        let mut f = File::create(&path).unwrap();
        f.write_all(&fake_elf(1024)).unwrap();
        f.write_all(image.get_ref()).unwrap();
        drop(f);

        let (text, name, icon) = extract(&path, &dir.join("icons")).unwrap();
        assert!(text.unwrap().contains("Name=Test App"));
        assert_eq!(name.as_deref(), Some("testapp.desktop"));
        assert!(icon.unwrap().is_file());

        let sys = SysRoot { home: dir.clone(), cache_home: dir.join("cache"), ..SysRoot::fixture(&dir) };
        let units = scan(&sys, &DesktopIndex::default(), &[".".to_string()]);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].version.as_deref(), Some("2.5"));
        assert_eq!(units[0].desktop_entries[0].name, "Test App");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_from_files() {
        assert_eq!(name_and_version("Obsidian-1.6.7.AppImage"), ("Obsidian".into(), Some("1.6.7".into())));
        assert_eq!(name_and_version("LM-Studio-0.3.5-2-x64.AppImage"), ("LM Studio".into(), Some("0.3.5-2".into())));
        assert_eq!(name_and_version("nvim.appimage"), ("nvim".into(), None));
        assert_eq!(name_and_version("Foo_v2.1_x86_64.AppImage"), ("Foo".into(), Some("2.1".into())));
    }

    #[test]
    fn magic() {
        assert_eq!(detect(&fake_elf(256)), Some(Kind::Type2));
        assert_eq!(squashfs_offset(&fake_elf(256)), Some(256));
        assert_eq!(detect(b"\x7fELF\x02\x01\x01\x00\x00\x00\x00"), None);
    }
}
