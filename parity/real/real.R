#!/usr/bin/env Rscript
# Upstream side of parity/real/run_real.sh: the same processing with flowCore,
# PeacoQC and FlowSOM, written per event next to the cytiq outputs.
# Usage: Rscript parity/real/real.R <out_dir>

suppressPackageStartupMessages({
  library(flowCore)
  library(PeacoQC)
  library(FlowSOM)
})

out_dir <- commandArgs(trailingOnly = TRUE)[[1]]

read_raw <- function(dir) {
  read.FCS(file.path(dir, "sample.fcs"), transformation = FALSE, truncate_max_range = FALSE,
           emptyValue = FALSE)
}

spill_of <- function(ff) {
  spill <- spillover(ff)
  spill[!vapply(spill, is.null, logical(1))][[1]]
}

compensate_and_transform <- function(ff, dir, channels) {
  compensated <- compensate(ff, spill_of(ff))
  write.csv(exprs(compensated)[, channels, drop = FALSE], file.path(dir, "r_compensated.csv"),
            row.names = FALSE)
  transformed <- transform(compensated, transformList(channels, logicleTransform()))
  write.csv(exprs(transformed)[, channels, drop = FALSE], file.path(dir, "r_transformed.csv"),
            row.names = FALSE)
  transformed
}

# FlowSOM 68983 ---------------------------------------------------------------
dir <- file.path(out_dir, "flowsom_68983")
if (dir.exists(dir)) {
  markers <- strsplit(readLines(file.path(dir, "markers.txt")), ",")[[1]]
  transformed <- compensate_and_transform(read_raw(dir), dir, markers)
  for (seed in c(1, 2)) {
    fsom <- FlowSOM(transformed, colsToUse = markers, nClus = 10, xdim = 10, ydim = 10,
                    rlen = 10, scale = TRUE, seed = seed, silent = TRUE)
    write.csv(data.frame(metacluster = as.integer(GetMetaclusters(fsom))),
              file.path(dir, sprintf("r_flowsom_seed%d.csv", seed)), row.names = FALSE)
  }
}

# QC chains: PeacoQC 111 and CYTIQ_REAL_DATA files ------------------------------
for (dir in list.dirs(out_dir, recursive = FALSE)) {
  if (!file.exists(file.path(dir, "channels.txt"))) next
  channels <- strsplit(readLines(file.path(dir, "channels.txt")), ",")[[1]]
  raw <- read_raw(dir)
  margins <- RemoveMargins(raw, channels = channels, output = "full")
  removed <- rep(0L, nrow(raw))
  removed[margins$indices_margins] <- 1L
  write.csv(data.frame(removed = removed), file.path(dir, "r_margins.csv"), row.names = FALSE)
  transformed <- compensate_and_transform(raw, dir, channels)
  peacoqc <- PeacoQC(transformed, channels = channels, determine_good_cells = "all", plot = FALSE,
                     save_fcs = FALSE, report = FALSE, output_directory = NULL, MAD = 6,
                     IT_limit = 0.6, consecutive_bins = 5, remove_zeros = FALSE,
                     time_channel_parameter = "Time", suppress_message = TRUE)
  write.csv(data.frame(good = as.integer(peacoqc$GoodCells)), file.path(dir, "r_peacoqc.csv"),
            row.names = FALSE)
}
writeLines(capture.output(sessionInfo()), file.path(out_dir, "sessionInfo.txt"))
