//! Pre-generates shell completion scripts for packaging (`build-deb.sh`, the
//! release tarball). Not installed for end users; `crate completions <shell>`
//! (built into the real CLI) covers anyone building/installing another way.
//!
//! Usage: `cargo run --bin gen-completions -- <output-dir>`

#[path = "../cli.rs"]
mod cli;

use clap::CommandFactory;
use clap_complete::Shell;

fn main() -> std::io::Result<()> {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    std::fs::create_dir_all(&out_dir)?;

    let mut cmd = cli::Cli::command();
    for shell in [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::Elvish,
        Shell::PowerShell,
    ] {
        let path = clap_complete::generate_to(shell, &mut cmd, "crate", &out_dir)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
