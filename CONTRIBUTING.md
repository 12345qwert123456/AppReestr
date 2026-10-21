# Contributing

Thanks for helping. Bug reports, rule fixes (`data/rules/*.toml`) and translations are especially
welcome. The architecture, the removal safety model and the UI conventions are described in
[CLAUDE.md](CLAUDE.md); read the relevant part before changing code.

## Setup

Arch Linux or a derivative with `rust`, `gtk4`, `libadwaita`, `gettext`, `desktop-file-utils` and
`appstream` installed.

```bash
git clone https://github.com/12345qwert123456/AppReestr
cd AppReestr
cargo run -p appreestr                         # the GUI from the tree
cargo run -p appreestr-dump -- --timings --all # what is listed and why
```

## Before you send a pull request

```bash
cargo fmt --all
make check    # fmt --check, clippy -D warnings, tests, desktop/metainfo validation (= CI)
```

- Tune classification and protection in `data/rules/*.toml` before adding code.
- Code, comments and gettext msgids are in English. User-visible strings must be literals inside
  `tr`, `trf` or `trn` (`appreestr_core::i18n`). After adding strings run `make pot` and translate
  the new entries in `po/ru.po` (clear `#, fuzzy`, keep `msgfmt --check` clean).
- Build screens from the factories in `crates/app/src/ui.rs`; do not hard-code margins or colours.
- Anything that changes what the helper may remove needs a test and a note in the pull request.
  Never route an ordinary uninstall through `force-remove-packages`.
- Do not run `pkexec` probes casually: your session's polkit agent will show a password dialog.

## Reporting bugs

Open an issue and include the output of `appreestr-dump --timings --all` (or `--plan PKG`) when the
problem is about what is listed or what a removal would do. Report vulnerabilities privately, see
[SECURITY.md](SECURITY.md).
