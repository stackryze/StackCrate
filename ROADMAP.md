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

Rollback/snapshots (archive and AppImage), exact-version pinning (apt-mark
hold-backed), dependency-aware removal warnings, and AppImage self-update
all shipped in v0.2.0 — see [CHANGELOG.md](CHANGELOG.md). What's left:

- **Rollback/snapshots for apt itself.** v0.2.0's snapshot/rollback only
  covers archive and AppImage installs, since those are the two sources
  StackCrate is the sole owner of. Hooking into Timeshift/Snapper where
  present would extend the same `crate rollback` idea to apt-managed
  packages, but that's a meaningfully different mechanism (filesystem/btrfs
  snapshots of the whole system, not StackCrate's own registry) and a
  larger scope than the archive/AppImage case.
- **More AppImage update backends.** v0.2.0 only auto-checks the
  `gh-releases-zsync` `Updateinformation` scheme (GitHub Releases), the most
  common one in practice. GitLab/Codeberg/Forgejo-hosted releases and a
  direct `zsync`/static-URL scheme are recognized as "this AppImage has
  *some* update info" but aren't queried automatically yet.
- **Real zsync delta updates.** v0.2.0's AppImage self-update always
  downloads the full new asset; real rsync-style delta transfer (only
  downloading the bytes that changed) would need the `zsync` protocol
  specifically, not just reading `.zsync` file headers. `zsync-rs` on
  crates.io is the most mature option if this gets picked up later; it's a
  meaningfully bigger lift than the size/digest-based "is there an update"
  check already shipped.
- **Stop executing untrusted AppImages just to read their metadata.**
  `extract_desktop_info` still runs the AppImage's own `--appimage-extract`
  to pull out its `.desktop` file/icon. `backhand` (pure-Rust SquashFS
  reader) + `goblin` (ELF parsing, already informing `elf.rs`'s design)
  would let that become in-process parsing instead, with execution as an
  explicit, opt-in last resort — a real security hardening, not just a
  refactor, and deferred because it touches an already-working, tested path
  for a benefit that's about defense in depth rather than a missing
  capability.
- **"Keep both" on a same-name reinstall**, instead of today's always-replace
  (snapshot-then-overwrite). Low effort on top of the existing snapshot
  mechanism, but needs registry support for two concurrently-tracked
  installs sharing a display name, which v0.2.0 didn't take on alongside
  everything else.

## Phase 4 — interface

- **Qt frontend** alongside GTK4, if there's demand from KDE/Plasma users —
  the CLI/daemon/GUI split already means this would be a new frontend crate
  calling the same `stackcrate-core` + IPC surface, not a rewrite.
- **System tray indicator** fed by the daemon's IPC, as a lighter-weight
  alternative to opening the full GUI just to see update counts. Deferred
  specifically because GTK4 dropped the old `GtkStatusIcon` API entirely;
  the StatusNotifierItem/AppIndicator replacement doesn't render at all on
  stock GNOME Shell without an extra, non-default shell extension, which
  would make this unreliable across desktops rather than a straightforward
  addition.
- **JSON output for mutating commands.** v0.2.0 added `--json` to every
  read-only command (`search`/`list`/`status`/`history`/`show`/`explain`/
  `doctor`); `install`/`remove`/`update`/`autoremove`/`rollback`/`pin`/
  `unpin` still only print human-readable confirmation/outcome text. Left
  out deliberately for now rather than guessed at — these are also the
  commands with confirmation prompts and streaming output, so a JSON mode
  for them needs its own design (how does a prompt behave in `--json` mode?
  what does streamed progress look like as JSON lines?) rather than reusing
  the read-only commands' "just serialize the result struct" approach.

## Explicitly not planned

- **Apt repository/PPA management** (`add-apt-repository`-equivalent, or
  managing `/etc/apt/sources.list.d` entries). This would mean StackCrate
  deciding what repositories — and by extension, signing keys — a system
  trusts, which is a meaningfully larger security surface than everything
  else here (which only ever acts on repositories the system already
  trusts). Worth real design/review before taking on, not a quick addition.
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
