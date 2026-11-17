//! Pre-generates the `crate(1)` man page for packaging (`build-deb.sh`, the
//! release tarball). Not installed for end users.
//!
//! Usage: `cargo run --bin gen-man -- <output-dir>`

#[path = "../cli.rs"]
mod cli;

use clap::CommandFactory;
use clap_mangen::Man;
use std::io::Write;

fn main() -> std::io::Result<()> {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    std::fs::create_dir_all(&out_dir)?;

    let cmd = cli::Cli::command();
    let path = std::path::Path::new(&out_dir).join("crate.1");
    let mut file = std::fs::File::create(&path)?;
    Man::new(cmd).render(&mut file)?;
    file.flush()?;
    println!("wrote {}", path.display());
    Ok(())
}
