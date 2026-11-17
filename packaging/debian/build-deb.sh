#!/usr/bin/env bash
# Builds stackcrate_<version>_<arch>.deb (CLI + daemon) and, if stackcrate-gui was
# built, stackcrate-gui_<version>_<arch>.deb (GUI, depends on the base package).
# Split in two because the CLI/daemon only link against glibc (see `ldd`),
# while the GUI needs the full GTK4/libadwaita stack -- no reason to force
# that onto a headless/server install of the base package.
#
# Validated with `lintian`; every remaining warning is a deliberate,
# documented tradeoff (see comments below) rather than an oversight.
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

for bin in crate stackcrate-daemon; do
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
    "$stage/usr/share/doc/stackcrate"

  install -m 0755 "$RELEASE_DIR/crate" "$stage/usr/bin/crate"
  install -m 0755 "$RELEASE_DIR/stackcrate-daemon" "$stage/usr/bin/stackcrate-daemon"
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
Depends: stackcrate (= $VERSION), libc6 (>= 2.34), libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.4)
Recommends: desktop-file-utils
Homepage: $HOMEPAGE
Description: GTK4/libadwaita desktop app for StackCrate
 Search, installed-packages, and pending-updates views, plus a
 header-bar action to install a local .deb/archive/AppImage file,
 backed by the same stackcrate-core engine as the crate CLI.
EOF

  dpkg-deb --build --root-owner-group "$stage" "$OUTPUT_DIR/stackcrate-gui_${VERSION}_${ARCH}.deb"
  rm -rf "$stage"
}

build_base_package
build_gui_package

echo "Built packages:"
ls -la "$OUTPUT_DIR"/*.deb
