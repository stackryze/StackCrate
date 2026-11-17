# Roadmap

StackCrate's first milestone (this codebase, today) deliberately targets
Debian/Ubuntu + the cross-distro app formats (archives, AppImage, Flatpak,
Snap). Everything below is scoped-out on purpose, not forgotten.

## Phase 2 — more distro package managers

The `PackageSource` trait and `SourceId` enum were designed for this; adding
a source doesn't touch the CLI, daemon, or GUI:

- Fedora/RHEL: `dnf`/`dnf5`, with PackageKit (`pkcon`) preferred when present
  (mirrors how this codebase already prefers it conceptually for apt, except
  v1 doesn't implement the PackageKit path at all yet — worth revisiting for
  apt too, since `pkcon` gives consistent output across apt/dnf/zypper).
- openSUSE/SUSE: `zypper`.
- Arch/Manjaro: `pacman`, plus an AUR helper (`yay`/`paru`) adapter.
- Alpine: `apk`.
- Void: `xbps`.
- Solus: `eopkg`.
- Developer/language-level: Homebrew, npm (global), pipx, and probably
  `cargo install`/`go install` given how often those come up alongside
  system packages on a dev machine.

## Phase 3 — update quality of life

- **Rollback/snapshots.** Hook into Timeshift/Snapper where present for
  distro packages; for archive/AppImage installs, a tar snapshot before
  overwrite-on-update would make `crate rollback <id>` straightforward
  since StackCrate already owns that directory.
- **Version pinning beyond "skip in bulk update".** Pin to an exact version,
  not just hold back from auto-update.
- **AppImage self-update.** Parse embedded `Updateinformation` and use
  zsync for delta updates instead of "remove and reinstall."
- **Dependency-aware removal warnings** ("removing libfoo will also remove
  3 packages that depend on it") for apt, surfaced before the confirmation
  prompt rather than discovered from apt's own output after the fact.

## Phase 4 — interface

- **JSON output mode** (`crate --json list updates`) for scripting, once
  the output shape has stabilized from real use.
- **Qt frontend** alongside GTK4, if there's demand from KDE/Plasma users —
  the CLI/daemon/GUI split already means this would be a new frontend crate
  calling the same `stackcrate-core` + IPC surface, not a rewrite.
- **System tray indicator** fed by the daemon's IPC, as a lighter-weight
  alternative to opening the full GUI just to see update counts.

## Explicitly not planned

- **A privileged system daemon.** Every privileged operation goes through
  `pkexec`/`sudo` per call; see [ARCHITECTURE.md](ARCHITECTURE.md#privilege-model)
  for why. A root-owned always-on service would need a much larger security
  review to justify the extra attack surface over what this buys.
- **Full cross-ecosystem dependency resolution.** StackCrate delegates
  dependency resolution to each native package manager (apt resolves apt
  dependencies, Flatpak resolves its own runtimes, etc.) rather than
  attempting a universal solver across unrelated ecosystems — that's a
  different, much harder project than "one interface over several package
  managers."
