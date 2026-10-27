#!/usr/bin/env Rscript
# Upstream side of parity/real/run_real.sh: the same processing with flowCore,
# PeacoQC, FlowSOM, autospill, AutoSpectral and CompensAID, written next to the
# cytiq outputs.
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

# CompensAID chains: compensaid_68983 and CYTIQ_REAL_DATA files ----------------
source(file.path(dirname(sub("^--file=", "", grep("^--file=", commandArgs(FALSE), value = TRUE))),
                 "..", "compensaid.R"))
for (dir in list.dirs(out_dir, recursive = FALSE)) {
  if (!dir.exists(file.path(dir, "spillqc"))) next
  write_compensaid(compensaid_chain(file.path(dir, "sample.fcs")), file.path(dir, "r_spillqc"))
  preprocessed <- file.path(dir, "preprocessed.fcs")
  if (file.exists(preprocessed)) {
    write_compensaid(read.FCS(preprocessed, truncate_max_range = FALSE),
                     file.path(dir, "r_spillqc_preprocessed"))
  }
}

# AutoSpill chains: single-stain controls --------------------------------------
for (dir in list.dirs(out_dir, recursive = FALSE)) {
  if (!file.exists(file.path(dir, "controls_full.csv"))) next
  suppressPackageStartupMessages(library(autospill))
  work <- file.path(dir, "r_autospill_work")
  dir.create(work, showWarnings = FALSE)
  old <- setwd(work)
  asp <- get.autospill.param("minimal")
  asp$worker.process.n <- parallel::detectCores()
  asp$verbose <- FALSE
  flow.control <- read.flow.control(dir, file.path(dir, "controls_full.csv"), asp)
  flow.gate <- gate.flow.data(flow.control, asp)
  initial <- get.marker.spillover(TRUE, flow.gate, flow.control, asp)
  refined <- refine.spillover(initial, NULL, flow.gate, flow.control, asp)
  setwd(old)
  write.csv(refined[[1]], file.path(dir, "r_autospill_spillover.csv"))
  gates <- do.call(rbind, lapply(names(flow.gate), function(s) {
    ids <- flow.gate[[s]]
    if (is.character(ids)) ids <- as.integer(sub(".*\\.", "", ids))
    data.frame(sample = s, event = as.integer(ids))
  }))
  write.csv(gates, file.path(dir, "r_autospill_gates.csv"), row.names = FALSE)
}

# AutoSpectral chains: spectral single-stain controls and stained samples -------
# The vignette workflow: define.flow.control (density gates), clean.controls
# (autofluorescence removal, scatter-matched universal negatives),
# get.fluorophore.spectra (rlm slopes), then OLS unmixing. Abundances are
# written for every 10th event: with R's spectra, with cytiq's spectra (the
# solver check) and with cytiq's fluorophore spectra plus R's AF spectrum.
for (dir in list.dirs(out_dir, recursive = FALSE)) {
  if (!file.exists(file.path(dir, "control_file.csv"))) next
  suppressPackageStartupMessages(library(AutoSpectral))
  work <- file.path(dir, "r_autospectral_work")
  dir.create(work, showWarnings = FALSE)
  old <- setwd(work)
  asp <- get.autospectral.param(cytometer = readLines(file.path(dir, "cytometer.txt")), figures = TRUE)
  flow.control <- define.flow.control(file.path(dir, "controls"), file.path(dir, "control_file.csv"), asp)
  spectra <- get.fluorophore.spectra(clean.controls(flow.control, asp, main.figures = FALSE), asp,
                                     figures = FALSE)
  # R's own spread: the same spectra without clean.controls.
  uncleaned <- get.fluorophore.spectra(flow.control, asp, use.clean.expr = FALSE, figures = FALSE)
  setwd(old)
  write.csv(spectra, file.path(dir, "r_spectra.csv"))
  write.csv(uncleaned[rownames(spectra), colnames(spectra)], file.path(dir, "r_spectra_uncleaned.csv"))
  mine <- read.csv(file.path(dir, "autospectral", "references", "fluorophore_spectra.csv"),
                   row.names = 1, check.names = FALSE)
  rownames(mine)[rownames(mine) == "Autofluorescence"] <- "AF"
  mine <- as.matrix(mine[rownames(spectra), colnames(spectra)])
  with_r_af <- mine
  with_r_af["AF", ] <- spectra["AF", ]
  for (fcs in list.files(file.path(dir, "sample"), pattern = "\\.fcs$", full.names = TRUE)) {
    raw <- exprs(read.FCS(fcs, transformation = FALSE, truncate_max_range = FALSE))[, colnames(spectra)]
    rows <- seq(1, nrow(raw), by = 10)
    unmixed <- function(matrix, prefix) {
      values <- unmix.ols(raw[rows, ], matrix)
      colnames(values) <- paste0(prefix, colnames(values))
      values
    }
    write.csv(data.frame(event = as.integer(rows - 1), unmixed(spectra, "r_"), unmixed(mine, "cytiq_"),
                         unmixed(with_r_af, "cytiq_r_af_"), check.names = FALSE),
              file.path(dir, paste0("r_unmixed_", tools::file_path_sans_ext(basename(fcs)), ".csv")),
              row.names = FALSE)
  }
}
writeLines(capture.output(sessionInfo()), file.path(out_dir, "sessionInfo.txt"))
