//! Snap packages, read from snapd's REST API over its unix socket (no root needed).

use crate::desktop::DesktopIndex;
use crate::model::{Installation, Source, UnitKey};
use crate::sources::pacman_log::days_from_civil;
use crate::sysroot::SysRoot;
use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SOCKET: &str = "/run/snapd.socket";

fn get(path: &str) -> Option<String> {
    let mut s = UnixStream::connect(SOCKET).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").ok()?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    buf.split_once("\r\n\r\n").map(|(_, body)| body.to_string())
}

/// `2024-05-01T10:00:00.123456789+03:00` → unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let rest = &s[19..];
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let oh: i64 = digits.get(0..2)?.parse().ok()?;
            let om: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
            (oh * 3600 + om * 60) * if sign == '-' { -1 } else { 1 }
        }
        _ => 0,
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec - offset)
}

pub fn parse(json: &str, sys: &SysRoot, desktop: &DesktopIndex) -> Vec<Installation> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let Some(list) = v.get("result").and_then(Value::as_array) else { return Vec::new() };
    let str_of = |s: &Value, k: &str| s.get(k).and_then(Value::as_str).map(String::from);
    list.iter()
        .filter_map(|s| {
            let name = str_of(s, "name")?;
            let kind = str_of(s, "type").unwrap_or_else(|| "app".into());
            let publisher =
                s.get("publisher").and_then(|p| p.get("display-name")).and_then(Value::as_str).map(String::from);
            let mut u = Installation::new(
                UnitKey::new("snap", &name),
                Source::Snap { publisher, channel: str_of(s, "channel") },
                &name,
            );
            u.version = str_of(s, "version");
            u.size = s.get("installed-size").and_then(Value::as_u64);
            u.installed_at = str_of(s, "install-date").and_then(|d| parse_rfc3339(&d));
            u.description = str_of(s, "summary").or_else(|| str_of(s, "title"));
            u.is_runtime = kind != "app";
            u.locations = vec![PathBuf::from("/snap").join(&name).join("current")];
            let data = sys.home.join("snap").join(&name);
            if data.is_dir() {
                u.locations.push(data);
            }
            let files: Vec<PathBuf> = s
                .get("apps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|a| a.get("desktop-file").and_then(Value::as_str).map(PathBuf::from))
                .collect();
            u.desktop_entries =
                files.iter().filter_map(|p| desktop.effective_for_path(&sys.path(p)).cloned()).collect();
            Some(u)
        })
        .collect()
}

pub fn scan(sys: &SysRoot, desktop: &DesktopIndex) -> Vec<Installation> {
    if !sys.is_host() || !Path::new(SOCKET).exists() {
        return Vec::new();
    }
    get("/v2/snaps").map(|body| parse(&body, sys, desktop)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_snaps() {
        let json = r#"{"type":"sync","result":[
          {"name":"firefox","type":"app","version":"131.0","installed-size":250000000,
           "install-date":"2024-05-01T10:00:00.5+03:00","publisher":{"display-name":"Mozilla"},
           "channel":"latest/stable","summary":"Web browser",
           "apps":[{"name":"firefox","desktop-file":"/var/lib/snapd/desktop/applications/firefox_firefox.desktop"}]},
          {"name":"core22","type":"base","version":"2024"}]}"#;
        let dir = std::env::temp_dir();
        let units = parse(json, &SysRoot::fixture(&dir), &DesktopIndex::default());
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].size, Some(250_000_000));
        assert_eq!(units[0].installed_at, parse_rfc3339("2024-05-01T07:00:00Z"));
        assert!(!units[0].is_runtime);
        assert!(units[1].is_runtime);
    }
}
