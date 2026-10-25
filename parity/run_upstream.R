#!/usr/bin/env Rscript
# Run the upstream R tools on the synthetic parity datasets.
# Usage: Rscript parity/run_upstream.R <data_dir> <out_dir> [tool ...]
# R_LIBS_USER must point at parity/.env/Rlib (see parity/setup_env.sh).

suppressPackageStartupMessages({
  library(flowCore)
})

args <- commandArgs(trailingOnly = TRUE)
data_dir <- normalizePath(if (length(args) >= 1) args[[1]] else "parity/.data")
out_dir <- if (length(args) >= 2) args[[2]] else "parity/.out/upstream"
dir.create(out_dir, recursive = TRUE, showWarnings = FALSE)
out_dir <- normalizePath(out_dir)
tools <- if (length(args) >= 3) args[3:length(args)] else
  c("transform", "autospill", "peacoqc", "flowsom", "autospectral", "cytoscan", "pipeline")

timed <- function(name, expr) {
  started <- proc.time()[["elapsed"]]
  result <- force(expr)
  elapsed <- proc.time()[["elapsed"]] - started
  write(sprintf("%s,%.3f", name, elapsed), file.path(out_dir, "timings.csv"), append = TRUE)
  invisible(result)
}

# ------------------------------------------------------------------ transform
run_transform <- function() {
  ff <- read.FCS(file.path(data_dir, "transform", "grid.fcs"), transformation = FALSE, truncate_max_range = FALSE)
  x <- exprs(ff)[, "X"]
  out <- data.frame(x = x)
  out$arcsinh_cf150 <- asinh(x / 150)
  out$flowcore_arcsinh <- arcsinhTransform(a = 0, b = 1 / 150, c = 0)(x)
  lgcl <- logicleTransform(w = 0.5, t = 262144, m = 4.5, a = 0)
  out$flowcore_logicle <- lgcl(x)
  # GatingML / FlowJo-compatible logicle scaled to [0, 1] of M decades
  out$flowcore_logicle_unit <- out$flowcore_logicle / 4.5
  hlog <- hyperlogtGml2(parameters = "X", T = 262144, M = 4.5, W = 0.5, A = 0)
  out$flowcore_hyperlog_gml2 <- as.numeric(eval(hlog)(matrix(x, ncol = 1, dimnames = list(NULL, "X"))))
  bex <- flowWorkspace::flowjo_biexp(channelRange = 4096, maxValue = 262144, pos = 4.5, neg = 0, widthBasis = -10)
  out$flowjo_biexp_4096 <- bex(x)
  bex_as <- flowWorkspace::flowjo_biexp(channelRange = 256, maxValue = 262144, pos = 4.418539922, neg = 0, widthBasis = -100)
  out$flowjo_biexp_autospill <- bex_as(x)
  write.csv(out, file.path(out_dir, "transform.csv"), row.names = FALSE)
}

# ------------------------------------------------------------------ autospill
run_autospill <- function() {
  suppressPackageStartupMessages(library(autospill))
  work <- file.path(out_dir, "autospill_work")
  dir.create(work, recursive = TRUE, showWarnings = FALSE)
  old <- setwd(work)
  on.exit(setwd(old))
  asp <- get.autospill.param("minimal")
  asp$worker.process.n <- 1
  asp$verbose <- FALSE
  control_dir <- file.path(data_dir, "autospill")
  timed("autospill", {
    flow.control <- read.flow.control(control_dir, file.path(control_dir, "controls_full.csv"), asp)
    flow.gate <- gate.flow.data(flow.control, asp)
    initial <- get.marker.spillover(TRUE, flow.gate, flow.control, asp)
    refined <- refine.spillover(initial, NULL, flow.gate, flow.control, asp)
  })
  spill <- refined[[1]]
  write.csv(spill, file.path(out_dir, "autospill_spillover.csv"))
  write.csv(initial, file.path(out_dir, "autospill_spillover_initial.csv"))
  gates <- do.call(rbind, lapply(names(flow.gate), function(s) {
    ids <- flow.gate[[s]]
    if (is.character(ids)) ids <- as.integer(sub(".*\\.", "", ids))
    data.frame(sample = s, event = as.integer(ids))
  }))
  write.csv(gates, file.path(out_dir, "autospill_gates.csv"), row.names = FALSE)
}

# ------------------------------------------------------------------ peacoqc
run_peacoqc <- function() {
  suppressPackageStartupMessages(library(PeacoQC))
  ff <- read.FCS(file.path(data_dir, "peacoqc", "sample.fcs"), transformation = FALSE, truncate_max_range = FALSE)
  channels <- strsplit(readLines(file.path(data_dir, "peacoqc", "channels.txt"), warn = FALSE), ",")[[1]]
  for (method in c("all", "MAD", "IT")) {
    res <- timed(paste0("peacoqc_", tolower(method)), PeacoQC(
      ff, channels = channels, determine_good_cells = method, plot = FALSE,
      save_fcs = FALSE, report = FALSE, output_directory = NULL,
      MAD = 6, IT_limit = 0.6, consecutive_bins = 5, remove_zeros = FALSE,
      time_channel_parameter = "Time", suppress_message = TRUE))
    write.csv(data.frame(good = as.integer(res$GoodCells)),
              file.path(out_dir, paste0("peacoqc_", tolower(method), "_mask.csv")), row.names = FALSE)
    write(sprintf("%s,%d,%d", method, res$EventsPerBin, length(res$GoodCells)),
          file.path(out_dir, "peacoqc_meta.csv"), append = TRUE)
  }
}

