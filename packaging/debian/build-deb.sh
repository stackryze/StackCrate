#!/usr/bin/env bash
# Builds stackcrate_<version>_<arch>.deb (CLI + daemon) and, if stackcrate-gui was
# built, stackcrate-gui_<version>_<arch>.deb (GUI, depends on the base package).
# Split in two because the CLI/daemon only link against glibc (see `ldd`),
# while the GUI needs the full GTK4/libadwaita stack -- no reason to force
# that onto a headless/server install of the base package.
#
# Validated with `lintian`. `crate`'s man page and shell completions
# (generated below) mean `no-manual-page` no longer fires for usr/bin/crate;
# the remaining warnings are deliberate, documented tradeoffs rather than
# oversights: `no-manual-page` for usr/bin/stackcrate-daemon and
# usr/bin/stackcrate-gui (neither is meant to be run by typing a command --
# one's a systemd-managed background service, the other's launched from its
# desktop file/app menu entry), and `uses-dpkg-database-directly` (the
# daemon's filesystem watch reads /var/lib/dpkg/status directly for faster
# reaction to changes made outside StackCrate -- see
# crates/stackcrate-daemon/src/watch.rs).
set -euo pipefail
umask 0022 # otherwise a group-writable umask leaks into package file modes

VERSION="${1:?usage: build-deb.sh <version> [arch] [output-dir]}"
ARCH="${2:-amd64}"
OUTPUT_DIR="${3:-dist}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RELEASE_DIR="$ROOT/target/release"
MAINTAINER="StackCrate contributors <stackcrate@stackryze.invalid>"
HOMEPAGE="https://github.com/stackryze/StackCrate"
DATE_RFC2822="$(date -R)"

for bin in crate stackcrate-daemon gen-man gen-completions; do
  [[ -x "$RELEASE_DIR/$bin" ]] || {
    echo "error: $RELEASE_DIR/$bin not found; run cargo build --release first" >&2
    exit 1
  }
done

mkdir -p "$OUTPUT_DIR"

# Debian policy wants /usr/share/doc/<pkg>/changelog.gz even for a
# single-package-per-release "native" layout like this one.
write_changelog() {
  local pkg="$1" dest="$2"
  gzip -9 -n -c >"$dest" <<EOF
$pkg ($VERSION) unstable; urgency=medium

  * See https://github.com/stackryze/StackCrate/blob/main/CHANGELOG.md

 -- $MAINTAINER  $DATE_RFC2822
EOF
}

