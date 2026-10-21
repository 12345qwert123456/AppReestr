//! Running a removal job in a worker thread: privileged steps through pkexec and the helper,
//! user-level steps directly, files to the trash.
//!
//! Tiling window managers often run without a polkit authentication agent, and pkexec then
//! fails. In that case we start polkit's own text agent (`pkttyagent`) for this process only,
//! on a pseudo-terminal, and show its password prompt in our window. The password is passed
//! straight to the agent and not kept.
//!
//! When the whole application runs as root (see `elevate.rs`) the helper is started directly:
//! there is nothing to ask. Steps that belong to the user (their Flatpak installation, their
//! trash) are then run with the user's identity so that no root-owned files appear in their home.

use crate::elevate;
use appreestr_core::i18n::{tr, trf};
use appreestr_core::sysroot::invoking_user;
use nix::pty::openpty;
use relm4::gtk::gio::{self, prelude::FileExt};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Step {
    Pacman {
        pkgs: Vec<String>,
        fingerprint: String,
        nosave: bool,
    },
    /// `pacman -Rdd`: the Danger Zone. Ignores dependencies and protection.
    PacmanForce {
        pkgs: Vec<String>,
        fingerprint: String,
        recursive: bool,
        nosave: bool,
    },
    FlatpakUser {
        flatpak_ref: String,
    },
    FlatpakSystem {
        flatpak_ref: String,
    },
    Snap {
        name: String,
    },
    RootPath {
        path: PathBuf,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    /// Runtimes no application uses any more (`flatpak uninstall --unused`).
    FlatpakUnused {
        system: bool,
    },
}

impl Step {
    fn privileged(&self) -> bool {
        !matches!(self, Step::FlatpakUser { .. } | Step::Trash { .. } | Step::FlatpakUnused { system: false })
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub title: String,
    pub steps: Vec<Step>,
    /// No polkit agent is running in the session: provide one.
    pub need_agent: bool,
}

pub enum Event {
    Status(String),
    Log(String),
    Password { message: String, reply: mpsc::Sender<Option<String>> },
    Done(Result<(), String>),
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Status(s) => write!(f, "Status({s})"),
            Event::Log(s) => write!(f, "Log({s})"),
            Event::Password { .. } => write!(f, "Password"),
            Event::Done(r) => write!(f, "Done({r:?})"),
        }
    }
}

pub fn helper_path() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        if let Ok(p) = std::env::var("APPREESTR_HELPER") {
            return PathBuf::from(p);
        }
        let installed = PathBuf::from("/usr/lib/appreestr/appreestr-helper");
        if !installed.exists()
            && let Some(dev) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("appreestr-helper")))
            && dev.exists()
        {
            return dev;
        }
    }
    PathBuf::from(option_env!("APPREESTR_HELPER_PATH").unwrap_or("/usr/lib/appreestr/appreestr-helper"))
}

/// Run `job`, reporting progress through `emit`. Blocks until done.
pub fn run(job: Job, emit: impl Fn(Event) + Send + Sync + Clone + 'static) {
    let mut agent: Option<Agent> = None;
    let mut result = Ok(());
    // As root there is nobody to ask.
    let need_agent = job.need_agent && !elevate::is_root();
    for step in &job.steps {
        if step.privileged() && need_agent && agent.is_none() {
            agent = Agent::start(emit.clone());
        }
        result = run_step(step, &emit);
        // No agent in the session after all: provide ours and try once more.
        if matches!(&result, Err(StepError::NoAgent)) && agent.is_none() {
            emit(Event::Log(tr("No authentication agent is running; asking for the password here.")));
            agent = Agent::start(emit.clone());
            result = run_step(step, &emit);
        }
        if result.is_err() {
            break;
        }
    }
    drop(agent);
    emit(Event::Done(result.map_err(|e| match e {
        StepError::NoAgent => tr("No authentication agent is available to ask for the password."),
        StepError::Failed(m) => m,
    })));
}

enum StepError {
    NoAgent,
    Failed(String),
}

impl From<String> for StepError {
    fn from(s: String) -> Self {
        StepError::Failed(s)
    }
}

