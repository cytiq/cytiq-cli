use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cytiq_gate::{GateCommand, run_gate};

#[derive(Debug, Parser)]
#[command(name = "cytiq")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Gate {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        fcs: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Gate { model, fcs, output } => run_gate(&GateCommand {
            model_path: model,
            fcs_path: fcs,
            output_path: output,
        }),
    };

    match result {
        Ok(summary) => {
            println!(
                "wrote {} predictions for {}",
                summary.event_count, summary.gate_name
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
