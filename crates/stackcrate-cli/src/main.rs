mod cli;
mod commands;
mod output;

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;

fn main() {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        "crate=debug"
    } else {
        "crate=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with_target(false)
        .without_time()
        .init();

    if let Err(e) = run(cli) {
        eprintln!("{} {e:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    // `setup` runs before the data dirs / registry necessarily exist in a
    // fully set-up state, but `Engine::open` creates them, so it's safe to
    // always open it first.
    let engine = stackcrate_core::Engine::open()?;

    match cli.command {
        Command::Search { query, source } => commands::search(&engine, &query, source),
        Command::Install {
            target,
            source,
            name,
            exec,
            icon,
            no_launcher,
            checksum,
        } => commands::install(
            &engine,
            &target,
            source,
            name,
            exec,
            icon,
            no_launcher,
            checksum,
            cli.yes,
        ),
        Command::Remove { id, source } => commands::remove(&engine, &id, source, cli.yes),
        Command::Update {
            id,
            source,
            refresh,
        } => commands::update(&engine, id, source, refresh, cli.yes),
        Command::List { what } => commands::list(&engine, &what),
        Command::Status => commands::status(&engine),
        Command::History { limit } => commands::history(&engine, limit),
        Command::Pin { source, id } => commands::pin(&engine, source, &id),
        Command::Unpin { source, id } => commands::unpin(&engine, source, &id),
        Command::Setup { what } => commands::setup(&what, cli.yes),
        Command::Config { what } => commands::config(&what),
    }
}