fn run_step(step: &Step, emit: &(impl Fn(Event) + Clone + Send + 'static)) -> Result<(), StepError> {
    let helper = helper_path();
    let helper = helper.to_string_lossy();
    match step {
        Step::Pacman { pkgs, fingerprint, nosave } => {
            emit(Event::Status(tr("Removing packages…")));
            let mut args = vec!["remove-packages".to_string(), "--recursive".into()];
            if *nosave {
                args.push("--nosave".into());
            }
            args.extend(["--expect".into(), fingerprint.clone(), "--".into()]);
            args.extend(pkgs.iter().cloned());
            privileged(&helper, &args, emit)
        }
        Step::PacmanForce { pkgs, fingerprint, recursive, nosave } => {
            emit(Event::Status(tr("Removing packages…")));
            let mut args = vec!["force-remove-packages".to_string()];
            if *recursive {
                args.push("--recursive".into());
            }
            if *nosave {
                args.push("--nosave".into());
            }
            args.extend(["--expect".into(), fingerprint.clone(), "--".into()]);
            args.extend(pkgs.iter().cloned());
            privileged(&helper, &args, emit)
        }
        Step::FlatpakSystem { flatpak_ref } => {
            emit(Event::Status(tr("Removing the Flatpak application…")));
            privileged(&helper, &["remove-flatpak".into(), "--".into(), flatpak_ref.clone()], emit)
        }
        Step::Snap { name } => {
            emit(Event::Status(tr("Removing the Snap package…")));
            privileged(&helper, &["remove-snap".into(), "--".into(), name.clone()], emit)
        }
        Step::RootPath { path } => {
            emit(Event::Status(tr("Removing files…")));
            privileged(&helper, &["remove-path".into(), "--".into(), path.to_string_lossy().into_owned()], emit)
        }
        Step::FlatpakUser { flatpak_ref } => {
            emit(Event::Status(tr("Removing the Flatpak application…")));
            let mut cmd = user_command("flatpak");
            cmd.args(["uninstall", "--user", "--noninteractive", "-y", flatpak_ref]);
            let code = stream(cmd, emit)?.0;
            if code == 0 { Ok(()) } else { Err(trf("flatpak exited with code {}", &[&code.to_string()]).into()) }
        }
        Step::FlatpakUnused { system: true } => {
            emit(Event::Status(tr("Removing unused Flatpak runtimes…")));
            privileged(&helper, &["remove-flatpak-unused".into(), "--".into()], emit)
        }
        Step::FlatpakUnused { system: false } => {
            emit(Event::Status(tr("Removing unused Flatpak runtimes…")));
            let mut cmd = user_command("flatpak");
            cmd.args(["uninstall", "--user", "--unused", "--noninteractive", "-y"]);
            let code = stream(cmd, emit)?.0;
            if code == 0 { Ok(()) } else { Err(trf("flatpak exited with code {}", &[&code.to_string()]).into()) }
        }
        Step::Trash { paths } => {
            emit(Event::Status(tr("Moving files to the trash…")));
            for p in paths {
                if p.symlink_metadata().is_err() {
                    continue;
                }
                let moved = if invoking_user().is_some() {
                    // Root would put it in its own trash, or leave root-owned files in the user's.
                    let mut cmd = user_command("gio");
                    cmd.args(["trash", "--"]).arg(p);
                    match stream(cmd, emit) {
                        Ok((0, _)) => Ok(()),
                        Ok((_, errors)) => Err(errors.join(" ")),
                        Err(e) => Err(e),
                    }
                } else {
                    gio::File::for_path(p).trash(None::<&gio::Cancellable>).map_err(|e| e.to_string())
                };
                match moved {
                    Ok(()) => emit(Event::Log(trf("Moved to the trash: {}", &[&p.display().to_string()]))),
                    Err(e) => {
                        return Err(trf("Cannot move {} to the trash: {}", &[&p.display().to_string(), &e]).into());
                    }
                }
            }
            Ok(())
        }
    }
}

/// Spawn a command, forwarding stdout and stderr lines; returns the exit code and stderr.
fn stream(mut cmd: Command, emit: &(impl Fn(Event) + Clone + Send + 'static)) -> Result<(i32, Vec<String>), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let err = child.stderr.take().unwrap();
    let e2 = emit.clone();
    let t = std::thread::spawn(move || {
        let mut errors = Vec::new();
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            errors.push(line.clone());
            e2(Event::Log(line));
        }
        errors
    });
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        emit(Event::Log(line));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let errors = t.join().unwrap_or_default();
    Ok((status.code().unwrap_or(-1), errors))
}

/// A command that works on the user's own files. In an administrator session it runs with the
/// user's identity and environment; otherwise it is an ordinary command.
fn user_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(u) = invoking_user() {
        let run = format!("/run/user/{}", u.uid);
        cmd.env_clear()
            .env("HOME", &u.home)
            .env("USER", &u.name)
            .env("LOGNAME", &u.name)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("XDG_RUNTIME_DIR", &run)
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={run}/bus"))
            .uid(u.uid)
            .gid(u.gid);
    }
    cmd
}

