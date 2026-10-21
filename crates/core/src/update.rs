//! Optional "is there a newer release" check. It only reports; installing updates stays with the
//! package manager. Off unless the user turns it on, and never run by the administrator session.

use crate::sysroot::as_user;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LATEST_URL: &str = "https://api.github.com/repos/12345qwert123456/AppReestr/releases/latest";
/// A release page we are willing to open; the response is untrusted.
const PAGE_PREFIX: &str = "https://github.com/12345qwert123456/AppReestr/";
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
    html_url: String,
}

pub fn stamp_path(cache_home: &Path) -> PathBuf {
    cache_home.join("appreestr/update-check")
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// True when the last successful check is more than a day old (or there was none).
pub fn due(cache_home: &Path) -> bool {
    let last = std::fs::read_to_string(stamp_path(cache_home)).ok().and_then(|s| s.trim().parse::<u64>().ok());
    last.is_none_or(|t| now_secs().saturating_sub(t) >= INTERVAL.as_secs())
}

fn mark_checked(cache_home: &Path) {
    let path = stamp_path(cache_home);
    let res = as_user(|| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, now_secs().to_string())
    });
    if let Err(e) = res {
        tracing::warn!("saving update check time: {e}");
    }
}

/// Asks GitHub for the latest release at most once a day. `None` when it is not due, offline,
/// the answer is unusable, or `current` is already the newest.
pub fn check(cache_home: &Path, current: &str) -> Option<Release> {
    if nix::unistd::geteuid().is_root() || !due(cache_home) {
        return None;
    }
    #[cfg(debug_assertions)]
    let url = std::env::var("APPREESTR_UPDATE_URL").unwrap_or_else(|_| LATEST_URL.into());
    #[cfg(not(debug_assertions))]
    let url = LATEST_URL;
    // Plain http only for the local mock server used with debug builds.
    let proto = if cfg!(debug_assertions) { "=https,http" } else { "=https" };
    // curl is a dependency of pacman, so it is there; this avoids linking a TLS stack.
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "5", "--proto", proto, "-H", "Accept: application/vnd.github+json"])
        .arg(&url)
        .output()
        .map_err(|e| tracing::info!("update check: {e}"))
        .ok()?;
    if !out.status.success() {
        tracing::info!("update check failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        return None;
    }
    mark_checked(cache_home);
    let release = parse_release(&String::from_utf8_lossy(&out.stdout))?;
    is_newer(current, &release.version).then_some(release)
}

pub fn parse_release(json: &str) -> Option<Release> {
    let latest: Latest = serde_json::from_str(json).ok()?;
    let version = latest.tag_name.trim_start_matches('v').to_string();
    parse_version(&version)?;
    latest.html_url.starts_with(PAGE_PREFIX).then_some(Release { version, url: latest.html_url })
}

/// `1.2.3` → `[1, 2, 3]`; anything with a suffix (`-rc1`) or junk is rejected.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.split('.').map(|p| p.parse().ok()).collect()
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    let (Some(mut c), Some(mut l)) = (parse_version(current), parse_version(latest)) else { return false };
    let len = c.len().max(l.len());
    c.resize(len, 0);
    l.resize(len, 0);
    l > c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.2.0-rc1"));
        assert!(!is_newer("0.1.0", "garbage"));
    }

    #[test]
    fn parses_github_answer() {
        let ok = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/12345qwert123456/AppReestr/releases/tag/v0.2.0","x":1}"#;
        let r = parse_release(ok).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert!(r.url.ends_with("v0.2.0"));
        // A release page elsewhere is never offered to the user.
        let evil = r#"{"tag_name":"v9.9.9","html_url":"https://evil.example/releases"}"#;
        assert!(parse_release(evil).is_none());
        assert!(parse_release("not json").is_none());
        assert!(
            parse_release(r#"{"tag_name":"nightly","html_url":"https://github.com/12345qwert123456/AppReestr/x"}"#)
                .is_none()
        );
    }

    #[test]
    fn once_a_day() {
        let dir = std::env::temp_dir().join(format!("appreestr-upd-{}", std::process::id()));
        assert!(due(&dir));
        mark_checked(&dir);
        assert!(!due(&dir));
        std::fs::write(stamp_path(&dir), "1").unwrap();
        assert!(due(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }
}
