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
    // `completions` needs no engine/registry at all, and should work even in
    // a completely fresh environment (e.g. piped straight into a shell
    // rc file during provisioning), so handle it before `Engine::open`.
    if let Command::Completions { shell } = cli.command {
        commands::completions(shell);
        return Ok(());
    }

    // `setup` runs before the data dirs / registry necessarily exist in a
    // fully set-up state, but `Engine::open` creates them, so it's safe to
    // always open it first.
    let engine = stackcrate_core::Engine::open()?;
    let json = cli.json;

    match cli.command {
        Command::Search { query, source } => commands::search(&engine, &query, source, json),
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
        Command::Autoremove => commands::autoremove(&engine, cli.yes),
        Command::Clean => commands::clean(),
        Command::Rollback { source, id } => commands::rollback(&engine, source, &id, cli.yes),
        Command::Doctor => commands::doctor(&engine, json),
        Command::Explain { source, id } => commands::explain(&engine, source, &id, json),
        Command::Changelog { name } => commands::changelog(&name),
        Command::Show { source, id } => commands::show(&engine, source, &id, json),
        Command::List { what } => commands::list(&engine, &what, json),
        Command::Status => commands::status(&engine, json),
        Command::History { limit } => commands::history(&engine, limit, json),
        Command::Pin {
            source,
            id,
            version,
        } => commands::pin(&engine, source, &id, version),
        Command::Unpin { source, id } => commands::unpin(&engine, source, &id),
        Command::Setup { what } => commands::setup(&what, cli.yes),
        Command::Config { what } => commands::config(&what),
        Command::Completions { .. } => unreachable!("handled above"),
    }
}