fn privileged(
    helper: &str,
    args: &[String],
    emit: &(impl Fn(Event) + Clone + Send + 'static),
) -> Result<(), StepError> {
    let mut cmd = if elevate::is_root() {
        // Already administrator. The helper gets a plain environment, as it would from pkexec.
        let mut c = Command::new(helper);
        c.env_clear().env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin").env("HOME", "/root");
        c
    } else {
        let mut c = Command::new("pkexec");
        c.arg(helper);
        c
    };
    cmd.args(args);
    let (code, errors) = stream(cmd, emit)?;
    let msg = match code {
        0 => return Ok(()),
        127 if errors.iter().any(|l| l.contains("No authentication agent found")) => return Err(StepError::NoAgent),
        126 => tr("Authentication was cancelled."),
        127 => tr("Authentication failed: wrong password, or no authentication agent is available."),
        10 => tr("Internal error: the request was rejected as invalid."),
        12 => tr("It is no longer installed."),
        13 => tr("It is protected: the system or the desktop needs it."),
        14 => tr("The system changed since you confirmed. Check again and repeat."),
        15 => tr("Another installation or update is running. Wait for it to finish and try again."),
        16 => tr("The package manager reported an error. See the details."),
        18 => tr("These files cannot be removed from here."),
        c => trf("Unexpected error (code {})", &[&c.to_string()]),
    };
    Err(StepError::Failed(msg))
}

/// Start a privileged copy of the application through pkexec and wait until its window is up
/// (it prints `READY`). Reports `Done(Ok)` then; the caller closes the original window.
pub fn launch_elevated(emit: impl Fn(Event) + Send + Sync + Clone + 'static) {
    let mut agent: Option<Agent> = None;
    let mut result = elevated_once(&emit);
    if matches!(&result, Err(StepError::NoAgent)) {
        emit(Event::Log(tr("No authentication agent is running; asking for the password here.")));
        agent = Agent::start(emit.clone());
        result = elevated_once(&emit);
    }
    drop(agent);
    emit(Event::Done(result.map_err(|e| match e {
        StepError::NoAgent => tr("No authentication agent is available to ask for the password."),
        StepError::Failed(m) => m,
    })));
}

fn elevated_once(emit: &(impl Fn(Event) + Clone + Send + 'static)) -> Result<(), StepError> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // After an upgrade the running binary is reported as "… (deleted)".
    let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    let mut cmd = Command::new("pkexec");
    cmd.arg(&exe).args(elevate::relaunch_args());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let err = child.stderr.take().unwrap();
    let e2 = emit.clone();
    let errors = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            lines.push(line.clone());
            e2(Event::Log(line));
        }
        lines
    });
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if line.trim() == "READY" {
            // From here the privileged copy runs on its own; do not wait for it.
            return Ok(());
        }
        emit(Event::Log(line));
    }
    let code = child.wait().map_err(|e| e.to_string())?.code().unwrap_or(-1);
    let errors = errors.join().unwrap_or_default();
    Err(match code {
        126 => StepError::Failed(tr("Authentication was cancelled.")),
        127 if errors.iter().any(|l| l.contains("No authentication agent found")) => StepError::NoAgent,
        127 => StepError::Failed(tr("Authentication failed: wrong password, or no authentication agent is available.")),
        c => StepError::Failed(trf("The administrator session did not start (code {})", &[&c.to_string()])),
    })
}

/// polkit's text agent running on a pseudo-terminal, registered for this process only.
/// Started only when the session has no agent of its own (pkexec reports none).
struct Agent {
    child: Child,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn process_start_time() -> Option<String> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the command name (which may contain spaces) start after ") ".
    let rest = stat.rsplit_once(") ")?.1;
    rest.split_whitespace().nth(19).map(String::from)
}

