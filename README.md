# Pallet

[![CI](https://github.com/stackryze/StackCrate/actions/workflows/ci.yml/badge.svg)](https://github.com/stackryze/StackCrate/actions/workflows/ci.yml)
[![Release](https://github.com/stackryze/StackCrate/actions/workflows/release.yml/badge.svg)](https://github.com/stackryze/StackCrate/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/stackryze/StackCrate)](https://github.com/stackryze/StackCrate/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**One package manager for every way software ends up on your Linux box.**

Pallet is a native, Rust-based package manager for Debian/Ubuntu systems that
unifies `apt`, local `.deb` files, plain archives (`.tar.gz`, `.tar.xz`,
`.tar.bz2`, `.zip`), AppImages, Flatpak, and Snap behind one CLI, one
background monitor, and one GTK4/libadwaita desktop app. It watches for
updates in the background and notifies you — it does not install anything
without asking unless you explicitly turn that on.

```
$ pallet install firefox            # apt
$ pallet install ./app.deb           # local .deb, with dependency resolution
$ pallet install ./tool-1.2.tar.gz   # archive, with launcher + PATH symlink
$ pallet install ./App.AppImage      # AppImage, with icon + launcher
$ pallet install --source flatpak org.videolan.VLC
$ pallet install --source snap spotify
```

## Why another package manager?

Every distro package manager is excellent at its own job and bad at
everyone else's. Real machines end up with software from five or six of
these at once, each with different update stories, and nothing watches all
of them together. Pallet doesn't replace apt, Flatpak, or Snap — it wraps
them with one consistent interface and adds the parts none of them do on
their own: a single install verb regardless of source, a background monitor
that actually watches for changes instead of only polling on a timer, and
checksum-verified handling of files you download yourself.

## What's in this repository

| Crate | What it is |
|---|---|
| [`pallet-core`](crates/pallet-core) | The library: source adapters, config, local registry (SQLite), verification, privilege handling. No UI, fully unit tested, no GUI dependencies. |
| [`pallet-cli`](crates/pallet-cli) | The `pallet` command. Scriptable, colored output, confirmation prompts, JSON-free by design for now (see [ROADMAP](ROADMAP.md)). |
| [`pallet-daemon`](crates/pallet-daemon) | `pallet-daemon`: a small background service — timer-driven checks, a filesystem watch for faster reaction to real package-state changes, desktop notifications, and a Unix-socket control API the CLI/GUI use to avoid duplicating logic. |
| [`pallet-gui`](crates/pallet-gui) | A GTK4 + libadwaita desktop app: search, installed packages, pending updates, a header-bar "install a local file" action, and settings. Confirmed building and running live (search, install, remove, update all exercised against a real daemon) on Ubuntu 26.04 with `libgtk-4-dev`/`libadwaita-1-dev` installed. |

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the pieces fit together,
[ROADMAP.md](ROADMAP.md) for what's deliberately not here yet, and
[CHANGELOG.md](CHANGELOG.md) for release history. Releases are tagged
`vX.Y.Z` and published as prebuilt binaries, `.deb` packages, and a
tarball — see [Releases](https://github.com/stackryze/StackCrate/releases).

## Supported sources (v1 scope)

Pallet's first milestone targets Debian/Ubuntu machines specifically, rather
than trying to cover every distro's package manager on day one:

- **apt / dpkg** — named packages through `apt-get`, local `.deb` files
  through `dpkg -i` with automatic `apt-get install -f` dependency recovery
  (the GDebi pattern), update detection via `apt list --upgradable` with
  security-update classification.
- **Archives** (`.tar`, `.tar.gz`/`.tgz`, `.tar.xz`/`.txz`, `.tar.bz2`/`.tbz2`,
  `.zip`) — extracted with zip-slip/path-traversal protection, with automatic
  detection (and manual override) of the main executable, a `~/.local/bin`
  symlink, and a `.desktop` launcher.
- **AppImage** — copied into a stable location, made executable, and given a
  real application-menu entry by extracting its embedded `.desktop` file and
  icon (via the AppImage's own `--appimage-extract`).
- **Flatpak** and **Snap** — thin wrappers over their own CLIs, including
  non-destructive pending-update queries (`flatpak remote-ls --updates`,
  `snap refresh --list`) so checking never silently changes anything.

Fedora/RPM, Arch/pacman+AUR, Alpine/apk, and the rest are an intentional
phase 2 — see [ROADMAP.md](ROADMAP.md).

## Installing

### From a release (recommended)

Download `pallet_<version>_amd64.deb` (and `pallet-gui_<version>_amd64.deb`
for the desktop app) from the
[latest release](https://github.com/stackryze/StackCrate/releases/latest)
and install with:

```
sudo apt install ./pallet_<version>_amd64.deb ./pallet-gui_<version>_amd64.deb
systemctl --user enable --now pallet-daemon
```

Using `apt install ./file.deb` rather than `dpkg -i` lets apt resolve the
declared dependencies (polkit, GTK4/libadwaita, etc.) automatically.

### From source

```
git clone https://github.com/stackryze/StackCrate.git
cd StackCrate
./install.sh
```

This builds release binaries with `cargo`, installs `pallet` and
`pallet-daemon` to `~/.local/bin`, builds `pallet-gui` if GTK4/libadwaita
development headers are available (`sudo apt install libgtk-4-dev
libadwaita-1-dev` if not), and sets up a `systemd --user` service for the
background monitor. Flags: `--skip-gui`, `--skip-polkit`, `--skip-daemon`,
`--yes`.

### Manual build

```
cargo build --release --workspace --exclude pallet-gui   # core + CLI + daemon
cargo build --release -p pallet-gui                       # needs GTK4/libadwaita
```

## Using the CLI

```
pallet search <query> [--source apt|flatpak|snap]
pallet install <name|file|url> [--source ...] [--name ...] [--exec ...] [--icon ...] [--checksum sha256sum]
pallet remove <id> [--source ...]
pallet update [<id>] [--refresh] [--all]
pallet list installed|updates|sources
pallet status
pallet history
pallet pin|unpin <source> <id>
pallet setup sudoers|daemon
pallet config show|set <key> <value>
```

Every mutating command prompts for confirmation unless you pass `-y/--yes`;
in a non-interactive session (scripts, cron), it refuses to proceed without
`--yes` rather than hanging or silently defaulting to "yes".

## Real-time monitoring

`pallet-daemon` runs as a `systemd --user` service:

- A timer (`check-interval-minutes` in config, default 60) checks every
  active source for updates. This never needs root: apt's check reads only
  the local cache, and Flatpak's/Snap's pending-update queries are
  unprivileged by design.
- A filesystem watch on `/var/lib/dpkg/status` and `/var/lib/snapd/state.json`
  triggers an extra check shortly after something changes outside Pallet
  (e.g. you ran `apt install` by hand), rate-limited to at most once a
  minute so an unusually chatty filesystem can't turn this into a busy loop.
- A desktop notification fires when the pending-update set actually changes
  (not on every tick), summarizing the count and how many are security
  updates.
- Update policy (`pallet config set update-policy ...`) controls what
  happens automatically: `notify-only` (default), `security-only`, or
  `auto-all`. Anything that needs root only runs unattended if a passwordless
  sudo rule is installed (`pallet setup sudoers`); otherwise the daemon never
  pops a surprise authentication prompt on its own timer — it just notifies,
  and you run `pallet update` yourself.

## Security model

- **No copyleft/trust surprises for privileged code paths.** Every
  privileged operation goes through `pkexec` (preferred, for a proper
  polkit dialog) or `sudo`, never a custom setuid binary. The optional
  sudoers rule (`pallet setup sudoers`) is narrowly scoped with a
  `Cmnd_Alias` to the exact package-manager binaries found on the system —
  never a blanket `NOPASSWD: ALL`.
- **Archive extraction is zip-slip-safe.** Every tar/zip entry's resolved
  path is checked to stay inside the destination before anything is written;
  symlink targets are checked too. See the tests in
  `crates/pallet-core/src/archive_safety.rs` for the exact attacks this
  blocks (a crafted `../../etc/passwd` path and a symlink that tries to point
  outside the destination are both exercised directly against raw archive
  bytes, not just inputs a well-behaved archive library would produce).
- **Checksum verification** (`--checksum sha256:...` or `sha512:...`) is
  available for URL installs, and can be made mandatory per source type via
  `pallet config set require-checksum-for-appimage true` (and
  `-for-archive`). GPG signature verification (`verify::verify_signature`)
  is implemented for callers that have a detached signature and a trusted
  keyring.
- **No telemetry.** Pallet makes network requests only to download a file
  you asked it to install, or to talk to apt/Flatpak/Snap's own configured
  repositories.

## Acknowledgments

[OmniPkg](https://github.com/grosserknallkopf/OmniPkg) was reviewed in
detail while planning Pallet's adapter behavior (PackageKit fallbacks,
pacman-lock handling, safe-extraction edge cases, sudoers templating). No
code from it is vendored here — Pallet is a from-scratch Rust codebase with
a different architecture (CLI + daemon + GUI instead of GUI-only, a
real-time filesystem watch instead of a cron job, direct `.deb`/archive/file
installs) — but it's worth crediting as prior art that shaped some of the
design decisions above. If your priority is the widest possible distro
coverage today rather than the real-time-monitoring and direct-file-install
features described here, it's a solid, actively simpler alternative.

## License

MIT — see [LICENSE](LICENSE).
