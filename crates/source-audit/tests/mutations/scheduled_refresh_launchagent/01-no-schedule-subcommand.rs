//! target: crates/cli/src/main.rs
//! mode: replace
//! why: the scheduled-refresh entry point removed, so the documented background refresh has no way in
#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Report,
    Observe,
    Scope,
}

fn main() {}
