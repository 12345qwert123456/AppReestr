//! "Restart as Administrator": the whole application runs as root, so removing does not ask
//! for a password. The helper still enforces every protection rule.
//!
//! The privileged copy is started with `pkexec appreestr NAME=VALUE… --notify-ready`. pkexec
//! drops the environment, so the display and session variables travel as arguments (only the
//! ones listed in `PASS`). The copy says `READY` on its stdout when its window is up, which is
//! when the original closes. Who started it comes from the `PKEXEC_UID` that pkexec sets: the
//! user's apps and settings are read from their home, not from root's.

use std::io::Write;
use std::path::Path;

/// Variables the privileged copy needs to reach the user's display and speak their language.
const PASS: &[&str] = &[
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
    "GDK_BACKEND",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_MESSAGES",
];

pub const NOTIFY_READY: &str = "--notify-ready";

pub fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

/// Arguments that recreate the session in a privileged copy.
pub fn relaunch_args() -> Vec<String> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/'));
    let mut args: Vec<String> = Vec::new();
    for name in PASS {
        let Some(mut value) = std::env::var(name).ok().filter(|v| !v.is_empty()) else { continue };
        // The runtime directory is not passed on (root should not write into it), so the
        // Wayland socket and the session bus are named by their full paths instead.
        if *name == "WAYLAND_DISPLAY"
            && !value.starts_with('/')
            && let Some(dir) = &runtime
        {
            value = Path::new(dir).join(&value).to_string_lossy().into_owned();
        }
        args.push(format!("{name}={value}"));
    }
    if !args.iter().any(|a| a.starts_with("DBUS_SESSION_BUS_ADDRESS="))
        && let Some(dir) = &runtime
    {
        args.push(format!("DBUS_SESSION_BUS_ADDRESS=unix:path={dir}/bus"));
    }
    args.push(NOTIFY_READY.into());
    args
}

/// In the privileged copy: take over the variables passed as arguments. Must run before any
/// other thread exists. Returns whether to say `READY` once the window is shown.
pub fn adopt_args(args: &[String]) -> bool {
    if !is_root() {
        return false;
    }
    let mut notify = false;
    for arg in args {
        if arg == NOTIFY_READY {
            notify = true;
        } else if let Some((name, value)) = arg.split_once('=')
            && PASS.contains(&name)
        {
            // SAFETY: called from `main` before GTK or any thread is started.
            unsafe { std::env::set_var(name, value) };
        }
    }
    // dconf would write root's settings into the user's runtime directory.
    // SAFETY: as above.
    unsafe { std::env::set_var("GSETTINGS_BACKEND", "memory") };
    notify
}

/// Tell the process that started us that the window is up, then stop writing to the pipe.
pub fn notify_ready() {
    let mut out = std::io::stdout();
    let _ = out.write_all(b"READY\n");
    let _ = out.flush();
    // The reader goes away when the original closes; later output must not hit a broken pipe.
    if let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
        let _ = nix::unistd::dup2_stdout(&null);
        let _ = nix::unistd::dup2_stderr(&null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_socket_becomes_a_full_path() {
        // SAFETY: tests of this module are the only ones touching these variables.
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::remove_var("DBUS_SESSION_BUS_ADDRESS");
        }
        let args = relaunch_args();
        assert!(args.contains(&"WAYLAND_DISPLAY=/run/user/1000/wayland-1".to_string()));
        assert!(args.contains(&"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus".to_string()));
        assert!(args.contains(&NOTIFY_READY.to_string()));
        assert!(!args.iter().any(|a| a.starts_with("XDG_RUNTIME_DIR")));
    }

    #[test]
    fn not_root_adopts_nothing() {
        if !is_root() {
            assert!(!adopt_args(&["LANG=x".into(), NOTIFY_READY.into()]));
        }
    }
}
