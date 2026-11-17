# Architecture

## Crate layout

```
crates/
  stackcrate-core/     library: adapters, config, registry, verification, privilege, IPC types
  stackcrate-cli/      bin "crate": argument parsing, confirmation prompts, output formatting
  stackcrate-daemon/   bin "stackcrate-daemon": scheduler, notifications, filesystem watch, IPC server
  stackcrate-gui/      bin "stackcrate-gui": GTK4 + libadwaita frontend
```

`stackcrate-core` has no GUI dependencies and no required async runtime, so it
builds and tests anywhere Rust does. The daemon is the only crate that pulls
in `tokio`; the CLI and GUI call into `stackcrate-core` synchronously (the GUI
bridges blocking calls onto a background thread — see
`crates/stackcrate-gui/src/backend.rs` — so the UI thread never stalls).

## The `PackageSource` trait

Every install mechanism implements one trait
(`crates/stackcrate-core/src/sources/mod.rs`):

```rust
trait PackageSource {
    fn id(&self) -> SourceId;
    fn is_available(&self) -> bool;
    fn list_installed(&self) -> Result<Vec<Package>>;
    fn search(&self, query: &str) -> Result<Vec<Package>>;
    fn list_updates(&self) -> Result<Vec<UpdateInfo>>;
    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome>;
    fn remove(&self, id: &str) -> Result<OperationOutcome>;
    fn update(&self, id: Option<&str>) -> Result<OperationOutcome>;
}
```

Adapters with no remote index (archive, AppImage) return an empty vec from
`search`/`list_updates` rather than erroring — that's a real "there is
nothing to search" answer, not a failure.

`archive` and `appimage` need richer, adapter-specific install options
(`--name`, `--exec`, `--icon`, `--no-launcher`) that don't belong on the
shared trait, so the CLI and GUI call
`sources::archive::install_archive(path, &ArchiveInstallOptions { .. })` /
`sources::appimage::install_appimage(..)` directly for those two instead of
going through the trait object. Every other operation (search, list,
remove, update, and install for apt/deb-file/flatpak/snap) goes through the
trait so callers can treat `Box<dyn PackageSource>` uniformly.

Several adapters also expose extra, adapter-specific free functions beyond
the trait, called directly by `Engine`/CLI/GUI where relevant rather than
forcing every adapter to implement a method most of them have no equivalent
for: `sources::apt::details/changelog/removal_preview/list_autoremovable/
autoremove/hold/unhold/explain`, and streaming (`*_with`/`*_streaming`)
variants of install/remove/update on apt/flatpak/snap/deb-file for callers
that want live output (see "Streaming output" below).

Adding a new source (phase 2: dnf, zypper, pacman+AUR, apk, xbps, eopkg,
Homebrew, npm, pipx — see [ROADMAP.md](ROADMAP.md)) means: add a
`SourceId` variant, a module under `sources/`, implement the trait, and
register it in `sources::all_sources()` / `source_by_id()`. Nothing else
needs to change — the CLI, daemon, and GUI were written against the trait,
not against apt specifically.

## Install-target resolution

`sources::resolve_install_target(input, source_hint)` turns whatever a user
typed into one of:

- `InstallTarget::Named { source, name }` — a bare name, routed to apt
  unless `--source` says otherwise (Flatpak app IDs and Snap names must be
  explicit; see the function's doc comment for why auto-guessing between
  ecosystems was rejected).
- `InstallTarget::LocalFile { source, path }` — an existing file, sniffed by
  extension first and magic bytes second (`sources::sniff`), so a
  downloaded file with no extension still resolves correctly.
- `InstallTarget::Url { source, url }` — downloaded to the cache dir,
  checksum-verified if requested or required, then re-sniffed from the
  actual downloaded bytes (the URL-based guess is only used for progress
  messages before the download starts).

## Privilege model

`crates/stackcrate-core/src/privilege.rs` picks, per call, between:

1. Already root → run directly.
2. A graphical session is present and `pkexec` exists → use it, so polkit
   shows a native dialog. Preferred for the GUI and for anything the daemon
   might do.
3. stdin is a real terminal and `sudo` exists → use `sudo`, which prompts
   inline. This matters for CLI/SSH use, where `pkexec` just fails with
   "cannot open display" if there's no agent to talk to.
4. `SUDO_ASKPASS` is set and `sudo` supports `-A` → use that.

