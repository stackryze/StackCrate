# Security Policy

## Reporting a vulnerability

Please report security issues privately rather than opening a public issue.
Open a GitHub security advisory on this repository (Security tab → "Report
a vulnerability"), or contact the maintainer directly if that isn't
available. Include:

- the affected version/commit
- what's needed to reproduce it
- what you believe the impact is (what does it actually let an attacker do)

Please allow a reasonable amount of time to investigate and release a fix
before any public disclosure.

## Scope and design notes relevant to security review

- **Privilege escalation** happens only via `pkexec` or `sudo`, invoked as a
  normal child process for one command at a time — there is no setuid
  binary, no always-on root service, and no custom privilege helper.
  `crates/stackcrate-core/src/privilege.rs` is the only place that decides how a
  command gets elevated.
- **The optional sudoers rule** (`crate setup sudoers`) is rendered by
  `privilege::render_sudoers_rule`, restricted to the exact package-manager
  binaries detected on the system via a `Cmnd_Alias`, syntax-checked with
  `visudo -cf` before being installed, and never grants `NOPASSWD: ALL`.
  Review the rendered rule (the CLI prints it before asking for
  confirmation) rather than trusting this description blindly.
- **Archive extraction** (`crates/stackcrate-core/src/archive_safety.rs`)
  resolves every entry path against the destination directory before
  writing and rejects anything that would escape it, including via a
  symlink target. The test suite constructs malicious archives at the raw
  header-byte level (bypassing the `tar`/`zip` crates' own API-level
  guardrails) specifically so these tests don't pass merely because a
  well-behaved library wouldn't let you build that payload in the first
  place.
- **Downloaded files** (URL installs) support checksum verification
  (`--checksum`) and can be configured to require one
  (`require-checksum-for-appimage`/`-archive`). Nothing downloaded is
  executed as part of installation beyond what's inherent to the install
  type itself (e.g. an AppImage is, by definition, an executable; StackCrate
  does not add code execution beyond making it runnable and generating a
  launcher).
- **The background daemon** never escalates privilege on its own timer
  unless a passwordless sudo rule is already installed
  (`privilege::passwordless_available`, which only ever probes with
  `sudo -n`, which fails fast rather than prompting). Review
  `crates/stackcrate-daemon/src/scheduler.rs` for exactly what it's allowed to
  do unattended under each update policy.
- **No telemetry, no phone-home.** Network access is limited to installing
  what you asked for and to each package manager's own configured
  repositories.