build_base_package() {
  local stage
  stage="$(mktemp -d)"

  mkdir -p "$stage/DEBIAN" \
    "$stage/usr/bin" \
    "$stage/usr/share/polkit-1/actions" \
    "$stage/usr/lib/systemd/user" \
    "$stage/usr/share/doc/stackcrate" \
    "$stage/usr/share/man/man1" \
    "$stage/usr/share/bash-completion/completions" \
    "$stage/usr/share/zsh/vendor-completions" \
    "$stage/usr/share/fish/vendor_completions.d"

  install -m 0755 "$RELEASE_DIR/crate" "$stage/usr/bin/crate"
  install -m 0755 "$RELEASE_DIR/stackcrate-daemon" "$stage/usr/bin/stackcrate-daemon"

  # gen-man/gen-completions are additional [[bin]] targets of the
  # stackcrate-cli crate (see its Cargo.toml; not installed for end users),
  # so the same `cargo build --release --workspace` this script already
  # requires (see the check above) also produced them at $RELEASE_DIR --
  # just run them, the same way `crate` itself above is installed, not
  # rebuilt, by this script.
  local gen_dir
  gen_dir="$(mktemp -d)"
  "$RELEASE_DIR/gen-man" "$gen_dir"
  "$RELEASE_DIR/gen-completions" "$gen_dir"

  # Debian convention is a gzipped man page; -n omits the embedded
  # filename/timestamp so the .gz is reproducible across rebuilds (same
  # reasoning as write_changelog's gzip below). Mode 0644 comes from this
  # script's own `umask 0022` above, same as changelog.gz.
  gzip -9 -n -c "$gen_dir/crate.1" >"$stage/usr/share/man/man1/crate.1.gz"

  # clap_complete names bash's output "<bin-name>.bash" (here: crate.bash).
  # bash-completion's loader does also fall back to trying "<name>.bash",
  # but the bare "<name>" (no extension) is the long-standing, portable
  # convention essentially every other Debian-packaged completion uses, so
  # that's the name it's installed under here. zsh's "_crate" and fish's
  # "crate.fish" are already exactly the filenames their completion systems
  # look for, so those two install unchanged. elvish/PowerShell completions
  # also come out of gen-completions but aren't installed into this package
  # -- not meaningful on Debian/Ubuntu -- `crate completions elvish` /
  # `crate completions powershell` still cover anyone who wants them
  # directly.
  install -m 0644 "$gen_dir/crate.bash" "$stage/usr/share/bash-completion/completions/crate"
  install -m 0644 "$gen_dir/_crate" "$stage/usr/share/zsh/vendor-completions/_crate"
  install -m 0644 "$gen_dir/crate.fish" "$stage/usr/share/fish/vendor_completions.d/crate.fish"
  rm -rf "$gen_dir"

  install -m 0644 "$ROOT/packaging/polkit/org.stackcrate.policy" "$stage/usr/share/polkit-1/actions/org.stackcrate.policy"
  # Package-installed binaries are on PATH, so the unit doesn't need the
  # %h/.local/bin-relative ExecStart that `crate setup daemon` generates
  # for from-source installs.
  sed 's#%h/\.local/bin/stackcrate-daemon#/usr/bin/stackcrate-daemon#' \
    "$ROOT/packaging/systemd/stackcrate-daemon.service" >"$stage/usr/lib/systemd/user/stackcrate-daemon.service"
  chmod 0644 "$stage/usr/lib/systemd/user/stackcrate-daemon.service"
  install -m 0644 "$ROOT/LICENSE" "$stage/usr/share/doc/stackcrate/copyright"
  install -m 0644 "$ROOT/README.md" "$stage/usr/share/doc/stackcrate/README.md"
  write_changelog stackcrate "$stage/usr/share/doc/stackcrate/changelog.gz"

  cat >"$stage/DEBIAN/control" <<EOF
Package: stackcrate
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Depends: libc6 (>= 2.34)
Recommends: polkitd, pkexec, flatpak, snapd
Suggests: stackcrate-gui (= $VERSION)
Homepage: $HOMEPAGE
Description: unified package manager for apt, archives, AppImage, Flatpak, Snap
 StackCrate brings apt, local .deb files, plain archives, AppImages, Flatpak,
 and Snap behind one CLI and one background update monitor. The monitor
 never escalates privilege on its own timer unless a passwordless sudo
 rule has been explicitly installed (crate setup sudoers); by default it
 only notifies.
EOF

  dpkg-deb --build --root-owner-group "$stage" "$OUTPUT_DIR/stackcrate_${VERSION}_${ARCH}.deb"
  rm -rf "$stage"
}

build_gui_package() {
  [[ -x "$RELEASE_DIR/stackcrate-gui" ]] || {
    echo "stackcrate-gui binary not found; skipping stackcrate-gui_*.deb" >&2
    return 0
  }
  local stage
  stage="$(mktemp -d)"

  mkdir -p "$stage/DEBIAN" \
    "$stage/usr/bin" \
    "$stage/usr/share/applications" \
    "$stage/usr/share/doc/stackcrate-gui"

  install -m 0755 "$RELEASE_DIR/stackcrate-gui" "$stage/usr/bin/stackcrate-gui"
  install -m 0644 "$ROOT/packaging/desktop/org.stackcrate.StackCrate.desktop" \
    "$stage/usr/share/applications/org.stackcrate.StackCrate.desktop"
  if [[ -f "$ROOT/assets/stackcrate.png" ]]; then
    mkdir -p "$stage/usr/share/icons/hicolor/512x512/apps"
    install -m 0644 "$ROOT/assets/stackcrate.png" "$stage/usr/share/icons/hicolor/512x512/apps/org.stackcrate.StackCrate.png"
  fi
  install -m 0644 "$ROOT/LICENSE" "$stage/usr/share/doc/stackcrate-gui/copyright"
  write_changelog stackcrate-gui "$stage/usr/share/doc/stackcrate-gui/changelog.gz"

  cat >"$stage/DEBIAN/control" <<EOF
Package: stackcrate-gui
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Depends: libc6 (>= 2.34), libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.5)
Recommends: stackcrate (= $VERSION), desktop-file-utils
Homepage: $HOMEPAGE
Description: GTK4/libadwaita desktop app for StackCrate
 Search, installed-packages, pending-updates, history, and sources views
 with filtering/sorting, a package detail view with live install/remove/
 update progress, plus a header-bar action to install a local .deb/
 archive/AppImage file, backed by the same stackcrate-core engine as the
 crate CLI.
EOF

  dpkg-deb --build --root-owner-group "$stage" "$OUTPUT_DIR/stackcrate-gui_${VERSION}_${ARCH}.deb"
  rm -rf "$stage"
}

build_base_package
build_gui_package

echo "Built packages:"
ls -la "$OUTPUT_DIR"/*.deb