Separately, `privilege::passwordless_available(program)` probes
`sudo -n <program> --version` (never prompts; fails fast if it would need
to) so the daemon's unattended timer can decide whether a privileged
metadata refresh or auto-update is possible *without ever risking a surprise
auth dialog*. Without the optional sudoers rule
(`crate setup sudoers` / `privilege::install_sudoers_rule`), the daemon
still runs its unprivileged checks and still notifies — it just can't
silently refresh apt's metadata or auto-apply anything, and says so in its
log.

## Local state

Everything StackCrate itself needs to remember lives in one SQLite database
(`crates/stackcrate-core/src/registry.rs`, opened in WAL mode so the CLI,
daemon, and GUI can all touch it concurrently without their own locking):

- `manual_installs` — archives/AppImages, since there's no system package
  database for those.
- `update_cache` — what the last check found, so `crate list updates` and
  `crate status` are instant reads instead of re-querying every source.
- `history` — an audit log of every install/remove/update, including
  unattended ones the daemon performed.
- `pins` — packages held back from bulk/auto updates, optionally to a
  specific version string (`pinned_version`); apt pins are additionally
  enforced at the OS level via `apt-mark hold`, so a plain `apt upgrade` run
  outside StackCrate respects them too, not just StackCrate's own bulk update.
- `snapshots` — a previous on-disk state of a manually-tracked install
  (archive or AppImage), saved right before it was overwritten; see
  "Rollback" below.
- `meta` — small key/value bits: last-checked timestamp, and a fingerprint
  of the last update set a notification was actually sent for (so the
  notification doesn't repeat every single check interval for the same
  unchanged set of pending updates).

Schema changes for databases created before a column/table existed go
through a small idempotent `migrate()` run after `CREATE TABLE IF NOT
EXISTS`, e.g. `pins.pinned_version` was added after v0.1.x; `ALTER TABLE
... ADD COLUMN` has no `IF NOT EXISTS` in SQLite, so `migrate()` just
ignores the specific "duplicate column name" failure on a database that
already has it.

## Rollback (`crates/stackcrate-core/src/snapshot.rs`)

apt, Flatpak, and Snap each already have their own rollback story (apt's
package cache, Flatpak's commit history, snap's revisions); archive and
AppImage installs don't, since StackCrate is itself the only thing tracking
them. So for those two sources specifically: before an install overwrites
an existing one (a manual reinstall, or an AppImage self-update), the
current on-disk artifact is moved — not copied — into a dated snapshot
directory (`Paths::snapshot_dir()`, deliberately under the same
`~/.local/opt/stackcrate` tree as the live install directories, so this is
always a same-filesystem rename, never a cross-filesystem copy), and a full
copy of its registry row is stored alongside it as JSON, so restoring never
has to regenerate a desktop file, icon, or exec path by guesswork.

`crate rollback <archive|appimage> <id>` restores the most recent snapshot.
Every destructive change (install-over-existing, and rollback itself)
snapshots whatever it's about to replace first, so a rollback is itself
always undoable by rolling back again — a symmetric stack, not a one-way
operation.

## Streaming output (`process::run_owned_streaming`, `LineSink`)

Every adapter's install/remove/update historically returned only once the
underlying command finished, with the full combined stdout/stderr attached
to the result — fine for the CLI (which can just print it after the fact
with `--verbose`/`STACKCRATE_VERBOSE`), but it meant the GUI's only option
was an opaque "Installing..." button label with zero visible progress for
however long `apt-get install` takes.

`process::run_owned_streaming` is the same primitive as `run_owned`, except
it also invokes a `LineSink` (`Arc<dyn Fn(StreamKind, &str) + Send + Sync>`)
once per line as it arrives, in addition to still assembling the complete
output for the final `CommandOutput` — so this is purely additive, not a
different code path for the CLI vs GUI. Each adapter that benefits from
this (apt, flatpak, snap, deb-file) exposes an additional `*_with`/
`*_streaming` function alongside its plain one (e.g.
`apt::install_named_with(name, Some(sink))`); the plain functions the
CLI/daemon already used are unchanged. `stackcrate-gui/src/backend.rs`'s
`run_streaming` bridges this to the GTK main loop via an `async_channel`,
and `stackcrate-gui/src/progress.rs` is the dialog that displays it live.

## AppImage self-update (`sources/appimage.rs`, `elf.rs`)

