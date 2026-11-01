use std::path::PathBuf;

use clap::Args;
use cytiq_meta::{MetaConfig, MetaRunner};
use cytiq_provenance::RunRecorder;
use log::debug;
use serde::Serialize;

#[derive(Debug, Serialize, Args)]
pub(crate) struct MetaArgs {
    /// Input FCS file.
    #[arg(long)]
    input: PathBuf,
    /// Output JSON path; omit or pass `-` to write to stdout.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Replace an existing output file.
    #[arg(long)]
    overwrite: bool,
}

pub(crate) fn run(args: MetaArgs, recorder: RunRecorder) -> Result<(), Box<dyn std::error::Error>> {
    let MetaArgs {
        input,
        output,
        overwrite,
    } = args;
    let output = output.filter(|path| path.as_os_str() != "-");
    debug!("starting meta: input={}", input.display());
    let config = MetaConfig {
        input,
        output,
        overwrite,
    };
    // The metadata document already records the input's hash.
    drop(recorder);
    let result = MetaRunner::run(&config)?;
    debug!(
        "meta read {} TEXT keyword(s) and {} parameter(s)",
        result.metadata.text.len(),
        result.metadata.parameter_count
    );
    Ok(())
}
