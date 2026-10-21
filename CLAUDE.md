# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

AppReestr is a GTK4/libadwaita app for Arch Linux and derivatives. It lists the user's installed *applications* from every source (pacman repos, AUR, Flatpak, AppImage, Snap, manual installs), shows where each came from, and removes them safely. The README and UI are in Russian; code, comments and gettext msgids are in English.

## Commands

```bash
cargo build                                   # debug build of the whole workspace
cargo run -p appreestr                        # run the GUI from the tree
cargo run -p appreestr-dump -- --timings --all  # what is listed and why (App/Console/System + reasons)
cargo run -p appreestr-dump -- --plan PKG...    # removal plan + protection for pacman packages (read-only)
cargo run -p appreestr-dump -- --leftovers APP-ID   # leftovers in $HOME (APP-ID like pacman:firefox)
cargo test --workspace
cargo test -p appreestr-core classify::tests::console_and_system   # single test
make check    # fmt --check, clippy -D warnings, tests, desktop/metainfo validation (= CI)
make build && sudo make install               # release build; PREFIX/DESTDIR/TARGET honoured
make pot      # re-extract strings (tools/extract-strings.py) and msgmerge po/*.po
```

- `rustfmt.toml` sets `max_width = 120`; run `cargo fmt --all` before finishing.
- `plan::tests::plans_on_host_without_root` reads the real `/var/lib/pacman` database (read-only).
- `removal::tests::fallback_agent_prompts_and_cancels` is `#[ignore]`. It is only meaningful in a session with no polkit agent: elsewhere the session's agent pops a password dialog on the user's screen. Do not run `pkexec` probes casually for the same reason.

## Workspace layout

- `crates/core` (`appreestr-core`): all logic. No GTK, so it is testable and shared with the helper.
- `crates/app` (`appreestr`): Relm4 UI. Plain gtk/adw builders inside one `Component` (`window.rs`), not the `view!` macro.
- `crates/helper` (`appreestr-helper`): the only code that runs as root, through `pkexec`.
- `crates/dump`: developer CLI over the same scan. It is not a product feature (a terminal mode is out of scope).
- `data/rules/*.toml`: curated classification, repo-label and protection rules. Embedded with `include_str!`; users can extend them in `~/.config/appreestr/rules.toml`. Tune behaviour here before adding code.

## Architecture (core)

`scan::scan` is the pipeline:

1. **`DesktopIndex::scan`** reads every XDG `applications` dir, plus Flatpak/Snap export dirs added in `SysRoot::host`. User overrides with the same desktop ID win (`effective_for_path`).
2. **Sources run in parallel threads**: `sources/{pacman,flatpak,appimage,snap}.rs`. A missing tool or directory yields no units, not an error.
   - Each source returns `Installation` units. A unit is what one uninstall action removes, keyed by `UnitKey` such as `pacman:firefox` or `flatpak:user:<ref>`.
   - **pacman** uses libalpm through the `alpm` crate.
     - Builds its own reverse-dependency graph and an `OwnerIndex`: exact files in a few dirs, plus directories owned by exactly one package. `filesystem` and `/home` are excluded on purpose.
     - Repo origin comes from sync DBs. AUR evidence comes from helper caches (`~/.cache/paru/clone` …) or `pacman.log`.
     - The real first-install date, the install command and "installed with the OS" come from `pacman_log.rs`, because `%INSTALLDATE%` changes on every upgrade.
   - **flatpak** shells out to `flatpak list`, so it has no libflatpak link dependency. **appimage** reads the embedded SquashFS with `backhand` and never executes the file. **snap** queries snapd's REST API over `/run/snapd.socket`.
3. **`sources/manual.rs`** runs after the others because it needs the pacman `OwnerIndex` and the set of launchers already claimed.
   - Finds unowned `/opt/*`, apps in the home dir (install root via `install_root`), and `/usr/local/bin` files grouped by mtime minute.
   - Recognises Steam/Wine/web-app launchers (`Source::Shortcut`, listed but not removable).
   - User launchers that start a packaged binary are returned as `links` and attached to that package.
4. **`classify::classify`** is a pure function `Installation → (Visibility, Vec<Reason>)` with `Visibility = App | Console | System`.
   - A key rule: a dependency with a launcher is an App only if every package requiring it is a "set" (`classify::is_set`: meta packages, or config packages with no binaries and no launcher, like `cachyos-hypr-noctalia`).
   - Reasons are data and are rendered by `reasons.rs`.
