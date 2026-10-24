# Real-data validation

`parity/real/run_real.sh` (or `just parity-real`) runs cytiq and the upstream
R tools on real acquisitions and compares them per event. It needs
`parity/setup_env.sh` once. The datasets are listed in
[`datasets.json`](datasets.json): each file comes from an installed R package
or a URL (downloaded once into `parity/.data/real-cache`) and must match its
SHA-256. A dataset's `chain` picks the comparison: `qc` works for any file
with `$SPILL`; `flowsom_gating` is specific to FlowSOM's 68983 sample. To add
a dataset, add an entry with its source, SHA-256, cytometry type and panel; a
URL entry marked `optional` is skipped with a warning when its download
fails.

| Dataset | Source | What is compared |
| --- | --- | --- |
| `flowsom_68983` | FlowSOM `extdata/68983.fcs`: 19,225-event, 18-colour mouse sample; `manualGating.xml` and the FlowJo labels of every event (`gatingResult.csv`, from `gating.wsp`) | `cytiq compensate` and `transform --method logicle` against flowCore; the manual Gating-ML strategy in `cytiq gate` against FlowKit and against the FlowJo labels; `cytiq flowsom` against R FlowSOM (two seeds give R's own spread) and against the FlowJo labels |
| `peacoqc_111` | PeacoQC `extdata/111.fcs`: 10,000-event, 20-colour LSRII sample from the PeacoQC vignette | detector-limit margins, compensation, logicle and PeacoQC against flowCore and R PeacoQC |
| `user_<name>` | every `*.fcs` with `$SPILL` in `$CYTIQ_REAL_DATA` | as `peacoqc_111`, for data that cannot be committed |

The run also executes the ignored >100 MB FCS round-trip test of
`cytiq-fcs`. Metrics go to `parity/results/metrics_real.json`; the run exits
with 1 when a rule in `parity/real/tolerances.json` is violated.

Notes:
- FlowJo exports channel names as `Comp-<detector>` and omits the
  `compensation-ref` Gating-ML 2.0 requires; the run compensates with
  `cytiq compensate` first and feeds both cytiq and FlowKit the same corrected
  strategy (`uncompensated` dimensions on the compensated file).
- FlowJo's own counts for `NK T cells` (535) differ from evaluating its
  exported Gating-ML (487, in cytiq and FlowKit alike): FlowJo evaluates that
  gate at `gateResolution="256"` on biex axes. See `parity/REPORT.md` §2.10.
- The AutoSpill datasets on FlowRepository (FR-FCM-Z2SS/Z2ST/Z2SV) are not
  fetched automatically: flowrepository.org serves a certificate that expired
  in March 2023 with an incomplete chain, so it cannot be downloaded over
  verified TLS. Download them by hand and point `CYTIQ_REAL_DATA` (QC chain) or
  `cytiq autospill` at them.
