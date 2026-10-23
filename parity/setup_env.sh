#!/usr/bin/env bash
# Install the upstream reference tools into an isolated environment under
# parity/.env (git-ignored): an R library and a uv-managed Python venv.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_DIR="$ROOT/parity/.env"
export R_LIBS_USER="$ENV_DIR/Rlib"
mkdir -p "$R_LIBS_USER"

Rscript - <<'R'
lib <- Sys.getenv("R_LIBS_USER")
.libPaths(c(lib, .libPaths()))
options(repos = c(CRAN = "https://cloud.r-project.org"), Ncpus = 8)
if (!requireNamespace("BiocManager", quietly = TRUE)) install.packages("BiocManager", lib = lib)
BiocManager::install(c("flowCore", "flowWorkspace", "FlowSOM", "PeacoQC", "ConsensusClusterPlus",
                       "remotes", "MASS", "deldir", "flowFP", "isotree", "mclust", "transport",
                       "doParallel", "foreach", "ggridges", "ggrepel", "gridExtra", "dplyr",
                       "jsonlite"), lib = lib, ask = FALSE, update = FALSE)
R

for repo in carlosproca/autospill DrCytometer/AutoSpectral AUMC-HEMA/CytoScan; do
  name="${repo#*/}"
  [[ -d "$ENV_DIR/$name-src" ]] || git clone -q --depth 1 "https://github.com/$repo.git" "$ENV_DIR/$name-src"
  Rscript -e ".libPaths(c(Sys.getenv('R_LIBS_USER'), .libPaths()));
    remotes::install_local('$ENV_DIR/$name-src', lib = '$R_LIBS_USER', upgrade = 'never',
                           dependencies = c('Depends', 'Imports', 'LinkingTo'))"
done

uv venv -q --allow-existing "$ENV_DIR/py" --python 3.12
VIRTUAL_ENV="$ENV_DIR/py" uv pip install -q numpy pandas pyarrow scipy scikit-learn flowkit flowio

{
  Rscript -e ".libPaths(c(Sys.getenv('R_LIBS_USER'), .libPaths()));
    for (p in c('flowCore','flowWorkspace','FlowSOM','PeacoQC','autospill','AutoSpectral','MASS',
                'CytoScan','flowFP','isotree','mclust','transport'))
      cat(p, as.character(packageVersion(p)), '\n')"
  R --version | head -1
  VIRTUAL_ENV="$ENV_DIR/py" uv pip freeze | grep -Ei '^(flowkit|flowio|numpy|pandas|pyarrow|scikit-learn|scipy)=='
  for name in autospill AutoSpectral CytoScan; do
    echo "$name git $(git -C "$ENV_DIR/$name-src" rev-parse --short HEAD)"
  done
  rustc --version
} > "$ROOT/parity/versions.txt"
cat "$ROOT/parity/versions.txt"
