# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.1] - 2026-11-17

### Fixed

- `stackcrate-gui`'s `.deb` listed `Depends: stackcrate` even though the GUI
  binary never actually calls the `crate` CLI or talks to the daemon's IPC
  socket -- it links `stackcrate-core` directly. Since neither package is
  hosted in an apt repository, apt had no way to satisfy that dependency
  unless you passed both `.deb` files to `apt install` in the same command,
  so installing a `stackcrate-gui_*.deb` downloaded by itself failed outright
  with "Depends: stackcrate (= 0.1.0) but it is not installable". Relaxed to
  `Recommends: stackcrate`, which apt still installs by default when both
  files are given together, but no longer blocks the GUI install when it
  isn't.

## [0.1.0] - 2026-11-17

Initial release. Debian/Ubuntu-focused v1 scope (see `ROADMAP.md` for what's
intentionally not here yet).

### Added

- **`stackcrate-core`**: the `PackageSource` trait and six adapters --
  apt (named packages via `apt-get`/`apt-cache`/`dpkg-query`), local `.deb`
  files (`dpkg -i` with automatic `apt-get install -f` dependency recovery),
  plain archives (`.tar`, `.tar.gz`, `.tar.xz`, `.tar.bz2`, `.zip` with
  zip-slip-safe extraction), AppImage (with `.desktop`/icon extraction),
  Flatpak, and Snap.
- Checksum (sha256/sha512) and GPG signature verification for downloaded
  files; configurable to be mandatory per source type.
- `pkexec`-preferred, `sudo`-fallback privilege escalation, with a
  passwordless-sudo probe so unattended operations never risk a surprise
  auth prompt, and an optional narrowly-scoped sudoers rule
  (`crate setup sudoers`).
- A WAL-mode SQLite registry for manual installs, update cache, operation
  history, and pins.
- **`crate` CLI**: search/install/remove/update/list/status/history/
  pin/unpin/setup/config, with confirmation prompts that refuse to proceed
  without `--yes` in a non-interactive session.
- **`stackcrate-daemon`**: a `systemd --user` background monitor --
  timer-driven checks, a debounced and rate-limited filesystem watch for
  faster reaction to changes made outside StackCrate, deduped desktop
  notifications, and update policies (notify-only/security-only/auto-all).
- **`stackcrate-gui`**: a GTK4 + libadwaita desktop app -- search, installed,
  updates, a header-bar "install a local file" action, and settings.
- Packaging: a `systemd --user` service unit, a polkit policy for clearer
  `pkexec` prompts, a `.desktop` launcher, `install.sh`, and `.deb` packages
  (`stackcrate`, `stackcrate-gui`) built and verified with `lintian` and a real
  `apt-get install` in a throwaway container.

[0.1.1]: https://github.com/stackryze/StackCrate/releases/tag/v0.1.1
[0.1.0]: https://github.com/stackryze/StackCrate/releases/tag/v0.1.0