# ------------------------------------------------------------------ flowsom
run_flowsom <- function() {
  suppressPackageStartupMessages(library(FlowSOM))
  for (dataset in c("flowsom", "flowsom_hard")) {
    ff <- read.FCS(file.path(data_dir, dataset, "sample.fcs"), transformation = FALSE, truncate_max_range = FALSE)
    channels <- strsplit(readLines(file.path(data_dir, dataset, "channels.txt"), warn = FALSE), ",")[[1]]
    for (scale in c(FALSE, TRUE)) {
      for (seed in 1:3) {
        tag <- sprintf("%s_scale%s_seed%d", dataset, if (scale) "T" else "F", seed)
        fsom <- timed(tag, FlowSOM(ff, colsToUse = channels, xdim = 10, ydim = 10, nClus = 20,
                                   rlen = 10, scale = scale, seed = seed, silent = TRUE))
        write.csv(data.frame(node = GetClusters(fsom), metacluster = as.integer(GetMetaclusters(fsom))),
                  file.path(out_dir, paste0(tag, ".csv")), row.names = FALSE)
      }
    }
  }
}

# ------------------------------------------------------------------ autospectral
# AutoSpectral's control pipeline (define.flow.control) needs cytometer-specific
# metadata, so the upstream reference here is its unmixing core applied to the
# known spectra and to the spectra cytiq extracted.
run_autospectral <- function() {
  suppressPackageStartupMessages(library(AutoSpectral))
  ff <- read.FCS(file.path(data_dir, "autospectral", "sample.fcs"), transformation = FALSE, truncate_max_range = FALSE)
  truth <- as.matrix(read.csv(file.path(data_dir, "autospectral", "truth", "spectra.csv"), row.names = 1, check.names = FALSE))
  raw <- exprs(ff)[, colnames(truth)]
  unmixed <- timed("autospectral_unmix_ols", unmix.ols(raw, truth))
  write.csv(unmixed, file.path(out_dir, "autospectral_unmix_truth_spectra.csv"), row.names = FALSE)
  # Cross-check: upstream OLS applied to the spectra cytiq extracted.
  rust_dir <- Sys.getenv("PARITY_RUST_OUT", file.path(dirname(out_dir), "rust"))
  rust_spectra_file <- file.path(rust_dir, "autospectral", "references", "fluorophore_spectra.csv")
  if (file.exists(rust_spectra_file)) {
    rust <- read.csv(rust_spectra_file, check.names = FALSE)
    rust_spectra <- as.matrix(rust[, colnames(truth)])
    rownames(rust_spectra) <- rust$component
    unmixed_rust <- unmix.ols(raw, rust_spectra)
    write.csv(unmixed_rust, file.path(out_dir, "autospectral_unmix_rust_spectra.csv"), row.names = FALSE)
  }
}

# ------------------------------------------------------------------ cytoscan
# Upstream CytoScan (AUMC-HEMA/CytoScan): features via generateFeatures(),
# outliers via isotree (seed 42 plus 43-44 for the isotree noise band), novelty
# via mclust::Mclust (default model search and VVV-only, which is what
# linfa's full-covariance GMM corresponds to).
cytoscan_features <- function(dataset_dir, method, seed = 42) {
  suppressPackageStartupMessages(library(CytoScan))
  if (seed != 42) {
    # noise band for the per-file subsample: same readInput with another seed
    patched <- function(CS, path, n = NULL) {
      set.seed(seed)
      ff <- flowCore::read.FCS(path, which.lines = n, truncate_max_range = CS$truncate_max_range)
      CS[["preprocessFunction"]](ff)
    }
    original <- get("readInput", envir = asNamespace("CytoScan"))
    utils::assignInNamespace("readInput", patched, ns = "CytoScan")
    on.exit(utils::assignInNamespace("readInput", original, ns = "CytoScan"), add = TRUE)
  }
  channels <- strsplit(readLines(file.path(dataset_dir, "channels.txt"), warn = FALSE), ",")[[1]]
  test <- sort(list.files(file.path(dataset_dir, "test"), full.names = TRUE))
  ref <- sort(list.files(file.path(dataset_dir, "reference"), full.names = TRUE))
  CS <- CytoScan()
  CS <- addTestdata(CS, test)
  CS <- addReferencedata(CS, ref)
  CS <- suppressMessages(generateFeatures(CS, channels, featMethod = method, cores = 1))
  CS
}

write_features <- function(features, path) {
  out <- data.frame(filename = basename(rownames(features)), features, check.names = FALSE)
  write.csv(out, path, row.names = FALSE)
}

