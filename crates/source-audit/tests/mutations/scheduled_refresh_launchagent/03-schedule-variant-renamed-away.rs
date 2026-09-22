//! target: crates/cli/src/main.rs
//! mode: replace
//! why: alias/rename variant -- the subcommand kept but renamed, so a user's documented `swamp schedule` silently stops existing
#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Report,
    Timer,
}

fn main() {}
