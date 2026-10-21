#!/bin/sh
# Usage: tools/bump-version.sh X.Y.Z
# Sets the version in Cargo.toml, Cargo.lock, the metainfo and the AUR PKGBUILD.
# Commit and tag (vX.Y.Z) by hand afterwards.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 X.Y.Z" >&2; exit 2; }
ver=$1
case $ver in *[!0-9.]* | .* | *. | *..* | "") echo "bad version: $ver" >&2; exit 2;; esac
cd "$(dirname "$0")/.."
meta=data/io.github.appreestr.AppReestr.metainfo.xml
grep -q "<release version=\"$ver\"" "$meta" && { echo "metainfo already has $ver" >&2; exit 1; }

sed -i "0,/^version = \".*\"/s//version = \"$ver\"/" Cargo.toml
cargo update --workspace --offline
sed -i "s|^\(  *\)<releases>|&\n\1  <release version=\"$ver\" date=\"$(date +%F)\"/>|" "$meta"
sed -i "s/^pkgver=.*/pkgver=$ver/; s/^pkgrel=.*/pkgrel=1/" packaging/aur/appreestr/PKGBUILD
echo "bumped to $ver; review with git diff, then: git commit -am 'Release $ver' && git tag v$ver"