# CytoScan's novelty model (Mclust defaults, plus Mclust(modelNames = "VVV")):
# `ll_*` as CytoScan computes it, log(dens()), which is -Inf where dens()
# underflows; `logll_*` the same in log space, dens(logarithm = TRUE).
write_novelty <- function(ref_features, test_features, target, tag) {
  novelty <- data.frame(filename = basename(rownames(test_features)))
  meta <- list()
  for (models in list(default = NULL, VVV = "VVV")) {
    key <- if (is.null(models)) "default" else models
    gmm <- tryCatch(suppressWarnings(Mclust(ref_features, modelNames = models, verbose = FALSE)),
                    error = function(e) NULL)
    if (is.null(gmm)) {
      novelty[[paste0("ll_", key)]] <- NA
      novelty[[paste0("logll_", key)]] <- NA
      meta[[key]] <- list(model = NA, G = NA, threshold = NA)
      next
    }
    ll_train <- log(dens(ref_features, gmm$modelName, parameters = gmm$parameters))
    threshold <- unname(quantile(ll_train, 0.05))
    novelty[[paste0("ll_", key)]] <- log(dens(test_features, gmm$modelName, parameters = gmm$parameters))
    novelty[[paste0("logll_", key)]] <- dens(test_features, gmm$modelName, parameters = gmm$parameters,
                                             logarithm = TRUE)
    meta[[key]] <- list(model = gmm$modelName, G = gmm$G, bic = gmm$bic, threshold = threshold,
                        train_ll = ll_train)
  }
  write.csv(novelty, file.path(target, paste0("novelty_", tag, ".csv")), row.names = FALSE)
  jsonlite::write_json(meta, file.path(target, paste0("novelty_", tag, ".json")),
                       auto_unbox = TRUE, digits = NA)
}

read_rust_features <- function(path) {
  table <- read.csv(path, check.names = FALSE)
  features <- as.matrix(table[, setdiff(names(table), c("filename", "path")), drop = FALSE])
  rownames(features) <- table$filename
  features
}

run_cytoscan <- function() {
  suppressPackageStartupMessages({ library(CytoScan); library(mclust) })
  rust_dir <- Sys.getenv("PARITY_RUST_OUT", file.path(dirname(out_dir), "rust"))
  for (dataset in c("cytoscan_exact", "cytoscan", "cytoscan_ref100")) {
    dataset_dir <- file.path(data_dir, dataset)
    if (!dir.exists(dataset_dir)) next
    target <- file.path(out_dir, dataset)
    dir.create(target, recursive = TRUE, showWarnings = FALSE)
    for (method in c("summary", "quantiles", "EMD", "binning", "fingerprint")) {
      tag <- tolower(method)
      CS <- timed(paste0(dataset, "_features_", tag), cytoscan_features(dataset_dir, method))
      test_features <- CS$features$test[[method]]
      ref_features <- CS$features$reference[[method]]
      write_features(test_features, file.path(target, paste0("features_test_", tag, ".csv")))
      write_features(ref_features, file.path(target, paste0("features_reference_", tag, ".csv")))

      scores <- sapply(c(42, 1042, 2042), function(seed) {
        forest <- isotree::isolation.forest(test_features, sample_size = 1, ntrees = 1000,
                                            ndim = 1, seed = seed, nthreads = 1)
        stats::predict(forest, test_features)
      })
      colnames(scores) <- paste0("score_seed", c(42, 1042, 2042))
      outlier <- data.frame(filename = basename(rownames(test_features)), scores, check.names = FALSE)
      write.csv(outlier, file.path(target, paste0("outlier_", tag, ".csv")), row.names = FALSE)

      write_novelty(ref_features, test_features, target, tag)
      # Mclust on cytiq's own features: the model choice without the
      # subsample difference (cytiq draws with `rand`, not R's RNG).
      rust_reference <- file.path(rust_dir, paste0(dataset, "_debug"), tag, "reference_features.csv")
      rust_test <- file.path(rust_dir, dataset, tag, "cytoscan_features.csv")
      if (file.exists(rust_reference) && file.exists(rust_test)) {
        write_novelty(read_rust_features(rust_reference), read_rust_features(rust_test), target,
                      paste0("rust_features_", tag))
      }
    }
    if (dataset == "cytoscan") {
      for (seed in c(1, 2)) {
        for (method in c("summary", "quantiles", "EMD", "binning", "fingerprint")) {
          CS <- cytoscan_features(dataset_dir, method, seed = seed)
          write_features(CS$features$test[[method]],
                         file.path(target, sprintf("features_test_%s_seed%d.csv", tolower(method), seed)))
        }
      }
    }
  }
}

source(file.path(dirname(sub("^--file=", "", grep("^--file=", commandArgs(FALSE), value = TRUE))),
                 "pipeline.R"))

for (tool in tools) {
  message("== ", tool)
  get(paste0("run_", tool))()
}
writeLines(capture.output(sessionInfo()), file.path(out_dir, "sessionInfo.txt"))
