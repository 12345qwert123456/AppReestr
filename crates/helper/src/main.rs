//! appreestr-helper: the only part of AppReestr that runs as root (through pkexec).
//!
//! It trusts nothing from the caller: names are validated, the removal is planned again and
//! must match the fingerprint the user confirmed, protected packages are refused, and the
//! actual work is done by the standard tools (pacman, flatpak, snap) without a shell.
//!
//! `force-remove-packages` is the one deliberate exception: it is the "Danger Zone" of the GUI
//! and removes exactly the packages the user confirmed (`pacman -Rdd`), protected or not.
//!
//! Usage:
//!   appreestr-helper remove-packages [--recursive] [--nosave] --expect FINGERPRINT -- PKG...
//!   appreestr-helper force-remove-packages [--recursive] [--nosave] --expect FINGERPRINT -- PKG...
//!   appreestr-helper remove-flatpak -- REF
//!   appreestr-helper remove-flatpak-unused --
//!   appreestr-helper remove-snap -- NAME
//!   appreestr-helper remove-path -- /opt/DIR

use appreestr_core::desktop::DesktopIndex;
use appreestr_core::plan;
use appreestr_core::protect;
use appreestr_core::rules::Rules;
use appreestr_core::sources::pacman;
use appreestr_core::sysroot::SysRoot;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod validate;

/// Exit codes understood by the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Code {
    Ok = 0,
    Usage = 10,
    NotRoot = 11,
    NotInstalled = 12,
    Protected = 13,
    PlanChanged = 14,
    Locked = 15,
    ToolFailed = 16,
    Internal = 17,
    NotAllowed = 18,
}

fn fail(code: Code, msg: impl AsRef<str>) -> ExitCode {
    eprintln!("APPREESTR-ERROR {}: {}", code as u8, msg.as_ref());
    ExitCode::from(code as u8)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        return fail(Code::Usage, "no command");
    };
    if !nix::unistd::geteuid().is_root() {
        return fail(Code::NotRoot, "must run as root through pkexec");
    }
    let Some(sep) = rest.iter().position(|a| a == "--") else {
        return fail(Code::Usage, "missing --");
    };
    let (opts, operands) = (&rest[..sep], &rest[sep + 1..]);
    match cmd.as_str() {
        "remove-packages" => remove_packages(opts, operands, false),
        "force-remove-packages" => remove_packages(opts, operands, true),
        "remove-flatpak" => remove_flatpak(opts, operands),
        "remove-snap" => remove_snap(opts, operands),
        "remove-flatpak-unused" => {
            if !opts.is_empty() || !operands.is_empty() {
                return fail(Code::Usage, "no arguments expected");
            }
            run("/usr/bin/flatpak", &["uninstall", "--system", "--unused", "--noninteractive", "-y"])
        }
        "remove-path" => remove_path(opts, operands),
        _ => fail(Code::Usage, format!("unknown command {cmd}")),
    }
}

fn run(program: &str, args: &[&str]) -> ExitCode {
    // Plain environment: pkexec already cleared it; keep it predictable for the tools.
    let status = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C.UTF-8")
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::from(Code::Ok as u8),
        Ok(s) => fail(Code::ToolFailed, format!("{program} exited with {s}")),
        Err(e) => fail(Code::ToolFailed, format!("cannot run {program}: {e}")),
    }
}

/// Most packages one request may name.
const MAX_PACKAGES: usize = 256;