5. **`merge::merge`** turns units into `App` rows with union-find over AppStream ID, desktop ID and WM class (strong keys) and normalised name (weak key).
   - Units of the same source family are never merged.
   - User marks (`overrides.toml`, keyed by unit key) are applied here.

The scan takes about 0.4 s in release. The UI shows `cache.rs`'s snapshot (postcard, versioned `MAGIC`; bump it on incompatible model changes) instantly, then swaps in the fresh scan. `listing.rs` holds search/filter/sort, kept out of the UI so it can be tested.

## Removal safety model

- **Protection.** `protect::analyze` computes blocked packages: the dependency closure of critical roots (rules `critical`, kernels, pacman `HoldPkg`, running compositor/shell/polkit agent from `/proc`, display manager). It also computes warnings: running, desktop component, enabled systemd units.
- **Planning.** `plan::plan` simulates `pacman -R[s]` with libalpm `TransFlag::NO_LOCK`, without root. It returns the removal set, `needed_by` when pacman would refuse, optional users, and a `fingerprint` (FNV over the sorted names). Cascade (`-Rc`) is never used.
- **The helper trusts nothing.** It validates names and refs (`validate.rs`), re-scans, recomputes protection with bundled rules only, re-plans, requires the same fingerprint (`--expect`), then runs `/usr/bin/pacman`, `flatpak` or `snap` with a cleared env.
  - Exit codes 10–18 are the contract with `crates/app/src/removal.rs`.
  - `remove-path` only deletes canonical, unowned direct children of `/opt` and `/usr/local/{bin,lib,share/applications}`.
- **UI flow.** `uninstall.rs` runs plan → confirm dialog → `removal::run` in a worker, which streams `Event`s. Then follow-up offers to clean orphans that are new since the plan, or unused Flatpak runtimes.
- **Files.** User-level files (AppImages, leftovers from `leftovers.rs`) go to the trash via `gio`, never permanent deletion. Root paths go through the helper.
- **polkit.** The session's own agent is used. The fallback (`pkttyagent` on a PTY, password dialog in the app) starts only when pkexec reports "No authentication agent found". The `polkit_agents` rule list (which includes `noctalia`) only feeds protection and the pre-start hint.
- **Helper path.** The policy action `io.github.appreestr.AppReestr.uninstall` is bound to `/usr/lib/appreestr/appreestr-helper`. Debug builds fall back to `target/debug/appreestr-helper`, or `$APPREESTR_HELPER`, when it is not installed. Release builds bake in `APPREESTR_HELPER_PATH` and `APPREESTR_LOCALEDIR` from the Makefile.

## i18n

- Use `appreestr_core::i18n::{tr, trf, trn}` with English literals. `trf` substitutes `{}` in order. `trn` handles plurals; Russian has 3 forms.
- Strings must be literals inside these calls for `tools/extract-strings.py` to find them. xgettext is not used because it chokes on Rust lifetimes.
- After adding strings, run `make pot` and translate the new entries in `po/ru.po`: clear `#, fuzzy` and keep `msgfmt --check` clean.
- `crates/app/build.rs` compiles `.po` into `OUT_DIR` for runs from the tree.
- `format_size` uses the locale's decimal separator.

## Checking the UI without a screen

Viewing the app in a browser through `gtk4-broadwayd` crashes broadwayd once the list re-renders. Debug builds instead have a screenshot mode (`crates/app/src/devtools.rs`). With a headless `gtk4-broadwayd :7` running:

```bash
GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 XDG_CONFIG_HOME=/tmp/x \
APPREESTR_SCREENSHOT=/tmp/shot.png APPREESTR_SCREENSHOT_ACTION=details:pacman:firefox \
APPREESTR_SCREENSHOT_DELAY=2500 target/debug/appreestr
```

- Actions: `details:<id>`, `search:<text>`, `uninstall:<id>`, `system`, `console`, `preferences`.
- Use a scratch `XDG_CONFIG_HOME` so the user's settings are not touched. Put `gtk-enable-animations=0` in its `gtk-4.0/settings.ini`.

## End-to-end removal test

`tests/pkg/PKGBUILD` builds harmless `appreestr-test-dummy` and `appreestr-test-dummy-dep` packages; the steps are in its header. Uninstalling the dummy from the GUI must remove exactly those two packages. It needs the user's sudo and password.
