use std::path::PathBuf;

use clap::Args;
use cytiq_meta::{MetaConfig, MetaRunner};
use cytiq_provenance::{RunRecorder, record_path_for_file};
use log::debug;
use serde::Serialize;

use crate::provenance::record_provenance;

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

pub(crate) fn run(
    args: MetaArgs,
    mut recorder: RunRecorder,
) -> Result<(), Box<dyn std::error::Error>> {
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
    // A file output gets its provenance record like every other command's
    // output; the document on stdout already carries the input's hash.
    if config.output.is_some() {
        recorder.snapshot_roles(&[("input", config.input.clone())])?;
    }
    let result = MetaRunner::run(&config)?;
    if let Some(output) = result.output.as_deref() {
        record_provenance(
            recorder,
            &[output.to_path_buf()],
            &record_path_for_file(output),
        )?;
    }
    debug!(
        "meta read {} TEXT keyword(s) and {} parameter(s)",
        result.metadata.text.len(),
        result.metadata.parameter_count
    );
    Ok(())
}
