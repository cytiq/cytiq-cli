# Upstream reference for the end-to-end golden pipeline (parity/gen/pipeline.py).
# Sourced by run_upstream.R: run_pipeline() writes, per dataset, per-event
# compensated and transformed values, the RemoveMargins and PeacoQC masks, and
# FlowSOM metaclusters on the true good events.

pipeline_fluor <- c("FL1-A", "FL2-A", "FL3-A", "FL4-A", "FL5-A", "FL6-A")
pipeline_scatter <- c("FSC-A", "FSC-H", "FSC-W", "SSC-A")
pipeline_qc_channels <- pipeline_fluor[1:5]

run_pipeline <- function() {
  suppressPackageStartupMessages({
    library(PeacoQC)
    library(FlowSOM)
  })
  for (dataset in c("pipeline", "pipeline_beads")) {
    dataset_dir <- file.path(data_dir, dataset)
    if (!dir.exists(dataset_dir)) next
    target <- file.path(out_dir, dataset)
    dir.create(target, recursive = TRUE, showWarnings = FALSE)
    # `Live/Dead` contains the TEXT delimiter, escaped as `//`.
    raw <- read.FCS(file.path(dataset_dir, "sample.fcs"), transformation = FALSE,
                    truncate_max_range = FALSE, emptyValue = FALSE)

    # RemoveMargins on raw values, as PeacoQC recommends before compensation;
    # cytiq judges scatter detectors too (`cytiq_fcs::margin_channels`).
    margins <- RemoveMargins(raw, channels = c(pipeline_fluor, pipeline_scatter), output = "full")
    removed <- rep(0L, nrow(raw))
    removed[margins$indices_margins] <- 1L
    write.csv(data.frame(removed = removed), file.path(target, "r_margins.csv"), row.names = FALSE)

    compensated <- timed(paste0(dataset, "_compensate"),
                         compensate(raw, spillover(raw)[["$SPILLOVER"]]))
    write.csv(exprs(compensated)[, pipeline_fluor], file.path(target, "r_compensated.csv"),
              row.names = FALSE)

    asinh150 <- arcsinhTransform(a = 0, b = 1 / 150, c = 0)
    transformed <- transform(compensated, transformList(pipeline_fluor, asinh150))
    write.csv(exprs(transformed)[, pipeline_fluor], file.path(target, "r_transformed.csv"),
              row.names = FALSE)

    peacoqc <- timed(paste0(dataset, "_peacoqc"), PeacoQC(
      transformed, channels = pipeline_qc_channels, determine_good_cells = "all",
      plot = FALSE, save_fcs = FALSE, report = FALSE, output_directory = NULL,
      MAD = 6, IT_limit = 0.6, consecutive_bins = 5, remove_zeros = FALSE,
      time_channel_parameter = "Time", suppress_message = TRUE))
    write.csv(data.frame(good = as.integer(peacoqc$GoodCells)), file.path(target, "r_peacoqc.csv"),
              row.names = FALSE)

    if (dataset == "pipeline") {
      truth <- read.csv(file.path(dataset_dir, "truth", "events.csv"))
      good <- rowSums(truth[, c("debris", "doublet", "dead", "unstable", "margin")]) == 0
      fsom <- timed("pipeline_flowsom", FlowSOM(
        transformed[good, ], colsToUse = pipeline_qc_channels, nClus = 7,
        xdim = 10, ydim = 10, rlen = 10, scale = TRUE, seed = 1, silent = TRUE))
      write.csv(data.frame(raw_row = truth$raw_row[good],
                           metacluster = as.integer(GetMetaclusters(fsom))),
                file.path(target, "r_flowsom.csv"), row.names = FALSE)
    }
  }
}
