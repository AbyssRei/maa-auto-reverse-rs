use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Gui,
    ScanOnce {
        #[arg(long)]
        window: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    maa_auto_reverse_rs::bootstrap()?;

    match cli.command.unwrap_or(Command::Gui) {
        Command::Gui => maa_auto_reverse_rs::app::run_gui().map_err(Into::into),
        Command::ScanOnce { window } => {
            let output = maa_auto_reverse_rs::orchestrator::run_scan_once_cli(window)?;
            println!("{output}");
            Ok(())
        }
    }
}
