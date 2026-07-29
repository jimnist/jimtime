mod commands;
mod daterange;
mod harvest;
mod mapping;
mod paths;
mod repo;
mod selection;
mod slug;
mod store;
mod timeutil;
mod view;

use anyhow::Result;
use clap::Parser;
use commands::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

// List the names of your sub commands here.
register_commands! {
    Status
    Map
    Add
    Today
    Review
    Approve
    Unapprove
    Harvest
}

// Async for the Harvest API.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    cli.commands.run().await?;
    Ok(())
}
