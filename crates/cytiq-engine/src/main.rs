//! The `cytiq` binary: parse the command line, record provenance and run one command.

mod cli;
mod commands;
mod logging;
mod parsers;
mod provenance;

use std::env;
use std::process::ExitCode;

use clap::Parser;
use cytiq_provenance::RunRecorder;

use crate::cli::{Cli, Commands, command_name};
use crate::logging::init_logging;
use crate::provenance::build_info;

fn main() -> ExitCode {
    if let Err(error) = init_logging() {
        eprintln!("failed to initialize logging: {error}");
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(
                error
                    .downcast_ref::<commands::specimen::OutcomeExit>()
                    .map_or(1, |e| e.0.exit_code),
            )
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut recorder = RunRecorder::start(
        format!("cytiq {}", command_name(&cli.command)),
        env!("CARGO_PKG_VERSION"),
        env::args().skip(1).collect(),
    )
    .with_build(build_info());
    recorder.set_config(&cli.command);
    match cli.command {
        Commands::Population(command) => commands::population::run(command, recorder),
        Commands::Specimen(command) => commands::specimen::run(command, recorder),
        Commands::Autospill(args) => commands::autospill::run(args, recorder),
        Commands::Autospread(args) => commands::autospread::run(args, recorder),
        Commands::Autospectral(args) => commands::autospectral::run(args, recorder),
        Commands::Peacoqc(args) => commands::peacoqc::run(args, recorder),
        Commands::Cytoscan(args) => commands::cytoscan::run(args, recorder),
        Commands::Transform(args) => commands::transform::run(args, recorder),
        Commands::Classify(args) => commands::classify::run(args, recorder),
        Commands::Gate(args) => commands::gate::run(args, recorder),
        Commands::Compensate(args) => commands::compensate::run(args, recorder),
        Commands::Mask(command) => commands::mask::run(command, recorder),
        Commands::Plot(args) => commands::plot::run(args, recorder),
        Commands::Preflight(args) => commands::preflight::run(args, recorder),
        Commands::Qc(args) => commands::qc::run(args, recorder),
        Commands::Meta(args) => commands::meta::run(args, recorder),
        Commands::Spillqc(args) => commands::spillqc::run(&args, recorder),
        Commands::Stats(args) => commands::stats::run(args, recorder),
        Commands::Clean(args) => commands::clean::run(args, recorder),
        Commands::Flowsom(args) => commands::flowsom::run(args, recorder),
        Commands::Cytonorm(command) => commands::cytonorm::run(command, recorder),
    }
}
