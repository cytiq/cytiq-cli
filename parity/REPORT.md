# Parity report: cytiq (Rust) vs the original tools

Date: 2026-10-22 · branch `parity-fixes` · baseline `main` (`0f4c72a`)
Updated 2026-10-24 (Stage 3): end-to-end chain (§2.9), real acquisitions (§2.10)
and pass/fail tolerances (§1, step 5).

## TL;DR

| Tool | Upstream | Before fixes | After fixes |
|---|---|---|---|
| `transform` arcsinh | flowCore `arcsinhTransform` | ✅ matches (Δ ≤ 4e-7, f32 precision) | ✅ unchanged |
| `transform` logicle | flowCore `logicleTransform` | ❌ different function (Δ = 0.93 over a 5.5 range) | ✅ Δ ≤ 2.4e-7 |
| `transform` hyperlog | flowCore `hyperlogtGml2`, FlowKit | ❌ different function (Δ = 3.0 over a 1.2 range) | ✅ Δ ≤ 3.4e-8 |
| `transform` biexponential | flowWorkspace `flowjo_biexp` | ❌ different function, f(−1000) = −420 instead of 10.3 | ✅ Δ ≤ 7.6e-6 channels |
| `autospill` | autospill 0.2.0 | ⚠️ matrix close (max Δ 3.8e-4), gates 18% wider | ✅ matrix Δ ≤ 4.8e-13, identical gates |
| `autospread` | — (no upstream) | — | — (regression test only) |
| `autospectral` | AutoSpectral 1.8.7 (OLS) | ❌ abundances inflated 1.39–1.60× (L2 normalisation) | ✅ scale 0.992–1.0003, RMSE equal to upstream |
| `peacoqc` | PeacoQC 1.20.0 | ❌ 82–85% mask agreement, IT precision 0.04; 1.5× slower than R | ✅ 100% mask agreement in every mode; 11× faster than R |
| `cytoscan` | CytoScan 0.1.0 (`c97f403`) | ❌ binning off by up to 0.004, sample depends on path spelling, outlier scores 3× outside isotree noise, novelty model unlike mclust | ✅ features = R to 5e-13 on identical events; outlier flags = R; novelty reproduces mclust's model choice when it is full-covariance |
| `flowsom` | FlowSOM 2.18.0 | ⚠️ ARI vs R 0.87–0.91 (hard set), 9× slower than R | ✅ ARI vs R 0.93–0.98 (R's own seed-to-seed level), ~8× faster than R |
| `gate` | FlowKit 1.3.0 `GatingStrategy.gate_sample` | — (new command) | ✅ identical per-event membership for all 11 gates; identical counts at 1M events, ~13× faster than FlowKit |
| whole chain (§2.9) | flowCore + PeacoQC + FlowKit + FlowSOM, same steps | — (new) | ✅ every stage equals upstream per event on 200,000 golden events (values Δ ≤ 1e-7, masks and gates identical, FlowSOM ARI vs R 0.998); against the truth: final mask F1 0.967, every population incl. a 0.2% rare one found with precision ≥ 0.999 |
| real data (§2.10) | flowCore, PeacoQC, FlowKit, FlowSOM, FlowJo labels | — (new) | ✅ compensation/logicle Δ ≤ 8e-8, PeacoQC identical, manual Gating-ML identical to FlowKit on all 15 gates; populations match FlowJo with Jaccard 0.98–0.996 (NK T 0.86, a FlowJo export effect) |

**44 deviations** from the upstream algorithms were found and fixed (8 of them in `cytoscan`), plus
two performance defects (section 2.6). The Stage 3 chain validation found two
more defects, both fixed: raw margin masks removed every event at a channel's
observed extremes (half of a bead sample), and FCS output lacked HEADER DATA
offsets, so strict readers such as FlowKit rejected it. All key
ports (`stats::density`, `smooth.spline`, logicle, hyperlog, `flowjo_biexp`)
are pinned by unit tests against reference values produced by R.

## 1. Method

1. **Static review.** Each crate was read line by line and compared with the
   upstream sources of the installed packages (`autospill`, `MASS`,
   `FlowSOM` + `src/som.c`, `PeacoQC`, `flowCore`, `flowWorkspace`/`cytolib`,
   `AutoSpectral`, `stats::density`/`smooth.spline` from `r-source`).
2. **Synthetic data with known ground truth** (`parity/gen/generate.py`,
   fixed seed `20261022`):

   | Dataset | Contents | Ground truth |
   |---|---|---|
   | `transform` | grid of 999 values from −1000 to 262144 | — |
   | `autospill` | 6 single-colour controls × 20,000 events, 15% debris, Poisson + electronic noise | 6×6 spillover matrix, debris mask |
   | `autospectral` | 16 detectors, 6 fluorophores + AF, 20,000-event controls, 30,000-event sample | spectra (peak = 1), per-event abundances |
   | `peacoqc` | 200,000 events, 6 channels, clog (4%), drift (6%), burst (1%) | bad-event mask (9.8%) |
   | `flowsom` | 100,000 × 10D, 15 well-separated populations | labels |
   | `flowsom_hard` | 100,000 × 10D, 20 overlapping populations, rare ones (≥0.2%) | labels |
   | `pipeline` (`gen/pipeline.py`, seed `20261024`) | 200,000 raw events, 6×6 spillover, 7 populations (one 0.3%), debris, doublets, dead cells, an unstable interval, saturation | per-event population and artefact flags |
   | `pipeline_beads` | 3,000 events of one discrete bead population (zero-MAD scatter) | — |

3. **Runs.** Rust (`parity/run_rust.sh`) for the current branch and for `main`
   as the baseline; the upstream R tools (`parity/run_upstream.R`) with the
   same parameters. FlowSOM in R runs with `nClus = 20`, `scale = FALSE/TRUE`,
   seeds 1–3 (R's RNG cannot be reproduced in Rust, so the comparison is
   statistical).
4. **Metrics** (`parity/compare.py` → `parity/results/*.json`): max |Δ| for
   transforms; MAE/max |Δ|/Frobenius for matrices; RMSE/scale/correlation for
   unmixing; precision/recall/Jaccard/agreement for QC masks; ARI/NMI/purity
   for clustering; wall time and peak memory (`/usr/bin/time -l`).
5. **Tolerances.** `parity/tolerances.json` (and `parity/real/tolerances.json`
   for §2.10) turns the results into pass/fail rules, each with its reason;
   `compare.py` exits with 1 on any violation or on a rule that matches no
   metric, so `parity/run_all.sh` fails. The same chain also runs in CI on a
   15,000-event golden fixture (`crates/cytiq-cli/tests/pipeline_cli.rs`,
   thresholds in `data/fixtures/pipeline/thresholds.json`).

Environment (`parity/versions.txt`): R 4.5.3, flowCore 2.22.1, flowWorkspace
4.22.1, FlowSOM 2.18.0, PeacoQC 1.20.0, autospill 0.2.0 (`1e60e86`),
AutoSpectral 1.8.7 (`292dfea`), MASS 7.3.66, FlowKit 1.3.0, rustc 1.99.1.
Apple Silicon, macOS.

## 2. Results by tool

### 2.1 `transform`

Maximum deviation of Rust from the reference over the 999-point grid:

| Method | Reference | Before | After |
|---|---|---|---|
| arcsinh (cofactor 150) | flowCore `arcsinhTransform(b = 1/150)` | 4.4e-7 | 4.4e-7 |
| logicle (T=262144, W=0.5, M=4.5, A=0) | flowCore `logicleTransform` (0..M scale) | **0.934** | 2.4e-7 |
| logicle | FlowKit / Gating-ML (0..1 scale) | 3.30 | ×M of flowCore; 2e-7 after affine alignment |
| hyperlog | flowCore `hyperlogtGml2` | **3.00** | 2.9e-8 |
| hyperlog | FlowKit `HyperlogTransform` | 3.00 | 3.4e-8 |
| biexponential | flowWorkspace `flowjo_biexp(256, 262144, 4.4185, 0, −100)` | **430** | 7.6e-6 |

Residual deviations of ~1e-7 come from writing FCS as float32.

**What was wrong.**
- Logicle was `asinh(x/s)/ln10` instead of the Parks logicle: f(0) = 0
  instead of 0.5, f(T) = 4.30 instead of 4.5.
- Parameter A compressed the negative range instead of adding decades.
- Hyperlog was a signed `log10(1 + |x|/s)`.
- Biexponential was a two-sided log1p whose negative-branch gain was 4×
  the positive one: f(−1000) = −419.8, f(+1000) = 106.2. Output went
  negative, whereas FlowJo output lies in [0, 256] with 0 at channel 57.

**Fix.** New module `crates/cytiq-transform/src/scales.rs`:
- `Logicle`: Moore–Parks 2012, with an inverse and validation of the
  constraints `2W ≤ M`, `−W ≤ A ≤ M − 2W`.
- `Hyperlog`: Gating-ML.
- `FlowJoBiexp`: a port of `cytolib::biexpTrans::computCalTbl`, including
  the float divisions and `abs((int64_t)…)` in `logRoot`, plus R's natural
  spline (`natural_spline`/`spline_eval`).

### 2.2 `autospill`

| Metric | Before | After | R autospill |
|---|---|---|---|
| Spillover vs truth, MAE | 1.68e-4 | 5.45e-5 | 5.45e-5 |
| Spillover vs truth, max \|Δ\| | 4.82e-4 | 2.17e-4 | 2.17e-4 |
| Rust vs R, max \|Δ\| | 3.83e-4 | **4.8e-13** | — |
| Rust vs R, ‖ΔC‖_F (compensation) | 9.7e-4 | 1.6e-12 | — |
| Gated events, BV421 (of 17,000 non-debris) | 13,954 | 11,837 | 11,837 |
| Refinement iterations | 5 | 3 | — |

After the fixes the gates of all 6 controls match R event for event. Neither
Rust nor R lets any debris into the gate.

**Fixed:**
1. KDE kernel width: σ = h, but `MASS::kde2d` uses h/4. The kernel was 4×
   too wide and the density oversmoothed (`gating.rs`).
2. The KDE grid spanned the trim/MAD box instead of the range of the points
   passed in.
3. FSC/SSC were not clamped to [0, max(`$P1R`)] before trimming.
4. When every peak lay in the lower-left corner, Rust silently took peak 0,
   while R stops with an error.
5. Stale `delta_change` in the refinement state machine: R resets it to −1
   when switching linear → bi-exp and λ 1 → 0.1. Without the reset, a plateau
   on the linear scale produced `NonConvergent` (`runner.rs`).
6. The refinement bi-exp transform is now `flowjo_biexp`, as in
   `autospill::get.transformation`.
7. The `rlm` scale is now `median(|r|)/0.6745` (as in MASS) instead of a
   centred MAD.
8. `rlm` convergence: now the `irls.delta ≤ 1e-4` criterion, falling back
   to OLS when it does not converge, as in `fit.robust.linear.model`.
9. Linear-stage skewness is computed on the trimmed data.
10. `trim_n` now rounds half to even.

**AutoSpread.** Neither `autospill` nor `AutoSpectral` computes a spread
matrix: AutoSpectral only consumes one, in σ²/F units and in
fluorophore × fluorophore layout. The Rust implementation is therefore a
Nguyen-2013-style heuristic without a reference. After the gate fix the
coefficients on the synthetic set moved by 4% at the median and 24% at most.
The golden file `data/fixtures/autospread/expected_spread.csv` was
regenerated.

### 2.3 `autospectral`

| Fluorophore | Scale vs truth: before | after | RMSE: before | after | RMSE: R `unmix.ols` (true spectra) |
|---|---|---|---|---|---|
| BV421 | 1.396 | 0.992 | 6217 | 862 | 860 |
| BV510 | 1.548 | 1.000 | 8553 | 341 | 339 |
| FITC | 1.439 | 1.000 | 6710 | 263 | 263 |
| PE | 1.540 | 1.000 | 8330 | 130 | 130 |
| PE-Cy5 | 1.604 | 1.000 | 9275 | 119 | 119 |
| APC | 1.393 | 1.000 | 5993 | 89 | 89 |

- Spectrum shapes matched the truth even before the fixes (cos = 1.0, max
  |Δ| after peak normalisation ≤ 5e-4). Only the scale was wrong.
- Independent solver check: R `unmix.ols` applied to the spectra extracted
  by Rust gives the same abundances as Rust (relative |Δ| ≤ 5e-8).

**Fixed:**
- L2 spectrum normalisation replaced by peak (L∞) normalisation, as in
  `get.fluorophore.spectra`. The AF spectrum is divided by `max |x|`.
- Positive events are now selected as in R: above the 99th percentile of
  the negative (AF) control on the primary channel, top 500, at least 20.
  Previously the top 20% of the control itself was used.
- Quantiles are now R type 7 instead of nearest rank.
- The `max(0)` background clamp was removed.
- A warning is emitted when the condition number (fluorophores only)
  exceeds the number of fluorophores, as in `calculate.condition.number`.

### 2.4 `peacoqc`

| Mode | Metric | Before | After | R PeacoQC |
|---|---|---|---|---|
| all | fraction removed | 27.8% | 20.5% | 20.5% |
| all | mask agreement with R / Jaccard of removed | 83.8% / 0.50 | **100% / 1.00** | — |
| MAD | agreement / Jaccard | 85.3% / 0.50 | **100% / 1.00** | — |
| IT | precision vs truth | **0.036** | 1.00 | 1.00 |
| IT | agreement / Jaccard | 82.3% / 0.00 | **100% / 1.00** | — |
| — | number of bins | 399 | 400 | 400 |
| — | time, 200k events × 6 channels | 2.30 s | **0.14 s** | 1.55 s |

Recall of bad events is the same for Rust and R: 1.0 in `all` mode.
Precision in `all` mode is 0.48 for both: upstream with `MAD = 6` is
aggressive on this synthetic set.

**Fixed:**
1. The peak detector was a histogram with [1,2,1] smoothing. It is now a
   port of `stats::density`: bw.nrd0, `BinDist`, convolution, `approx`.
   Matches R to 1e-9.
2. MAD smoothing was a 5-point moving average. It is now a port of
   `smooth.spline(spar = 0.5)`, including `.nknots.smspl` and the `0.3330`
   constant from `sgram.f`. Matches R to 1e-6.
3. `SplitWithOverlap` dropped the last bin.
4. IT picked the last column whose gain exceeded the limit instead of the
   best one.
5. On equal leaf sizes IT took the last leaf, while `which.max` takes the
   first.
6. `RemoveShortRegions` did not remove short good runs at the file edges.
7. `RemoveMargins`: the `max(0, …)` lower threshold removed every event ≤ 0
   in compensated data; the upper threshold was `$PnR` instead of
   `$PnR − 1`.
8. `FindEventsPerBin` used integer division with `remove_zeros`.
9. The single-peak cluster centre was not computed over all peaks.
10. The `TooSmallClusters` denominator was wrong.

The CLI test fixture of 32 events (a single bin) was expanded to 1,600
events. Under R semantics a file with fewer than 5 bins is removed entirely.

### 2.5 `flowsom`

| Dataset | Metric | Before | After | R FlowSOM (3 seeds × scale F/T) |
|---|---|---|---|---|
| flowsom (easy) | ARI vs truth | 0.987 | 1.000 | 0.9997–1.000 |
| flowsom_hard | ARI vs truth | **0.885** | 0.953 | 0.948–0.966 |
| flowsom_hard | NMI vs truth | 0.903 | 0.961 | 0.949–0.963 |
| flowsom_hard | ARI Rust vs R | 0.87–0.91 | **0.93–0.98** | R seed-to-seed: 0.93–0.95 |
| — | time, 100k × 10D | **36.7 s** | 0.52 s | 4.0–4.4 s |

After the fixes Rust differs from R no more than R differs from itself when
the seed changes.

**Fixed (checked against `som.c`):**
- Radius: 5 → 1 per epoch replaced by 6 → 0 per step (0.67 quantile of grid
  distances). Below 1 only the BMU is updated (threshold 0.5).
- Grid distance: Euclidean replaced by Chebyshev (`method = "maximum"`).
- Gaussian neighbourhood replaced by bubble: full α inside the radius.
- The α schedule is linear per step, not per epoch.
- Events are drawn at random with replacement (`rlen · n` steps) instead of
  in file order.
- Initial codes are sampled without replacement; the generator uses the top
  53 bits instead of `% n` on the LCG's low bits.
- The scaling SD is computed as in `base::scale` (n − 1).
- Removed the per-event rayon dispatch inside SOM training, which caused
  the ~60× slowdown.

**Deliberately unchanged** (documented in the README as fixed MVP choices):
- `scale = TRUE` by default (R: `FALSE`);
- 20 metaclusters (R: `nClus = 10`);
- metaclustering by plain average linkage instead of ConsensusClusterPlus.

This does not affect the metrics: Rust-vs-R ARI is in the same range for
scale T and F.

### 2.6 Performance

Wall time on the parity datasets (Apple Silicon, all cores):

| Command | Before (`main`) | After | Upstream R |
|---|---|---|---|
| `peacoqc` (200k events, mode `all`) | 2.30 s | **0.14 s** | 1.55 s |
| `flowsom` (100k × 10D) | 36.5 s | **0.52 s** | 4.0 s |
| `autospectral` (6 controls + 30k sample) | 0.53 s | **0.03 s** | — |
| `autospill` (6 controls × 20k) | 2.31 s | 2.19 s | 8.83 s |

**Fixed:**
1. **Unbuffered FCS writer** (`cytiq-fcs/src/writer.rs`). The data segment
   was written straight to `File` one float at a time: one `write` syscall
   per event and channel (2M syscalls for the PeacoQC output). Profiling
   showed ~70% of PeacoQC time in `write`, with sys time 2.4 s against user
   time 0.85 s, independent of `--threads`. Output now goes through a
   `BufWriter` with an explicit `flush`, so I/O errors still surface. This
   speeds up every command that writes FCS (`peacoqc`, `transform`,
   `autospectral`).
2. **`stats::density` used an O(n²) direct convolution**
   (`cytiq-peacoqc/src/density.rs`). It is now an FFT convolution
   (in-crate radix-2, 1024 points, no new dependency), the same
   `fft(y) * Conj(fft(kords))` scheme R uses. The R reference unit tests
   still pass at 1e-9, and the PeacoQC masks are still identical to R.
3. Per-event rayon dispatch inside FlowSOM SOM training (section 2.5).

The new Rust 1.99 toolchain (CI installs `@stable`) adds clippy lints that
pre-existing code in `cytiq-fcs` failed (`with_capacity(0)`, `chunks_exact(2)`
→ `as_chunks`). These were fixed so `clippy -D warnings` stays green.

### 2.7 `cytoscan`

Upstream: CytoScan 0.1.0 (`AUMC-HEMA/CytoScan@c97f403`) with isotree 0.6.1.5,
mclust 6.1.3, transport 0.15.4, flowFP 1.68.0. Baseline: the cytoscan branch
before these fixes (`7b1e2f6`).

**Datasets** (`parity/gen/cytoscan.py`):
- Cohorts follow `CytoScan::generateDemo`: 6 channels, each a two-normal
  mixture.
- Every file carries `$PnR` and a non-trivial `SPILL`.
- Anomalies were injected into known test files: gain shift, wrong `SPILL`,
  population loss, signal compression and debris.

| Dataset | Test / reference files | Events per file | Purpose |
|---|---|---|---|
| `cytoscan_exact` | 10 / 10 (3 anomalous) | 1,000 | Both tools use every event (`which.lines = 1000`, `nAgg = 1000`), so features compare without subsampling |
| `cytoscan` | 20 / 20 (5 anomalous) | 20,000 | Realistic cohort; subsamples differ, so the comparison is statistical |
| `cytoscan_ref100` | 20 / 100 (5 anomalous) | 20,000 | More reference files than features (the full-covariance regime) |

**Features.** Max |Δ| Rust vs R.

| Method | `cytoscan_exact` before | after | `cytoscan` mean \|Δ\| after | R subsample noise (seed 42 vs 1, 2) |
|---|---|---|---|---|
| summary | 5.0e-13 | 5.0e-13 | 0.103 | 0.095 |
| quantiles | 5.0e-13 | 5.0e-13 | 0.112 | 0.104 |
| EMD | 5.2e-13 | 5.2e-13 | 0.077 | 0.073 |
| binning | **4.4e-3** | 5.0e-13 | 0.011 | 0.011 |
| fingerprint | 0 | 0 | 0.021 | 0.021 |

- On identical events all five feature methods match upstream to rounding.
- On subsampled files the Rust–R difference equals R's own difference between
  subsample seeds. The draws come from `rand` rather than R's Mersenne
  Twister, so they are not bit-identical.

**Outlier (isotree).**
- Monte Carlo noise: isotree seeds tree `t` with `seed + t`, so seeds 42/43/44
  share 999 of 1000 trees. The noise band is therefore measured with
  independent seeds 42/1042/2042: 0.009–0.017 in score.

| Dataset | max \|Δscore\| before → after | isotree noise | flags agree with R | Rust TP/FP | R TP/FP |
|---|---|---|---|---|---|
| `cytoscan_exact` (5 methods) | 0.011–0.036 → **0.005–0.012** | 0.009–0.014 | 100% | 2–3 / 0 | 2–3 / 0 |
| `cytoscan` | 0.029–0.070 → 0.021–0.080 | 0.007–0.014 | 95–100% | 3–5 / 0 | 3–5 / 0 |
| `cytoscan_ref100` | 0.029–0.067 → 0.021–0.056 | 0.008–0.017 | 95–100% | 4–5 / 0 | 3–5 / 0–1 |

- On identical features the scores are now within isotree's own noise.
- On subsampled cohorts the larger Δ comes from the different feature values,
  not from the forest.

**Novelty (GMM).**
- The fit uses `linfa-clustering` (full covariance) with mclust's BIC over
  G = 1..9.
- Unit tests against `Mclust(modelNames = "VVV")` on fixed data:
  - 2-cluster data: same G, BIC within 1e-3, log-densities within 1e-3;
  - 1-cluster data: log-likelihood within 1e-6.
- On the parity cohorts:

| Dataset | Upstream model (`Mclust` default) | Rust model | Flag agreement with R | Rust TP/FP | R TP/FP |
|---|---|---|---|---|---|
| `cytoscan_exact` | fails for 4 of 5 methods (n_ref = 10 < d); EII for fingerprint | diagonal fallback, or G = 1 full for EMD | 100% where R fits | 3 / 6–7 | 3 / 7 |
| `cytoscan` (n_ref = 20) | EEI/VEI/VII/XII constrained models; XXX for fingerprint | diagonal fallback (d ≥ 20); G = 1 full for EMD and fingerprint | 50–100% | 5 / 4–15 | 5 / 3–15 |
| `cytoscan_ref100` | XXX (G = 1 full) for 4 of 5 methods; XII for EMD | G = 1 full for all 5 | 65–100% (95–100% when both are XXX) | 4–5 / 1–15 | 4–5 / 0–15 |

- Where mclust picks a full-covariance model, Rust picks the same model.
- The remaining Rust–R difference in those cases comes from the different
  subsamples: with d = 54 features and 100 files the covariance is close to
  singular and very sensitive to input.
- Upstream behaves the same way: full-covariance novelty on quantile or
  binning features flags 14–15 of 20 test files in R as well.
- The baseline's z-scored diagonal model flagged fewer files, but it did not
  follow upstream.

**Fixed:**
1. The subsample used a hand-written LCG seeded with `42 ^ FNV(path)`, so a
   relative and an absolute path gave different samples. It is now `rand`'s
   `StdRng` reseeded with 42 for every file, as `set.seed(42)` in `readInput`.
2. The aggregate was truncated to 10,000 events. It now takes
   `ceiling(10000 / n_files)` from every file, as `getAggregate` does.
3. Binning used `value <= boundary` and kept out-of-range values. It now
   follows `table(cut(x, breaks))`: right-closed bins, and values outside the
   aggregate range are dropped from counts and denominator.
4. Fingerprint is now a port of `flowFP`:
   - `dequantize` (event i gets i·1e-8);
   - one-pass variance for axis choice, first maximum from `DBL_MIN`;
   - median split, with ties going to the lower bin;
   - children numbered 2b / 2b+1;
   - an error on empty bins instead of an index-halving fallback.
5. Isolation forest:
   - `max_depth = ceil(log2 n)`; it was one deeper;
   - `c(n) = 2(H_n − 1)` with exact harmonic numbers; it was the logarithmic
     approximation;
   - column drawn uniformly among non-constant columns without replacement;
   - threshold from `rand_distr::Uniform`;
   - deterministic summation over rayon-parallel trees.
6. Novelty: the self-written z-scored diagonal EM (k ≤ 4) was replaced by
   `linfa-clustering` full-covariance mixtures. Features are not z-scored,
   as in upstream, and G is chosen by mclust's BIC (`2 loglik − npar ln n`)
   over G = 1..9. G values with components of ≤ d members are skipped. With
   no valid G it falls back to a single diagonal Gaussian and writes a
   warning.
7. `aggregate_events` in the diagnostics now reports the aggregate size,
   not the total number of events in all files.
8. The README stated the wrong preprocessing order. The code was already
   right: truncate to `$PnR` on read, compensate, then `asinh(x/150)`.

**Time.** Rust takes 0.04–0.3 s per run. R CytoScan takes 0.8–7.3 s for the
features alone.

### 2.8 `gate`

New command, so there is no baseline. The dataset is `parity/gen/gating.py`
(seed `20261023`): 4,000 events with `$SPILLOVER` and `$TIMESTEP`, and 11
events placed exactly on rectangle and polygon boundaries. The Gating-ML 2.0
strategy covers:
- polygon gates, one of them self-intersecting
- range, rectangle, ellipsoid and Boolean (`and`/`or`/`not`, complement) gates
- the `flin`, `flog`, `fasinh`, `logicle` and `hyperlog` transformations
- `FCS` compensation and spectrum-matrix compensation where the dimension is
  named by fluorochrome
- a four-level hierarchy

| Check | Result |
|---|---|
| Per-gate counts vs FlowKit (4,000 events) | 11 / 11 identical |
| Per-event membership vs FlowKit | identical for every gate (`crates/cytiq-gating/tests/flowkit_parity.rs`) |
| Per-gate counts vs FlowKit (same strategy, 1,000,000 events) | 11 / 11 identical |
| Wall time at 1M events | cytiq 0.17 s (includes the FCS read) vs FlowKit 2.2 s (includes `Sample` load) |

Semantics were taken from the FlowKit and flowutils sources, and the
flowutils C polygon routine was checked directly:
- rectangles use `min <= x < max`
- ellipsoids include their boundary
- polygons use Sunday's winding number with `wn % 2 != 0`. Points on
  left/bottom edges are inside and points on right/top edges are outside,
  despite the flowutils docstring, which says edges are inclusive
- raw events get `$PnE`, `$PnG` and `$TIMESTEP` pre-processing, as in flowio
- Boolean gates combine the parent-intersected membership of their references

### 2.9 End-to-end chain

The documented conventional chain runs on the golden `pipeline` data:
`preflight → compensate → transform (arcsinh 150) → peacoqc → clean (raw) →
mask combine → gate --mask → flowsom --mask`. Upstream runs the same steps
(`parity/pipeline.R`): `flowCore::compensate`, `arcsinhTransform`,
`PeacoQC::RemoveMargins` and `PeacoQC`, FlowSOM; FlowKit evaluates the
population strategy on the *raw* file with `FCS` compensation and `fasinh`
inside Gating-ML, and `clean`'s exported gates. Everything is joined per event
through `source_event_id`.

| Stage | cytiq vs upstream | cytiq vs truth |
|---|---|---|
| compensate | max relative Δ 6.0e-08 (f32 storage) | — |
| compensate → transform | max relative Δ 9.1e-08 | — |
| margins (raw) | detector limits 100%, `RemoveMargins` 100% | recall 1.0, 0.77% of good events (raw values below flowCore's −111 floor) |
| PeacoQC | 100% mask agreement | recall 1.0, 5.6% of good events (bins around the artefact; R removes the same) |
| clean | 100% agreement with FlowKit on the exported gates | debris 0.9988, doublets 1.0, dead 0.9953 recall; ≤ 0.12% good events lost |
| combined mask | — | F1 0.9668, 0.03% of artefacts kept |
| gates | 0 events differ from FlowKit, every gate | precision ≥ 0.999, recall ≥ 0.998; Rare 423 of 425 |
| FlowSOM (7 metaclusters) | ARI 0.9978 vs R | ARI 0.9988 (R 0.9987) |
| FCS output | opens in flowio without `ignore_offset_discrepancy` | — |

On `pipeline_beads` every stage completes and keeps the beads: the singlet
band uses the relative-spread fallback (area equals height, so MAD and IQR are both zero), the margin mask removes nothing, while
`PeacoQC::RemoveMargins` removes all 3,000 events (it drops every event
equal to a channel's observed minimum or maximum). This found the first Stage 3
defect: cytiq's raw margin masks used that rule too; they now use detector
limits only.

### 2.10 Real acquisitions

`parity/real/run_real.sh` runs on real samples that ship with the R packages
(no download) and on any files in `CYTIQ_REAL_DATA` (`parity/real/README.md`).

| Dataset | Check | Result |
|---|---|---|
| FlowSOM `68983.fcs` (19,225 events, 18 colours) | compensate, logicle vs flowCore | Δ 5.9e-08, 7.8e-08 |
| | manual Gating-ML (15 gates, rectangles and polygons) vs FlowKit | 0 events differ |
| | gated populations vs FlowJo labels (Jaccard) | B 0.9959, CD4 T 0.9949, CD8 T 0.9908, γδ T 0.9804, NK 0.9778, NK T 0.8616 |
| | FlowSOM (10 metaclusters) | ARI vs R 0.9395 (R seed to seed 0.9051); vs FlowJo labels 0.9869 (R 0.9718) |
| PeacoQC `111.fcs` (10,000 events, 13 detectors) | compensate, logicle | Δ 5.9e-08, 7.9e-08 |
| | PeacoQC vs R | 100% agreement (12.5% removed) |
| | margins | detector limits 100%; `RemoveMargins` removes 334 vs 328 (observed extremes) |

Running FlowKit on cytiq's compensated output found the second Stage 3 defect:
the FCS writer left the HEADER DATA offsets at zero, which strict readers
reject; it now writes them whenever they fit. The NK T difference is
FlowJo's: its workspace counts 535 events, its exported Gating-ML gives 487 in
both cytiq and FlowKit, because FlowJo evaluates that gate at
`gateResolution="256"` on biex axes under a parent drawn on biex axes.

## 3. Known remaining differences

| Tool | Difference | Impact | Recommendation |
|---|---|---|---|
| FlowSOM | RNG: no bit-exact match with R (MT19937 + `R_unif_index`) | statistical agreement (ARI ≈ seed-to-seed) | port MT19937 if bit-exact reproducibility is needed |
| FlowSOM | ConsensusClusterPlus → average-linkage hc | not visible on the tests | port if needed |
| FlowSOM | `scale` and `nClus` defaults differ from R | product decision | settable with `--no-scale`/`--metaclusters` |
| AutoSpectral | 1–99% box gate instead of the `do.gate` KDE gate by default; no PeacoQC/AF removal on controls | not visible on clean controls | `--clean-controls` uses the `cytiq-autospill` KDE gate; PeacoQC/AF removal remain |
| AutoSpectral | spectrum as a mean difference instead of an `rlm` slope; negatives are the whole AF control instead of scatter-matched kNN | spectrum shapes match to 5e-4 | `rlm` + kNN |
| AutoSpectral | no WLS/Poisson/per-cell AF (R's default `AutoSpectral` method) | comparable only to `method = "OLS"` | separate task |
| AutoSpread | no upstream; units are σ/√F while AutoSpectral expects σ²/F | format incompatibility | document the units |
| transform | biexponential parameters cannot be set from the CLI (defaults hard-coded) | functionality | add `--biex-*` flags |
| cytoscan | subsamples come from `rand`, not R's MT19937, so features on large files agree only statistically | Rust–R Δ equals R's seed-to-seed Δ | port R's RNG if bit-identical features are needed |
| cytoscan | isotree uses its own C++ RNG | scores agree within isotree's Monte Carlo noise (≈0.014 at 1000 trees) | — |
| cytoscan | `linfa` fits full covariance only; mclust also searches 13 constrained structures (EEI, VII, …) | with fewer reference files than features, Rust uses a diagonal Gaussian where mclust picks a constrained model; flags agree 50–100% | add constrained covariance models, or require n_ref > d |
| cytoscan | upstream errors without `SPILL`; Rust warns and keeps raw values | only files without spillover metadata | `--strict` errors like upstream |
| autospill | on a perfectly regular lattice the KDE produces exact ties, broken in a different order than R | degenerate data only | — |
| gate | compensation multiplies by an explicit inverse (Gauss-Jordan); FlowKit uses `np.linalg.solve` | round-off of about 1e-16 relative, so only events lying exactly on a boundary in compensated space could flip; none did at 1M events | — |
| gate | quadrant gates, `fratio` and `new-dimension` are not supported | rejected with an explicit error | add when a real strategy needs them |
| gate | Gating-ML without the required `compensation-ref` is read as `uncompensated`; FlowKit rejects it | FlowJo exports omit it | — |
| margins | `compensate`/`autospectral` masks flag detector limits only; `peacoqc --remove-margins` keeps `PeacoQC::RemoveMargins`, which also removes each channel's observed extremes | identical on continuous data (§2.9); on discrete data RemoveMargins removes far more | — |
| PeacoQC | on small files (15,000 events) the bins are coarse, so ~10% of good events around an artefact are removed (5.6% at 200,000) | same as R | prefer whole acquisitions |
| real data | FlowRepository (AutoSpill datasets) serves an expired certificate (March 2023) | not fetched automatically | download by hand into `CYTIQ_REAL_DATA` |

## 4. Changed files

- `crates/cytiq-transform/src/scales.rs` (new), `lib.rs`, `error.rs`
- `crates/cytiq-autospill/src/{gating,regression,runner,transform}.rs`,
  `Cargo.toml` (dependency on `cytiq-transform`)
- `crates/cytiq-autospectral/src/{runner,lib}.rs`
- `crates/cytiq-peacoqc/src/{density,smooth_spline}.rs` (new), `runner.rs`,
  `lib.rs`
- `crates/cytiq-flowsom/src/{runner,lib}.rs`
- `crates/cytiq-cytoscan/src/{runner,novelty,lib,error}.rs`, `Cargo.toml`
  (`linfa`, `linfa-clustering`, `ndarray`, `rand`, `rand_distr`,
  `rand_xoshiro`); `crates/cytiq-cli/tests/cytoscan_cli.rs`;
  `data/fixtures/cytoscan/reference.rs`
- `crates/cytiq-fcs/src/writer.rs` (buffered output), `data.rs`, `text.rs`
  (Rust 1.99 clippy lints)
- Tests and fixtures:
  - `data/fixtures/peacoqc/{density,smooth_spline}_reference.rs`;
  - `data/fixtures/transform/flowcore_reference.rs`;
  - regenerated `data/fixtures/autospread/expected_spread.csv`;
  - `crates/cytiq-cli/tests/peacoqc_cli.rs`.
- `README.md`: transform semantics, AutoSpectral, Algorithm Notes.

Checks: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D
warnings`, `cargo test --workspace` — all green.

## 5. Reproducing

```bash
parity/setup_env.sh        # once (also `just parity-setup`): R library and uv venv in parity/.env (git-ignored)
parity/run_all.sh main     # synthetic data → Rust (branch + main) → R → parity/results/*.json; fails on a tolerance violation
parity/real/run_real.sh    # real acquisitions → parity/results/metrics_real.json (also `just parity-real`)
just parity-ci-fixture     # regenerate data/fixtures/pipeline (byte-identical)
R_LIBS_USER=parity/.env/Rlib Rscript parity/make_fixtures.R   # regenerate R reference values for unit tests
```

A full run takes about 3 minutes. Raw outputs are in `parity/.out/`, data in
`parity/.data/`.

The suites run locally, not in CI: the R environment takes too long to
install on hosted runners. A violated tolerance exits non-zero, and the
timing wrappers read both BSD (macOS) and GNU `time`. `setup_env.sh`
(`just parity-setup`) pins the Bioconductor release, the git commits of
autospill, AutoSpectral and CytoScan, and the Python packages
(`parity/requirements.txt`); CRAN packages come from `PARITY_CRAN`, and the
installed versions are recorded in `versions.txt`.