impl Agent {
    fn start(emit: impl Fn(Event) + Send + Sync + Clone + 'static) -> Option<Agent> {
        let pty = openpty(None, None).ok()?;
        let (notify_r, notify_w) = nix::unistd::pipe().ok()?;
        let subject = match process_start_time() {
            Some(st) => format!("{},{}", std::process::id(), st),
            None => std::process::id().to_string(),
        };
        let slave: OwnedFd = pty.slave;
        let slave_raw = slave.as_raw_fd();
        let notify_raw = notify_w.as_raw_fd();
        let mut cmd = Command::new("pkttyagent");
        cmd.args(["--process", &subject, "--notify-fd", "3", "--fallback"])
            .env("LC_ALL", "C")
            .stdin(Stdio::from(slave.try_clone().ok()?))
            .stdout(Stdio::from(slave.try_clone().ok()?))
            .stderr(Stdio::from(slave.try_clone().ok()?));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                nix::unistd::setsid().map_err(std::io::Error::from)?;
                if nix::libc::ioctl(slave_raw, nix::libc::TIOCSCTTY as _, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if nix::libc::dup2(notify_raw, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("cannot start pkttyagent: {e}");
                return None;
            }
        };
        drop(slave);
        drop(notify_w);

        // Wait until the agent registered (it closes the notify fd) or gave up.
        let mut notify = std::fs::File::from(notify_r);
        let deadline = Instant::now() + Duration::from_secs(3);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut b = [0u8; 1];
            let _ = notify.read(&mut b);
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));

        // Read the terminal; answer password prompts through the UI.
        let master = std::fs::File::from(pty.master);
        let mut writer = master.try_clone().ok()?;
        let mut reader = master;
        std::thread::spawn(move || {
            let mut text = String::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                text.push_str(&String::from_utf8_lossy(&buf[..n]));
                if text.contains("AUTHENTICATION FAILED") {
                    emit(Event::Log(tr("Wrong password.")));
                    text.clear();
                    continue;
                }
                // The prompt is the unfinished last line ending with ':' ("Password:", "Пароль:"…).
                let last = text.rsplit('\n').next().unwrap_or("").trim_end();
                if last.to_lowercase().contains("choose identity") {
                    let _ = writer.write_all(b"1\n");
                    text.clear();
                } else if last.ends_with(':') {
                    let who = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Authenticating as: "))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let message = if who.is_empty() {
                        tr("Enter the administrator password to continue.")
                    } else {
                        trf("Enter the password of {} to continue.", &[&who])
                    };
                    let (reply_tx, reply_rx) = mpsc::channel();
                    emit(Event::Password { message, reply: reply_tx });
                    match reply_rx.recv() {
                        Ok(Some(pw)) => {
                            let mut bytes = pw.into_bytes();
                            bytes.push(b'\n');
                            let _ = writer.write_all(&bytes);
                            let _ = writer.flush();
                            // Do not leave the password in memory longer than needed.
                            bytes.iter_mut().for_each(|b| *b = 0);
                        }
                        _ => {
                            // Cancel: interrupt the agent; pkexec then reports "not authorized".
                            let _ = writer.write_all(b"\x03");
                            break;
                        }
                    }
                    text.clear();
                }
            }
        });
        Some(Agent { child })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only meaningful in a session WITHOUT any polkit agent (a bare tiling WM): otherwise the
    /// session's agent shows its own password dialog. Starts our fallback agent, asks pkexec
    /// to run `true`, and cancels the prompt: nothing runs as root.
    /// Run with `cargo test -p appreestr -- --ignored fallback_agent`.
    #[test]
    #[ignore]
    fn fallback_agent_prompts_and_cancels() {
        let (tx, rx) = mpsc::channel::<Event>();
        let emit = move |e: Event| {
            let _ = tx.send(e);
        };
        let agent = Agent::start(emit.clone()).expect("pkttyagent started");
        let mut child = Command::new("pkexec").arg("/usr/bin/true").stdin(Stdio::null()).spawn().unwrap();
        let mut prompted = false;
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(15)) {
            if let Event::Password { message, reply } = e {
                eprintln!("prompt: {message}");
                prompted = true;
                reply.send(None).unwrap();
                break;
            }
        }
        let status = child.wait().unwrap();
        drop(agent);
        assert!(prompted, "the agent did not ask for a password");
        assert!(!status.success(), "pkexec must not succeed after a cancelled prompt");
        eprintln!("pkexec exit: {status}");
    }
}