Many real AppImages embed an `Updateinformation` string in a named ELF
section (`.upd_info`) describing where to check for new releases — most
commonly `gh-releases-zsync|owner|repo|release|filename-glob`, pointing at
a GitHub repository's releases. `crates/stackcrate-core/src/elf.rs` is a
small, hand-rolled ELF64 section reader (not a dependency: the format
needed is a handful of fixed-offset reads from the section header table,
narrow enough to get right and unit-test directly, including against a real
AppImage on disk) used to read that section without ever executing the
AppImage itself.

`AppImageSource::list_updates`/`update` parse the `gh-releases-zsync` form
specifically (by far the most common in practice), query that repository's
latest release over GitHub's REST API, glob-match the asset filename, and
compare its reported size against the installed file's — the same
approach real-world AppImage update tooling falls back to when a release
doesn't carry a checksum, used here as the primary signal rather than a
fallback, since there's no reliable semantic-version string to compare
instead. `update()` downloads the matched asset (verifying it against
GitHub's own asset digest when one is published, and refusing to proceed
without one if `require-checksum-for-appimage` is set) and reinstalls it
under the same name, which automatically goes through the same
snapshot-before-overwrite path as any other reinstall. Other
`Updateinformation` schemes (a direct `zsync`/static URL) aren't
auto-checked yet — see [ROADMAP.md](ROADMAP.md).

## One-time local setup (`stackcrate-core/src/setup.rs`)

Writing and enabling the daemon's systemd `--user` unit is needed from two
places — the CLI's `crate setup daemon` and the GUI's Settings dialog — so
it lives in `stackcrate-core` rather than being implemented twice and
risking the two copies drifting apart. The sudoers half
(`privilege::render_sudoers_rule`/`install_sudoers_rule`) was already
shared the same way; `setup.rs` is the systemd equivalent.

## CLI/daemon/GUI control-plane IPC

The daemon listens on a Unix socket (`$XDG_RUNTIME_DIR/stackcrate.sock`) and
speaks newline-delimited JSON (`crates/stackcrate-core/src/ipc.rs`) — one
`Request` per line in, one `Response` per line out. This was chosen over
D-Bus specifically for this project: it needs zero additional dependency
surface beyond `serde`, there's exactly one daemon per user session so
service discovery doesn't need a bus, and it's trivially debuggable by hand
(`socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/stackcrate.sock`). `crate status` tries
the daemon first and falls back to reading the local SQLite cache directly
if it isn't running, so the CLI stays useful even without the service
enabled.

## Real-time monitoring, concretely

`stackcrate-daemon`'s scheduler (`crates/stackcrate-daemon/src/scheduler.rs` +
`main.rs`) runs three things concurrently via `tokio::select!`:

1. A timer at `check_interval_minutes` (reloaded from config every cycle, so
   changing it takes effect without a restart).
2. A debounced filesystem watch (`notify` + `notify-debouncer-mini`) on the
   handful of files listed in `watch.rs`, filtered to the exact target paths
   (not "anything in the directory changed" — lock files and other siblings
   in the same directories churn independently and would otherwise
   self-trigger against StackCrate's own read-only queries), and additionally
   rate-limited to at most one fs-triggered check per minute regardless of
   how often the underlying signal fires. Both of those safeguards came out
   of testing against a real filesystem, not just read from a spec — see the
   git history for `watch.rs` and `main.rs`'s `scheduler_loop` if you're
   wondering why they're there.
3. The shutdown signal.

## What the GUI does and doesn't own

Like the CLI, the GUI contains no package-manager-specific logic. Every
button handler calls into `stackcrate-core` (through
`crates/stackcrate-gui/src/backend.rs`'s `run_blocking`/`run_streaming`
helpers, which spawn a plain OS thread and deliver the result — and, for
`run_streaming`, each output line as it arrives — back on the GTK main
context via `async-channel` + `glib::spawn_future_local`). If you need to
change how an install or update actually happens, that change belongs in
`stackcrate-core`, not in a frontend.

Each top-level page (Search, Installed, Updates) is a `gtk::Stack` with two
named children, `"list"` and `"detail"` (`pages::scaffold`/`show_detail`/
`show_list`): clicking a row builds a fresh detail view
(`pages/detail.rs`) and switches to it in place, rather than opening a
separate window or restructuring the window's outer chrome (header, view
switcher, global actions), which stays exactly as it was. History and
Sources are flatter (no drill-down) and stay plain lists.
