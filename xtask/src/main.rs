mod cli;
mod commands;
mod common;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let repo_root = common::repo_root()?;
    commands::run(cli.command, &repo_root)
}
