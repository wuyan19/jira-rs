//! `jira`: one binary, two surfaces.
//!
//! - `jira mcp` — stdio MCP server (stdout is protocol; never write to it
//!   outside the rmcp transport)
//! - everything else — CLI: JSON on stdout, diagnostics on stderr,
//!   stable exit codes (1 = config, 2 = auth, 3 = api, 4 = not found)
//!
//! Bare `jira` prints help and exits; MCP mode is always an explicit
//! subcommand so an accidental invocation never blocks on stdin.

mod cli;
mod mcp;
mod ops;
mod upgrade;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Mcp => {
            if let Err(e) = mcp::serve().await {
                eprintln!("jira mcp: {e}");
                std::process::exit(1);
            }
        }
        command => cli::run(command).await,
    }
}
