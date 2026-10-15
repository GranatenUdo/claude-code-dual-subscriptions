use clap::Parser;
use claude_code_proxy::dual::{
    self,
    cli::{Cli, Command},
};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor { json } => {
            let report = dual::doctor::current()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("claude-code-dual: offline report; launch readiness is not established");
                for item in report.findings {
                    println!("{}: {} — {}", item.scope, item.status, item.detail);
                }
                println!("Client version and authentication: unknown; neither executed nor read.");
            }
            // A diagnostic run succeeding is not a readiness result.
            std::process::exit(2);
        }
        Command::Contract => {
            println!(
                "Parent: {}\nGeneral-purpose default: {}\nReviewer: explicit opus, invocation not guaranteed\nClient target: {}\nFORCE: never\n1M: client assumption, upstream capacity unverified\nLaunch: experimental Linux/WSL, preflight required",
                dual::PARENT_MODEL,
                dual::WORKER_MODEL,
                dual::CLIENT_VERSION
            );
        }
        Command::Launch { claude, resume } => {
            // No runtime or background thread has been started by this CLI.
            let code = unsafe { dual::launch::run(claude, resume)? };
            std::process::exit(code);
        }
    }
    Ok(())
}
