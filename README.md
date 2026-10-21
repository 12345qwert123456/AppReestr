# AppReestr

**English** | [Русский](README.ru.md)

[![CI](https://github.com/12345qwert123456/AppReestr/actions/workflows/ci.yml/badge.svg)](https://github.com/12345qwert123456/AppReestr/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

![AppReestr main window](docs/screenshots/main.png)

An installed-applications registry for Arch Linux and derivatives (CachyOS, Manjaro, EndeavourOS).
Like "Installed apps" in Windows: one list of programs from every source, a clear view of where
each one came from, and a safe way to remove it.

## Features

- **One list** from every source: official Arch repositories, distribution repositories (CachyOS,
  EndeavourOS, Manjaro), third-party repositories, AUR, Flatpak, AppImage, Snap and manual
  installs (`/opt`, `/usr/local/bin`, archives unpacked in the home folder).
- **Applications only.** Libraries, fonts, kernels, drivers and services are hidden. Show them with
  the "Show System Components" toggle (Ctrl+H); console utilities have their own toggle (Ctrl+J).
- **One program, one row**, even when it is installed in several ways (for example from a
  repository and from Flatpak).
- **App details**:
  - where it was installed from, when and with which command (read from `pacman.log`);
  - where its files are and what came along with it;
  - why it is in the list.
- **Safe removal** with the standard tools: `pacman -Rs`, `flatpak uninstall`, `snap remove`.
  - You see what goes away with the program before anything is removed.
  - The kernel, the base system, the bootloader, the running desktop and the login screen are protected.
  - Cascade removal (`-Rc`) is never used.
- **Bulk removal.** The check-mark button in the header bar turns on selection mode: tick several
  programs and press "Uninstall…". Programs that cannot be removed (protected or needed by others)
  are skipped with an explanation; the rest go in one operation.
- **Administrator mode.** The lock button in the header bar restarts AppReestr as root: the password
  is asked once, and removal does not ask for it again. System protection stays on.
- **Danger Zone** (Preferences → "Danger Zone" and the package details): force-remove any package,
  like `pacman -Rdd`. Dependencies and protection are ignored; what will break is shown first, and
  you confirm by typing the package name.
- **Leftovers** (settings, data and cache in the home folder) can be sent to the trash, with a
  preview.
- **Manual marks**: "hide from the list" and "show as an application" are remembered.
- **Unneeded components** after removal: AppReestr offers to clean up what nothing needs any more.
- **No telemetry, no network by default.** Browsing needs no administrator rights. The optional update
  check (Preferences, off by default) makes one request a day to github.com; it only tells you about a new
  version.
- Fully keyboard-driven, follows the system light/dark theme, available in English (default) and Russian. The
  language is chosen in Preferences and applies after a restart.

## Install

From the AUR (releases, or `appreestr-git` for the main branch):

```bash
yay -S appreestr        # or paru -S appreestr
```

Every release also has a ready package, `appreestr-X.Y.Z-1-x86_64.pkg.tar.zst`, on the
[Releases](https://github.com/12345qwert123456/AppReestr/releases) page (`sudo pacman -U <file>`). It is
linked against the libalpm of the Arch it was built on, so prefer the AUR package on other systems.

### Updates

The program is updated by your package manager (`yay -Syu`, `paru`), like any other package; it never
installs anything by itself. Turn on "Check for Updates" in Preferences to be told when a new release
appears. AppReestr targets Arch and derivatives (it is built on libalpm), so there are no packages for
Debian and other distributions.

### Build from source

Dependencies: `rust`, `gtk4`, `libadwaita`, `pacman` (libalpm), `polkit`, `gettext`.

```bash
git clone https://github.com/12345qwert123456/AppReestr
cd AppReestr
make
sudo make install
```

Or as a package built from the working tree:

```bash
cd packaging && makepkg -si
```

To run from the source tree without installing: `cargo run -p appreestr`. A debug build removes
packages through `target/debug/appreestr-helper`; pkexec asks for the password with a generic
message because the polkit policy is not installed.

## Tiling window managers (Hyprland, Sway, i3)

- **Administrator password.** The polkit agent of your session asks for it (hyprpolkitagent,
  polkit-gnome, a shell-integrated agent such as Noctalia, and so on). If there is no agent in the
  session and `pkexec` says so, AppReestr starts the standard `pkttyagent` for itself only and shows
  the password dialog on its own. The password goes straight to polkit and is never stored.
- **Light and dark theme** come from the desktop portal: you need `xdg-desktop-portal-gtk` (or
  `-gnome`/`-kde`). You can also pick the theme in Preferences (Ctrl+,).

## Administrator mode

"Restart as Administrator…" runs `pkexec appreestr`; the window reopens and the old one closes. The
app runs as root, but the list and settings are taken from the home folder of your user
(`PKEXEC_UID`), files there are written as you, and the trash and user Flatpak are handled as you.
The `appreestr-helper` still validates everything itself and is started directly, without pkexec.
The polkit policy `…run-as-admin` is installed with the program; from the source tree pkexec asks
for the password with a generic message.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| typing / Ctrl+F | search |
| ↑ ↓, Enter | select, app details |
| Delete | uninstall the selection (in selection mode: all ticked) |
| Esc, Alt+← | back / leave selection mode / clear search |
| Alt+1…8 | filter by source |
| Ctrl+H / Ctrl+J | system / console utilities |
| F5, Ctrl+R | refresh the list |
| Ctrl+, | preferences |
| Ctrl+? | all shortcuts |

## How it works

| Part | What it does |
|---|---|
| `crates/core` | No GUI: sources (`sources/`), classification (`classify.rs`), merging into apps (`merge.rs`), protection (`protect.rs`), removal plan through libalpm (`plan.rs`), leftovers (`leftovers.rs`) |
| `crates/app` | GTK4 + libadwaita interface (Relm4) |
| `crates/helper` | `appreestr-helper`, the only part that runs as root (through pkexec). It re-checks names, plan and protection, and removes with the standard tools |
| `crates/dump` | Developer tool: `appreestr-dump --all` shows what is listed and why; `--plan PKG`, `--leftovers APP-ID`, `--timings` |
| `data/rules/*.toml` | Classification, repository and protection rules, editable without recompiling; put your own additions in `~/.config/appreestr/rules.toml` |

Settings and manual marks live in `~/.config/appreestr/`, the cache in `~/.cache/appreestr/`.

## Development

```bash
make check          # rustfmt, clippy, tests, .desktop and metainfo validation
cargo run -p appreestr-dump -- --timings --all
```

An end-to-end removal test on harmless packages is in `tests/pkg/PKGBUILD`. See
[CONTRIBUTING.md](CONTRIBUTING.md) before sending a pull request, and [SECURITY.md](SECURITY.md)
to report a vulnerability.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
