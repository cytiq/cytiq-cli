#!/usr/bin/env bash
# Install the upstream reference tools into an isolated environment under
# parity/.env (git-ignored): an R library and a uv-managed Python venv.
#
# Pinned: the Bioconductor release (BIOC_VERSION), the git commits of the
# packages built from source, and the Python packages (parity/requirements.txt).
# CRAN packages come from PARITY_CRAN (default: the CRAN cloud mirror); the
# versions actually installed are written to parity/versions.txt.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_DIR="$ROOT/parity/.env"
export R_LIBS_USER="$ENV_DIR/Rlib"
export BIOC_VERSION="${BIOC_VERSION:-3.22}"
export PARITY_CRAN="${PARITY_CRAN:-https://cloud.r-project.org}"
mkdir -p "$R_LIBS_USER"

Rscript - <<'R'
lib <- Sys.getenv("R_LIBS_USER")
.libPaths(c(lib, .libPaths()))
options(repos = c(CRAN = Sys.getenv("PARITY_CRAN")), Ncpus = parallel::detectCores())
if (!requireNamespace("BiocManager", quietly = TRUE)) install.packages("BiocManager", lib = lib)
BiocManager::install(version = Sys.getenv("BIOC_VERSION"), lib = lib, ask = FALSE, update = FALSE)
BiocManager::install(c("flowCore", "flowWorkspace", "FlowSOM", "PeacoQC", "ConsensusClusterPlus",
                       "remotes", "MASS", "deldir", "flowFP", "isotree", "mclust", "transport",
                       "doParallel", "foreach", "ggridges", "ggrepel", "gridExtra", "dplyr",
                       "jsonlite", "flowDensity"), lib = lib, ask = FALSE, update = FALSE)
R

# Packages built from git, at the commits the parity report was produced with.
for pin in carlosproca/autospill@1e60e86337b297f1dd9ffec6701d8010dd06175b \
  DrCytometer/AutoSpectral@292dfeaae6b382d1a7c9c263c5bb4de35ff67207 \
  AUMC-HEMA/CytoScan@c97f403d3ad6c673bed67f1f7dff4062e2f28e19 \
  Olsman/CompensAID@db748036e977571e09ddb4267ca598a1d700f2fc; do
  repo="${pin%@*}"
  commit="${pin#*@}"
  name="${repo#*/}"
  src="$ENV_DIR/$name-src"
  if [[ ! -d "$src/.git" ]]; then
    git init -q "$src"
    git -C "$src" remote add origin "https://github.com/$repo.git"
  fi
  if [[ "$(git -C "$src" rev-parse -q --verify HEAD || true)" != "$commit" ]]; then
    git -C "$src" fetch -q --depth 1 origin "$commit"
    git -C "$src" checkout -q --detach FETCH_HEAD
  fi
  # Dependencies come from the pinned Bioconductor release. remotes would also
  # resolve the DESCRIPTION's Remotes field through the GitHub API, which
  # rate-limits unauthenticated CI runners.
  Rscript - "$src" <<'R'
lib <- Sys.getenv("R_LIBS_USER")
.libPaths(c(lib, .libPaths()))
options(repos = c(CRAN = Sys.getenv("PARITY_CRAN")), Ncpus = parallel::detectCores())
src <- commandArgs(trailingOnly = TRUE)[1]
deps <- remotes::local_package_deps(src, dependencies = c("Depends", "Imports", "LinkingTo"))
missing <- setdiff(deps, c(rownames(installed.packages()), "R"))
if (length(missing) > 0) BiocManager::install(missing, lib = lib, ask = FALSE, update = FALSE)
R
  R CMD INSTALL --library="$R_LIBS_USER" "$src"
done

uv venv -q --allow-existing "$ENV_DIR/py" --python 3.12
VIRTUAL_ENV="$ENV_DIR/py" uv pip install -q -r "$ROOT/parity/requirements.txt"

{
  Rscript -e ".libPaths(c(Sys.getenv('R_LIBS_USER'), .libPaths()));
    for (p in c('flowCore','flowWorkspace','FlowSOM','PeacoQC','autospill','AutoSpectral','MASS',
                'CytoScan','flowFP','isotree','mclust','transport','flowDensity','CompensAID'))
      cat(p, as.character(packageVersion(p)), '\n')"
  R --version | head -1
  VIRTUAL_ENV="$ENV_DIR/py" uv pip freeze | grep -Ei '^(flowkit|flowio|numpy|pandas|pyarrow|scikit-learn|scipy)=='
  for name in autospill AutoSpectral CytoScan CompensAID; do
    echo "$name git $(git -C "$ENV_DIR/$name-src" rev-parse --short HEAD)"
  done
  rustc --version
} > "$ROOT/parity/versions.txt"
cat "$ROOT/parity/versions.txt"