fn remove_packages(opts: &[String], pkgs: &[String], force: bool) -> ExitCode {
    let mut recursive = false;
    let mut nosave = false;
    let mut expect: Option<&str> = None;
    let mut it = opts.iter();
    while let Some(o) = it.next() {
        match o.as_str() {
            "--recursive" => recursive = true,
            "--nosave" => nosave = true,
            "--expect" => expect = it.next().map(String::as_str),
            other => return fail(Code::Usage, format!("unknown option {other}")),
        }
    }
    let Some(expect) = expect else { return fail(Code::Usage, "missing --expect") };
    if pkgs.is_empty() || pkgs.len() > MAX_PACKAGES {
        return fail(Code::Usage, format!("expected 1..{MAX_PACKAGES} packages"));
    }
    if let Some(bad) = pkgs.iter().find(|p| !validate::package_name(p)) {
        return fail(Code::Usage, format!("invalid package name {bad:?}"));
    }
    if Path::new("/var/lib/pacman/db.lck").exists() {
        return fail(Code::Locked, "the package database is locked by another operation");
    }

    let mut handle = match plan::open_local("/", "/var/lib/pacman") {
        Ok(h) => h,
        Err(e) => return fail(Code::Internal, e.to_string()),
    };
    let planned = if force {
        // The user was shown what this breaks and confirmed it: no protection, no dependency checks.
        plan::plan_force(&mut handle, pkgs, recursive, None)
    } else {
        let sys = SysRoot { home: PathBuf::from("/root"), ..SysRoot::host() };
        // Only the bundled rules: user files are not read as root.
        let rules = Rules::bundled();
        let data = match pacman::scan(&sys, &DesktopIndex::default(), &rules) {
            Ok(d) => d,
            Err(e) => return fail(Code::Internal, format!("{e:#}")),
        };
        let processes = protect::processes(&sys);
        let protection = protect::analyze(&protect::Input {
            sys: &sys,
            rules: &rules,
            owners: &data.owners,
            depends: &data.depends,
            hold_pkgs: &data.hold_pkgs,
            kernels: &data.kernels,
            processes: &processes,
        });
        plan::plan(&mut handle, pkgs, recursive, Some(&protection))
    };
    let p = match planned {
        Ok(p) => p,
        Err(plan::PlanError::NotInstalled(n)) => return fail(Code::NotInstalled, format!("{n} is not installed")),
        Err(e) => return fail(Code::Internal, e.to_string()),
    };
    if !force {
        if let Some((name, _)) = p.blocked.first() {
            return fail(Code::Protected, format!("{name} is protected"));
        }
        if !p.needed_by.is_empty() {
            return fail(Code::Protected, format!("needed by other packages: {:?}", p.needed_by));
        }
    }
    if p.fingerprint != expect {
        return fail(Code::PlanChanged, "the set of packages to remove changed since it was confirmed");
    }
    drop(handle);

    let mut flag = String::from("-R");
    if recursive {
        flag.push('s');
    }
    if nosave {
        flag.push('n');
    }
    if force {
        flag.push_str("dd");
    }
    let mut args: Vec<&str> = vec![&flag, "--noconfirm", "--"];
    args.extend(pkgs.iter().map(String::as_str));
    run("/usr/bin/pacman", &args)
}

fn single<'a>(opts: &[String], operands: &'a [String]) -> Result<&'a str, ExitCode> {
    if !opts.is_empty() {
        return Err(fail(Code::Usage, "no options expected"));
    }
    match operands {
        [one] => Ok(one),
        _ => Err(fail(Code::Usage, "expected exactly one operand")),
    }
}

fn remove_flatpak(opts: &[String], operands: &[String]) -> ExitCode {
    let r = match single(opts, operands) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !validate::flatpak_ref(r) {
        return fail(Code::Usage, format!("invalid flatpak ref {r:?}"));
    }
    run("/usr/bin/flatpak", &["uninstall", "--system", "--noninteractive", "-y", r])
}

fn remove_snap(opts: &[String], operands: &[String]) -> ExitCode {
    let name = match single(opts, operands) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !validate::snap_name(name) {
        return fail(Code::Usage, format!("invalid snap name {name:?}"));
    }
    run("/usr/bin/snap", &["remove", name])
}

fn remove_path(opts: &[String], operands: &[String]) -> ExitCode {
    let raw = match single(opts, operands) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path = Path::new(raw);
    if let Err(why) = validate::removable_path(path) {
        return fail(Code::NotAllowed, why);
    }
    let sys = SysRoot { home: PathBuf::from("/root"), ..SysRoot::host() };
    let data = match pacman::scan(&sys, &DesktopIndex::default(), &Rules::bundled()) {
        Ok(d) => d,
        Err(e) => return fail(Code::Internal, format!("{e:#}")),
    };
    let s = raw.trim_end_matches('/');
    if data.owners.dir_is_owned(s) || data.owners.file_is_owned(s) || data.owners.owner(&format!("{s}/x")).is_some() {
        return fail(Code::NotAllowed, format!("{raw} belongs to a package; remove the package instead"));
    }
    let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    match result {
        Ok(()) => {
            println!("removed {raw}");
            ExitCode::from(Code::Ok as u8)
        }
        Err(e) => fail(Code::ToolFailed, format!("{raw}: {e}")),
    }
}
