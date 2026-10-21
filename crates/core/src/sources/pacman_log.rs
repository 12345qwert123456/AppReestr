//! `/var/log/pacman.log`: when a package was first installed and by which command.
//!
//! `%INSTALLDATE%` in the local database changes on every upgrade, so the log is the only place
//! that knows the real installation date.

use std::collections::HashMap;

/// Packages first installed within this time after the first log line count as part of the OS
/// installation (installers run several transactions in a row).
const INSTALL_SESSION_SECS: i64 = 3600;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogEntry {
    /// Start of the current installation (the last `installed` after any `removed`).
    pub first_installed: Option<i64>,
    pub last_changed: Option<i64>,
    /// The `pacman` command line of the transaction that installed it.
    pub command: Option<String>,
    /// Installed by the OS installer.
    pub with_system: bool,
}

#[derive(Debug, Default)]
pub struct PacmanLog {
    pub entries: HashMap<String, LogEntry>,
    pub first_ts: Option<i64>,
}

impl PacmanLog {
    pub fn parse(text: &str) -> Self {
        let mut log = PacmanLog::default();
        let mut cmd: Option<String> = None;
        let mut cmd_is_installer = false;
        for line in text.lines() {
            let Some(rest) = line.strip_prefix('[') else { continue };
            let Some((ts, rest)) = rest.split_once("] ") else { continue };
            let Some(ts) = parse_timestamp(ts) else { continue };
            let first = *log.first_ts.get_or_insert(ts);
            let (source, msg) = match rest.split_once("] ") {
                Some((s, m)) if s.starts_with('[') => (&s[1..], m),
                _ => ("", rest),
            };
            match source {
                "PACMAN" => {
                    if let Some(c) = msg.strip_prefix("Running '").and_then(|m| m.strip_suffix('\'')) {
                        cmd_is_installer = is_installer_command(c);
                        cmd = Some(c.to_string());
                    }
                }
                "ALPM" | "" => {
                    let Some((action, rest)) = msg.split_once(' ') else { continue };
                    let name = rest.split(' ').next().unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    match action {
                        "installed" => {
                            let e = log.entries.entry(name.to_string()).or_default();
                            e.first_installed = Some(ts);
                            e.last_changed = Some(ts);
                            e.command = cmd.clone();
                            e.with_system = cmd_is_installer || ts - first <= INSTALL_SESSION_SECS;
                        }
                        "removed" => {
                            log.entries.remove(name);
                        }
                        "upgraded" | "downgraded" | "reinstalled" => {
                            log.entries.entry(name.to_string()).or_default().last_changed = Some(ts);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        log
    }
}

fn is_installer_command(cmd: &str) -> bool {
    let args = cmd.split_whitespace();
    for a in args {
        if a == "--sysroot" || a == "--root" || a == "-r" {
            return true;
        }
        if a.starts_with("--sysroot=") || a.starts_with("--root=") {
            return true;
        }
    }
    false
}

/// How the package was installed, derived from the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallMethod {
    Installer,
    /// `pacman -S`; `requested` is false when it came in as a dependency of other targets.
    Repo {
        requested: bool,
        targets: Vec<String>,
    },
    /// `pacman -U` of a locally built file; `helper` is paru/yay/… when recognisable.
    LocalFile {
        helper: Option<String>,
    },
    Other,
}

pub fn install_method(command: &str, pkg: &str, with_system: bool) -> InstallMethod {
    if with_system && is_installer_command(command) {
        return InstallMethod::Installer;
    }
    let args: Vec<&str> = command.split_whitespace().skip(1).collect();
    let is_upgrade =
        args.iter().any(|a| *a == "--upgrade" || (a.starts_with('-') && !a.starts_with("--") && a.contains('U')));
    let targets: Vec<String> = args
        .iter()
        .skip_while(|a| **a != "--")
        .skip(1)
        .chain(args.iter().filter(|a| !a.starts_with('-')))
        .map(|a| a.to_string())
        .collect();
    let mut targets_dedup: Vec<String> = Vec::new();
    for t in targets {
        if !targets_dedup.contains(&t) {
            targets_dedup.push(t);
        }
    }
    if is_upgrade {
        let helper = ["paru", "yay", "pikaur", "aurutils", "trizen", "pamac", "aura", "pakku"]
            .iter()
            .find(|h| command.contains(&format!("/{h}/")) || command.contains(&format!("/.cache/{h}")))
            .map(|h| h.to_string());
        return InstallMethod::LocalFile { helper };
    }
    let is_sync =
        args.iter().any(|a| *a == "--sync" || (a.starts_with('-') && !a.starts_with("--") && a.contains('S')));
    if is_sync {
        let short: Vec<String> = targets_dedup.iter().map(|t| t.rsplit('/').next().unwrap_or(t).to_string()).collect();
        let requested = short.iter().any(|t| t == pkg);
        return InstallMethod::Repo { requested, targets: short };
    }
    InstallMethod::Other
}

/// `2026-10-02T16:22:51+0300` or the pre-2019 `2019-01-01 12:34` (treated as UTC).
pub fn parse_timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 16 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi) = (num(11..13)?, num(14..16)?);
    let mut sec = 0;
    let mut rest = &s[16..];
    if let Some(r) = rest.strip_prefix(':') {
        sec = r.get(0..2)?.parse().ok()?;
        rest = &r[2..];
    }
    let mut offset = 0;
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 4 {
            let oh: i64 = digits[0..2].parse().ok()?;
            let om: i64 = digits[2..4].parse().ok()?;
            offset = (oh * 3600 + om * 60) * if sign == '-' { -1 } else { 1 };
        }
    } else if rest.starts_with('Z') {
        offset = 0;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec - offset)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
[2026-10-02T16:22:51+0300] [PACMAN] Running 'pacman --sysroot /tmp/calamares-root -Sy --noconfirm base nano'
[2026-10-02T16:24:52+0300] [ALPM] transaction started
[2026-10-02T16:24:53+0300] [ALPM] installed nano (8.0-1)
[2026-10-05T10:00:00+0300] [PACMAN] Running 'pacman -S firefox'
[2026-10-05T10:00:05+0300] [ALPM] installed nss (3.1-1)
[2026-10-05T10:00:06+0300] [ALPM] installed firefox (131.0-1)
[2026-10-06T10:00:00+0300] [ALPM] upgraded firefox (131.0-1 -> 132.0-1)
[2026-10-07T10:00:00+0300] [PACMAN] Running 'pacman --upgrade --noconfirm -- /home/u/.cache/paru/clone/foo/foo-1-1-any.pkg.tar.zst'
[2026-10-07T10:00:01+0300] [ALPM] installed foo (1-1)
[2026-10-08T10:00:00+0300] [ALPM] removed foo (1-1)
";

    #[test]
    fn tracks_history() {
        let log = PacmanLog::parse(LOG);
        let nano = &log.entries["nano"];
        assert!(nano.with_system);
        let ff = &log.entries["firefox"];
        assert_eq!(ff.first_installed, parse_timestamp("2026-10-05T10:00:06+0300"));
        assert_eq!(ff.last_changed, parse_timestamp("2026-10-06T10:00:00+0300"));
        assert!(!ff.with_system);
        assert!(!log.entries.contains_key("foo"));
        assert_eq!(
            install_method(ff.command.as_deref().unwrap(), "firefox", false),
            InstallMethod::Repo { requested: true, targets: vec!["firefox".into()] }
        );
        assert_eq!(
            install_method(log.entries["nss"].command.as_deref().unwrap(), "nss", false),
            InstallMethod::Repo { requested: false, targets: vec!["firefox".into()] }
        );
        assert_eq!(install_method(nano.command.as_deref().unwrap(), "nano", true), InstallMethod::Installer);
    }

    #[test]
    fn local_builds() {
        let cmd = "pacman --upgrade --noconfirm -- /home/u/.cache/paru/clone/foo/foo-1-1-any.pkg.tar.zst";
        assert_eq!(install_method(cmd, "foo", false), InstallMethod::LocalFile { helper: Some("paru".into()) });
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00+0000"), Some(0));
        assert_eq!(parse_timestamp("1970-01-01T03:00:00+0300"), Some(0));
        assert_eq!(parse_timestamp("2000-03-01 00:00"), Some(951868800));
    }
}
