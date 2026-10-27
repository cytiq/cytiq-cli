# CompensAID on raw FCS files, shared by run_upstream.R and real/real.R.
# Requires flowCore to be attached.

# The CompensAID vignette chain on each raw tube: RemoveMargins, compensate
# with $SPILL, estimateLogicle + transform and PeacoQC over the spillover
# channels (the vignette passes every column; cytiq uses the spillover
# channels), the marker channels, then CompensAID with its defaults.
compensaid_chain <- function(path) {
  suppressPackageStartupMessages({ library(PeacoQC); library(CompensAID) })
  ff <- read.FCS(path, truncate_max_range = FALSE)
  spill <- Filter(Negate(is.null), spillover(ff))[[1]]  # SPILL, $SPILLOVER or SPILLOVER
  channels <- colnames(spill)
  ff <- suppressWarnings(RemoveMargins(ff, channels = channels, output = "frame"))
  ff <- compensate(ff, spill)
  ff <- transform(ff, estimateLogicle(ff, channels))
  pq <- suppressWarnings(PeacoQC(ff, channels = channels, determine_good_cells = "all", plot = FALSE,
                                 save_fcs = FALSE, report = FALSE, output_directory = NULL,
                                 suppress_message = TRUE))
  pq$FinalFF[, names(markernames(pq$FinalFF))]
}

# CompensAID's matrix, matrixInfo and the .DensityGating cut-offs it starts from.
write_compensaid <- function(ff, target) {
  dir.create(target, recursive = TRUE, showWarnings = FALSE)
  res <- suppressMessages(CompensAID::CompensAID(ff = ff))
  write.csv(res$matrix, file.path(target, "ssi_matrix.csv"))
  write.csv(as.data.frame(res$matrixInfo), file.path(target, "ssi_segments.csv"), row.names = FALSE)
  padded <- ff
  padded@exprs <- rbind(padded@exprs, rep(0, ncol(padded@exprs)))
  center <- CompensAID:::.DetermineCenter(og = padded)
  cuts <- CompensAID:::.DensityGating(og = padded, cp.value = center$center.plot)
  write.csv(data.frame(channel = cuts$channel, cutoff = cuts$opt, center = center$center.plot,
                       separation = center$separation.distance, events = nrow(ff)),
            file.path(target, "cutoffs.csv"), row.names = FALSE)
}
