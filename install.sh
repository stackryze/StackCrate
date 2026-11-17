#!/usr/bin/env bash
set -euo pipefail

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
APPLICATIONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"
POLKIT_ACTIONS_DIR="/usr/share/polkit-1/actions"

SKIP_GUI=0
SKIP_POLKIT=0
SKIP_DAEMON=0
ASSUME_YES=0

for arg in "$@"; do
  case "$arg" in
    --skip-gui) SKIP_GUI=1 ;;
    --skip-polkit) SKIP_POLKIT=1 ;;
    --skip-daemon) SKIP_DAEMON=1 ;;
    -y|--yes) ASSUME_YES=1 ;;
    --help)
      printf 'Usage: ./install.sh [--skip-gui] [--skip-polkit] [--skip-daemon] [--yes]\n'
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "$arg" >&2
      exit 2
      ;;
  esac
done

require() {
  command -v "$1" >/dev/null 2>&1 || { printf '%s is required but was not found on PATH.\n' "$1" >&2; exit 1; }
}

require cargo
require rustc

printf 'Building crate and stackcrate-daemon (release)...\n'
( cd "$SRC_DIR" && cargo build --release -p stackcrate-cli -p stackcrate-daemon )

GUI_BUILT=0
if [[ "$SKIP_GUI" -eq 0 ]]; then
  printf 'Building stackcrate-gui (release)...\n'
  if ( cd "$SRC_DIR" && cargo build --release -p stackcrate-gui ); then
    GUI_BUILT=1
  else
    printf '\nstackcrate-gui failed to build, likely missing GTK4/libadwaita development packages.\n'
    printf 'On Debian/Ubuntu: sudo apt install libgtk-4-dev libadwaita-1-dev\n'
    printf 'Continuing without the GUI; the CLI and daemon are unaffected.\n\n'
  fi
fi

mkdir -p "$BIN_DIR" "$APPLICATIONS_DIR" "$ICON_DIR"
install -m 0755 "$SRC_DIR/target/release/crate" "$BIN_DIR/crate"
install -m 0755 "$SRC_DIR/target/release/stackcrate-daemon" "$BIN_DIR/stackcrate-daemon"
printf 'Installed %s and %s\n' "$BIN_DIR/crate" "$BIN_DIR/stackcrate-daemon"

if [[ "$GUI_BUILT" -eq 1 ]]; then
  install -m 0755 "$SRC_DIR/target/release/stackcrate-gui" "$BIN_DIR/stackcrate-gui"
  install -m 0644 "$SRC_DIR/packaging/desktop/org.stackcrate.StackCrate.desktop" "$APPLICATIONS_DIR/org.stackcrate.StackCrate.desktop"
  if [[ -f "$SRC_DIR/assets/stackcrate.png" ]]; then
    install -m 0644 "$SRC_DIR/assets/stackcrate.png" "$ICON_DIR/org.stackcrate.StackCrate.png"
  fi
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPLICATIONS_DIR" || true
  printf 'Installed %s and its desktop launcher\n' "$BIN_DIR/stackcrate-gui"
fi

if [[ "$SKIP_POLKIT" -eq 0 ]]; then
  if command -v sudo >/dev/null 2>&1; then
    printf '\nInstalling the polkit policy (cosmetic nicer auth prompts) requires sudo.\n'
    if [[ "$ASSUME_YES" -eq 1 ]] || { read -r -p 'Install packaging/polkit/org.stackcrate.policy to /usr/share/polkit-1/actions? [y/N] ' reply; [[ "$reply" =~ ^[Yy]$ ]]; }; then
      sudo install -m 0644 "$SRC_DIR/packaging/polkit/org.stackcrate.policy" "$POLKIT_ACTIONS_DIR/org.stackcrate.policy"
      printf 'Installed polkit policy.\n'
    fi
  else
    printf 'sudo not found; skipping the optional polkit policy install.\n'
  fi
fi

if [[ "$SKIP_DAEMON" -eq 0 ]]; then
  printf '\nSetting up the background monitor service...\n'
  if [[ "$ASSUME_YES" -eq 1 ]]; then
    "$BIN_DIR/crate" -y setup daemon
  else
    "$BIN_DIR/crate" setup daemon
  fi
fi

cat <<EOF

StackCrate is installed.
  CLI:    crate --help
  Daemon: systemctl --user status stackcrate-daemon
EOF
if [[ "$GUI_BUILT" -eq 1 ]]; then
  printf '  GUI:    stackcrate-gui (also in your application menu)\n'
fi
cat <<EOF

For unattended background checks without repeated password prompts, run:
  crate setup sudoers
EOF
