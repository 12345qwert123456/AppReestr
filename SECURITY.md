# Security policy

AppReestr removes software, and part of it runs as root, so security reports are taken seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting instead:
[Security → Report a vulnerability](https://github.com/12345qwert123456/AppReestr/security/advisories/new).

Include the AppReestr version, your distribution, and steps to reproduce. You should get an answer
within a few days.

## What counts

The privileged part is `crates/helper` (`appreestr-helper`, started through `pkexec`). It trusts
nothing it is given: it validates names and refs, re-scans the system, recomputes protection from
the bundled rules only, re-plans the removal and requires the same plan fingerprint before it runs
`pacman`, `flatpak` or `snap` with a cleared environment. `remove-path` deletes only canonical,
unowned direct children of `/opt` and `/usr/local/{bin,lib,share/applications}`.

Reports of the following kind are in scope:

- a way to make the helper remove something it should refuse (a protected package, a path outside
  the allowed directories, a symlink or race that redirects a removal);
- running commands or writing files as root with attacker-controlled input;
- a bypass of the authorization the helper or the "Restart as Administrator" session relies on.

`force-remove-packages` (the Danger Zone) intentionally ignores protection, but only after the user
types the package name in the confirmation dialog. Reaching it any other way is a vulnerability.

## Supported versions

Only the latest release and the `main` branch receive fixes.
