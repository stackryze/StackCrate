#!/usr/bin/env bash
# Builds pallet_<version>_<arch>.deb (CLI + daemon) and, if pallet-gui was
# built, pallet-gui_<version>_<arch>.deb (GUI, depends on the base package).
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
MAINTAINER="Pallet contributors <pallet@stackryze.invalid>"
HOMEPAGE="https://github.com/stackryze/StackCrate"
DATE_RFC2822="$(date -R)"

for bin in pallet pallet-daemon; do
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
    "$stage/usr/share/doc/pallet"

  install -m 0755 "$RELEASE_DIR/pallet" "$stage/usr/bin/pallet"
  install -m 0755 "$RELEASE_DIR/pallet-daemon" "$stage/usr/bin/pallet-daemon"
  install -m 0644 "$ROOT/packaging/polkit/org.pallet.policy" "$stage/usr/share/polkit-1/actions/org.pallet.policy"
  # Package-installed binaries are on PATH, so the unit doesn't need the
  # %h/.local/bin-relative ExecStart that `pallet setup daemon` generates
  # for from-source installs.
  sed 's#%h/\.local/bin/pallet-daemon#/usr/bin/pallet-daemon#' \
    "$ROOT/packaging/systemd/pallet-daemon.service" >"$stage/usr/lib/systemd/user/pallet-daemon.service"
  chmod 0644 "$stage/usr/lib/systemd/user/pallet-daemon.service"
  install -m 0644 "$ROOT/LICENSE" "$stage/usr/share/doc/pallet/copyright"
  install -m 0644 "$ROOT/README.md" "$stage/usr/share/doc/pallet/README.md"
  write_changelog pallet "$stage/usr/share/doc/pallet/changelog.gz"

  cat >"$stage/DEBIAN/control" <<EOF
Package: pallet
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Depends: libc6 (>= 2.34)
Recommends: polkitd, pkexec, flatpak, snapd
Suggests: pallet-gui (= $VERSION)
Homepage: $HOMEPAGE
Description: unified package manager for apt, archives, AppImage, Flatpak, Snap
 Pallet brings apt, local .deb files, plain archives, AppImages, Flatpak,
 and Snap behind one CLI and one background update monitor. The monitor
 never escalates privilege on its own timer unless a passwordless sudo
 rule has been explicitly installed (pallet setup sudoers); by default it
 only notifies.
EOF

  dpkg-deb --build --root-owner-group "$stage" "$OUTPUT_DIR/pallet_${VERSION}_${ARCH}.deb"
  rm -rf "$stage"
}

build_gui_package() {
  [[ -x "$RELEASE_DIR/pallet-gui" ]] || {
    echo "pallet-gui binary not found; skipping pallet-gui_*.deb" >&2
    return 0
  }
  local stage
  stage="$(mktemp -d)"

  mkdir -p "$stage/DEBIAN" \
    "$stage/usr/bin" \
    "$stage/usr/share/applications" \
    "$stage/usr/share/doc/pallet-gui"

  install -m 0755 "$RELEASE_DIR/pallet-gui" "$stage/usr/bin/pallet-gui"
  install -m 0644 "$ROOT/packaging/desktop/org.pallet.Pallet.desktop" \
    "$stage/usr/share/applications/org.pallet.Pallet.desktop"
  if [[ -f "$ROOT/assets/pallet.png" ]]; then
    mkdir -p "$stage/usr/share/icons/hicolor/512x512/apps"
    install -m 0644 "$ROOT/assets/pallet.png" "$stage/usr/share/icons/hicolor/512x512/apps/org.pallet.Pallet.png"
  fi
  install -m 0644 "$ROOT/LICENSE" "$stage/usr/share/doc/pallet-gui/copyright"
  write_changelog pallet-gui "$stage/usr/share/doc/pallet-gui/changelog.gz"

  cat >"$stage/DEBIAN/control" <<EOF
Package: pallet-gui
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Depends: pallet (= $VERSION), libc6 (>= 2.34), libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.4)
Recommends: desktop-file-utils
Homepage: $HOMEPAGE
Description: GTK4/libadwaita desktop app for Pallet
 Search, installed-packages, and pending-updates views, plus a
 header-bar action to install a local .deb/archive/AppImage file,
 backed by the same pallet-core engine as the pallet CLI.
EOF

  dpkg-deb --build --root-owner-group "$stage" "$OUTPUT_DIR/pallet-gui_${VERSION}_${ARCH}.deb"
  rm -rf "$stage"
}

build_base_package
build_gui_package

echo "Built packages:"
ls -la "$OUTPUT_DIR"/*.deb
