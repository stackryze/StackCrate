# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-17

A broad feature release across every crate, plus a real UX pass on the GUI.
No breaking changes to the CLI's existing commands (only additions), but the
local SQLite registry gained a column and a table — handled by an automatic,
idempotent migration, nothing to do on upgrade.

### Added

**Core engine**

- Package details enrichment: installed/download size, maintainer,
  homepage, and license, where each source can report them
  (`crate show <source> <id>`, and the GUI's new detail view). apt and
  Flatpak report size for free as part of their existing listing calls;
  apt/Flatpak/Snap gained an on-demand `details()` lookup for the rest.
- `crate autoremove` / `Engine::list_orphans`/`autoremove` — apt packages
  pulled in as dependencies that nothing installed still needs.
- `crate rollback <archive|appimage> <id>` — archive and AppImage installs
  now snapshot the previous on-disk state before being overwritten (a
  manual reinstall, or a self-update), so the overwrite can be undone.
  Every destructive change (including a rollback) snapshots first, so
  rolling back is itself always undoable by rolling back again.
- AppImage self-update: AppImages that embed a `gh-releases-zsync`
  `Updateinformation` string are now checked against that GitHub
  repository's latest release and can be updated like any other source,
  via a small hand-rolled ELF section reader (`elf.rs`) rather than a
  parsing dependency.
- Exact-version pinning: `crate pin <source> <id> --version <v>`; apt pins
  now also call `apt-mark hold`/`unhold`, so the hold is enforced at the OS
  level (a plain `apt upgrade` outside StackCrate respects it too), not
  just inside StackCrate's own bulk update.
- `crate explain <source> <id>` — whether a package was explicitly
  requested or pulled in as a dependency, and what currently depends on it
  (apt).
- `crate changelog <name>` — apt's upstream/Debian changelog, with a local
  fallback if network access isn't available.
- Dependency-aware removal warnings: removing an apt package now previews
  what else would be removed first, both in the CLI's confirmation prompt
  and the GUI's remove dialog.
- `crate list disk-usage` / `Engine::disk_usage_summary` — space used by
  each source's installed packages.
- `crate doctor` — a health check: which sources are available, whether the
  background daemon is reachable, whether unattended privilege escalation
  is configured, whether the registry database opens cleanly.
- Search results are now ranked (exact match, then prefix, then substring)
  and deduplicated, instead of whatever order each adapter happened to
  return.
- A live, line-by-line streaming output primitive
  (`process::run_owned_streaming`, `LineSink`) that adapters can opt into
  alongside their existing non-streaming functions, with zero change to
  existing CLI/daemon call sites.
- Config: `require-ac-power-for-auto-update` — on a laptop, `security-only`/
  `auto-all` policies fall back to notify-only while on battery.
- A shared `stackcrate-core::setup` module for installing the daemon's
  systemd `--user` unit, used by both the CLI and the new GUI setting,
  instead of two copies that could drift apart.

**CLI**

- `--json` on every read-only command (`search`, `list`, `status`,
  `history`, `show`, `explain`, `doctor`) for scripting.
- `crate completions <bash|zsh|fish|elvish|powershell>`, and a man page
  (`man crate`) — both also pre-generated and shipped by the `.deb`
  packages and `install.sh`.
- `crate clean` — clears the download cache used for URL installs.

**Daemon**

- Desktop notifications are now actionable: clicking one (or its
  "Open StackCrate" action) launches the GUI.
- Unattended auto-updates respect `require-ac-power-for-auto-update`.

**GUI**

- A package detail view (click any row in Search/Installed/Updates): size,
  maintainer, clickable homepage, license, version/update-available,
  whether it was explicitly requested, a lazily-loaded "why installed +
  what depends on it" section and changelog viewer (apt), a rollback
  option (archive/AppImage, when a snapshot exists), and contextual
  install/remove/update/pin actions — all previously nonexistent; the GUI
  had no per-package detail view at all before this.
- A live, line-by-line progress dialog for install/remove/update/autoremove
  operations, replacing an opaque "Installing..." button label with no
  visible feedback.
- New History and Sources pages: history exposes the existing audit log to
  the GUI for the first time; Sources lets you enable/disable each adapter
  and manage pinned packages, and now also shows the same health checks as
  `crate doctor`.
- Bulk select + bulk actions in Search ("Install Selected") and Installed
  ("Remove Selected"), matching the pattern Updates already had.
- An orphan/autoremove banner in Installed ("N packages are no longer
  needed, frees X MB") with a one-click "Clean Up".
- Friendlier empty/loading states (icon + title + subtitle, a spinner row)
  instead of bare placeholder text.
- Styled, colored "pill" source badges instead of plain dimmed text.
- An About dialog, a keyboard shortcuts dialog, and real keyboard
  shortcuts/accelerators (Settings, shortcuts help, close window, quit).
- Drag-and-drop: drop a `.deb`/archive/AppImage file onto the window to
  install it, alongside the existing file-picker button.
- Settings reorganized into General/Updates/Checksums/Advanced groups, with
  the AC-power toggle and in-GUI sudoers/daemon one-time setup buttons (no
  terminal needed for either anymore).
- The window remembers its size and maximized state across restarts.
- A real app icon, and filtering/sorting across Search, Installed, and
  Updates: a per-source toggle-button filter on all three; a local text
  filter and a sort dropdown (name, size, security-first as applicable) on
  Installed/Updates; an installed-status filter on Search; an "updates
  only" switch on Installed and a "security only" switch on Updates.
  Filtering/sorting is instant (client-side over whatever's already loaded,
  no re-query).

### Changed

- `stackcrate-gui`'s minimum libadwaita version moved from 1.4 to 1.5 (for
  `AlertDialog`/`Dialog`/`AboutDialog`/`PreferencesDialog`); the `.deb`
  package's declared `libadwaita-1-0` dependency was bumped to match.
  Ubuntu 24.04 LTS ships exactly 1.5.0, so this was chosen deliberately as
  the floor rather than the newer 1.6 these features also happen to be
  available at: an earlier pass of this same release used `adw::Spinner`
  (1.6-only) for loading-state rows, which silently raised the *real*
  minimum to 1.6 and would have made `stackcrate-gui_0.2.0_amd64.deb`
  uninstallable on 24.04 -- caught by actually installing libadwaita 1.5.0
  in a clean Ubuntu 24.04 container and running the real binary against it
  (a `symbol lookup error: undefined symbol: adw_spinner_new`), not by
  inspecting version numbers. Replaced with a plain `gtk::Spinner` (which
  has existed since GTK4 shipped), re-verified the same way against the
  same container afterward.
- `PreferencesWindow` (deprecated in libadwaita 1.6) replaced with
  `PreferencesDialog` in the Settings window.
- The CLI/daemon/GUI crates' internal dependency on `stackcrate-core` dropped
  its pinned `version` requirement, keeping only the `path`: these are
  workspace-internal path dependencies that are never resolved from a
  registry, so the version field was pure upkeep that `cargo check` then
  rejected on this very release (Cargo treats a `0.x` minor bump as
  breaking by default) for no actual benefit.

### Fixed

- Search results never reflected installed status: every adapter's
  `search()` always returned `installed: false`, even for a package that
  was, in fact, already installed, since none of them queried installed
  state as part of searching. `Engine::search_all`/the new `search_one`
  (used by `crate search --source <id>`, previously a direct, unenriched
  adapter call) now cross-reference each source's own `list_installed()`
  once per search and mark/version-correct the matches -- caught while
  building the GUI's new "installed only"/"not installed" search filter,
  which depends on this being accurate.
- A packaging-adjacent bug caught by actually launching the rebuilt GUI and
  invoking every window action over D-Bus rather than assuming they were
  correct: the keyboard shortcuts dialog's accelerator labels used
  human-readable strings (`"Ctrl+,"`) instead of GTK's accelerator syntax
  (`"<primary>comma"`), which `GtkShortcutLabel` can't parse — it rendered
  nothing where a shortcut should have shown, and spammed four `Gtk-WARNING`s
  to the terminal every launch.

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

[0.2.0]: https://github.com/stackryze/StackCrate/releases/tag/v0.2.0
[0.1.1]: https://github.com/stackryze/StackCrate/releases/tag/v0.1.1
[0.1.0]: https://github.com/stackryze/StackCrate/releases/tag/v0.1.0
