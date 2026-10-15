//! Command-line surface shared by `claude-code-dual` and the test-only
//! certification harness, so both parse launch arguments identically.
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "claude-code-dual",
    about = "Experimental Linux/WSL dual-session launcher",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inspect configuration offline without executing Claude, helpers, hooks or MCP.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show the fixed session model contract and validation status.
    Contract,
    /// Start an owned loopback listener and a scoped session on the pinned Claude Code version.
    Launch {
        #[arg(long)]
        claude: Option<std::path::PathBuf>,
        #[arg(long)]
        resume: Option<String>,
    },
}
