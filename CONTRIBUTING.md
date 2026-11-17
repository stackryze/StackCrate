# Contributing

Thanks for looking at StackCrate. A few ground rules to keep the codebase
coherent as it grows:

## Development setup

```
cargo build --workspace --exclude stackcrate-gui   # core + CLI + daemon, no system deps
sudo apt install libgtk-4-dev libadwaita-1-dev # only needed for stackcrate-gui
cargo build -p stackcrate-gui
```

## Checks before opening a PR

```
cargo fmt --all -- --check
cargo clippy --workspace --exclude stackcrate-gui --all-targets -- -D warnings
cargo test --workspace --exclude stackcrate-gui
```

(`stackcrate-gui` is excluded from the default commands because it needs system
GTK4/libadwaita headers that aren't assumed to be present; CI builds it in a
separate job that installs them first.)

## Architecture rules

- Package-manager command construction, output parsing, and anything that
  touches the filesystem beyond StackCrate's own state lives in `stackcrate-core`.
  The CLI, daemon, and GUI call into it; they don't duplicate it.
- New install mechanisms implement `sources::PackageSource`
  (`crates/stackcrate-core/src/sources/mod.rs`) and get registered in
  `all_sources()`/`source_by_id()`. See [ARCHITECTURE.md](ARCHITECTURE.md)
  for the shape of that trait and why two adapters (archive, appimage)
  bypass it for their richer install options.
- Anything that shells out to an external command goes through
  `stackcrate_core::process` (for plain commands) or `stackcrate_core::privilege`
  (for anything that needs root), not a raw `std::process::Command` in a
  source adapter — that's what keeps timeout handling, error shapes, and the
  privilege-escalation decision consistent across every adapter.
- Prefer a real subprocess call to a package manager's own CLI over
  reimplementing its logic. When a command's exact output format matters
  (parsers, `apt list --upgradable`'s security-suite heuristic, etc.),
  include a unit test with real captured sample output, and say in a code
  comment where the format assumption came from.

## Security-sensitive changes

Be extra careful with, and call out explicitly in the PR description:

- anything under `stackcrate_core::privilege` or `stackcrate_core::verify`
- archive/zip extraction (`archive_safety.rs`) — any change here should come
  with a test that tries to break it (crafted `..` paths, symlinks that
  point outside the destination), not just a test that it works for
  well-behaved input
- the sudoers template and polkit policy in `packaging/`
- anything that changes what the daemon does without an explicit user
  action (the default policy is notify-only for a reason)

## Testing against a real system

`stackcrate-core`'s adapters shell out to real package-manager binaries, so most
of its tests are either pure parsing/validation tests (no subprocess calls)
or deliberately skip when the relevant binary isn't present, rather than
mocking `apt-get`/`dpkg`/`flatpak`/`snap`. If you're adding an adapter,
prefer the same pattern: capture real sample output from the real tool into
a test fixture rather than installing/removing real packages in CI.

## Commit messages

Conventional Commits: `type(scope): summary`, where `type` is one of `feat`,
`fix`, `chore`, `docs`, `ci`, or `test`, and `scope` is usually a crate name
(`core`, `cli`, `daemon`, `gui`) or `packaging`. Keep commits scoped to one
concern — a reviewer should be able to read one commit's diff and its
message and understand both without needing the rest of the branch. Prefer
several small, logically-grouped commits over one large one.

## Cutting a release

1. Update `workspace.package.version` in the root `Cargo.toml` (every crate
   inherits it) and run `cargo check --workspace --exclude stackcrate-gui` once
   so `Cargo.lock` picks up the bump.
2. Add a new `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md` (and its
   `[X.Y.Z]: .../releases/tag/vX.Y.Z` link at the bottom), following
   [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
3. Commit both as `chore(release): vX.Y.Z`, tag it (`git tag vX.Y.Z`), and
   push the tag (`git push origin vX.Y.Z`).
4. `.github/workflows/release.yml` takes it from there: it fails closed if
   the tag doesn't match `Cargo.toml`'s version, runs the full test/lint
   suite, builds release binaries, builds `stackcrate`/`stackcrate-gui` `.deb`
   packages (`packaging/debian/build-deb.sh`), and publishes a GitHub
   Release with the binaries, `.deb`s, a tarball, and `SHA256SUMS`, using
   that version's `CHANGELOG.md` section as the release notes.

To build and sanity-check a release locally before tagging:

```
cargo build --release --workspace
./packaging/debian/build-deb.sh <version>      # -> dist/*.deb
lintian dist/*.deb                              # optional, if installed
```
