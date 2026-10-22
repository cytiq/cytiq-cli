# Parity report: cytiq (Rust) vs the original tools

Date: 2026-10-22 · branch `parity-fixes` · baseline `main` (`0f4c72a`)

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
| `flowsom` | FlowSOM 2.18.0 | ⚠️ ARI vs R 0.87–0.91 (hard set), 9× slower than R | ✅ ARI vs R 0.93–0.98 (R's own seed-to-seed level), ~8× faster than R |

**36 deviations** from the upstream algorithms were found and fixed, plus
two performance defects (section 2.6). All key
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

3. **Runs.** Rust (`parity/run_rust.sh`) for the current branch and for `main`
   as the baseline; the upstream R tools (`parity/run_upstream.R`) with the
   same parameters. FlowSOM in R runs with `nClus = 20`, `scale = FALSE/TRUE`,
   seeds 1–3 (R's RNG cannot be reproduced in Rust, so the comparison is
   statistical).
4. **Metrics** (`parity/compare.py` → `parity/results/*.json`): max |Δ| for
   transforms; MAE/max |Δ|/Frobenius for matrices; RMSE/scale/correlation for
   unmixing; precision/recall/Jaccard/agreement for QC masks; ARI/NMI/purity
   for clustering; wall time and peak memory (`/usr/bin/time -l`).

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

## 3. Known remaining differences

| Tool | Difference | Impact | Recommendation |
|---|---|---|---|
| FlowSOM | RNG: no bit-exact match with R (MT19937 + `R_unif_index`) | statistical agreement (ARI ≈ seed-to-seed) | port MT19937 if bit-exact reproducibility is needed |
| FlowSOM | ConsensusClusterPlus → average-linkage hc | not visible on the tests | port if needed |
| FlowSOM | `scale` and `nClus` defaults differ from R | product decision | expose as CLI flags |
| AutoSpectral | 1–99% box gate instead of the `do.gate` KDE gate; no PeacoQC/AF removal on controls | not visible on clean controls | reuse the gate from `cytiq-autospill` |
| AutoSpectral | spectrum as a mean difference instead of an `rlm` slope; negatives are the whole AF control instead of scatter-matched kNN | spectrum shapes match to 5e-4 | `rlm` + kNN |
| AutoSpectral | no WLS/Poisson/per-cell AF (R's default `AutoSpectral` method) | comparable only to `method = "OLS"` | separate task |
| AutoSpread | no upstream; units are σ/√F while AutoSpectral expects σ²/F | format incompatibility | document the units |
| transform | biexponential parameters cannot be set from the CLI (defaults hard-coded) | functionality | add `--biex-*` flags |
| autospill | on a perfectly regular lattice the KDE produces exact ties, broken in a different order than R | degenerate data only | — |

## 4. Changed files

- `crates/cytiq-transform/src/scales.rs` (new), `lib.rs`, `error.rs`
- `crates/cytiq-autospill/src/{gating,regression,runner,transform}.rs`,
  `Cargo.toml` (dependency on `cytiq-transform`)
- `crates/cytiq-autospectral/src/{runner,lib}.rs`
- `crates/cytiq-peacoqc/src/{density,smooth_spline}.rs` (new), `runner.rs`,
  `lib.rs`
- `crates/cytiq-flowsom/src/{runner,lib}.rs`
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
parity/setup_env.sh        # once: R library and uv venv in parity/.env (git-ignored)
parity/run_all.sh main     # synthetic data → Rust (branch + main) → R → parity/results/*.json
R_LIBS_USER=parity/.env/Rlib Rscript parity/make_fixtures.R   # regenerate R reference values for unit tests
```

A full run takes about 3 minutes. Raw outputs are in `parity/.out/`, data in
`parity/.data/`.
