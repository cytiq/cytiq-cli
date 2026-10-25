# Real-data validation

`parity/real/run_real.sh` (or `just parity-real`) runs cytiq and the upstream
R tools on real acquisitions and compares them per event. It needs
`parity/setup_env.sh` once. The datasets are listed in
[`datasets.json`](datasets.json): each file comes from an installed R package,
a URL, or a member of the dataset's zip `archive` (downloads and extracted
members are cached in `parity/.data/real-cache`), and must match its SHA-256.
A dataset's `chain` picks the comparison:

- `qc` works for any file with `$SPILL`;
- `autospill` takes single-stain controls, listed in the entry's `controls`
  table (filename, detector `$PnN` as `dye`, antigen, wavelength), and
  compares `cytiq autospill` with R autospill;
- `flowsom_gating` is specific to FlowSOM's 68983 sample.

To add a dataset, add an entry with its source, licence, instrument, SHA-256,
cytometry type and panel. A downloaded dataset is marked `optional`: it is
skipped with a warning when the download fails, and the tolerance rules that
only it can satisfy are marked `optional` too.

| Dataset | Source | What is compared |
| --- | --- | --- |
| `flowsom_68983` | FlowSOM `extdata/68983.fcs`: 19,225-event, 18-colour mouse sample; `manualGating.xml` and the FlowJo labels of every event (`gatingResult.csv`, from `gating.wsp`) | `cytiq compensate` and `transform --method logicle` against flowCore; the manual Gating-ML strategy in `cytiq gate` against FlowKit and against the FlowJo labels; `cytiq flowsom` against R FlowSOM (two seeds give R's own spread) and against the FlowJo labels |
| `peacoqc_111` | PeacoQC `extdata/111.fcs`: 10,000-event, 20-colour LSRII sample from the PeacoQC vignette | detector-limit margins, compensation, logicle and PeacoQC against flowCore and R PeacoQC |
| `celesta_mdsc_controls` | [Zenodo 21384789](https://doi.org/10.5281/zenodo.21384789) (CC-BY-4.0, 92 MB archive): 11 single-stain mouse cell controls, 5,000 events each, BD FACSCelesta (4-Blue 3-Red 5-Violet, FACSDiva 9.0) | `cytiq autospill` against R autospill: spillover matrix and the gate of every control |
| `celesta_mdsc_sample` | the same archive: `Myeloid SP_042.fcs`, 56,458-event mouse spleen sample with the FACSDiva compensation | as `peacoqc_111` |
| `symphony_omip24_controls` (slow) | [Zenodo 22994607](https://doi.org/10.5281/zenodo.22994607) `Table 8.zip` (CC-BY-4.0, 193 MB): 24 single-stain human PBMC controls of a 24-colour OMIP, 27,000–60,000 events each, BD FACSymphony A3 or A5 (`5B-3R-8V-7UV-3YG`, FACSDiva 9.1; the record does not name the model) | as `celesta_mdsc_controls`; R autospill needs about 25 minutes, so it runs only with `CYTIQ_REAL_SLOW=1` |
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
- Downloads use `curl`, which trusts the system certificate store (Python's
  `urllib` rejects TLS-inspecting proxies).
- On FACSDiva float data flowCore's −111 floor flags noise around zero, not
  saturation: `PeacoQC::RemoveMargins` removes 17,737 events of
  `celesta_mdsc_sample` (31%), 17,700 of them at or below −111. cytiq's margin
  mask uses the upper detector limit `$PnR − 1` only and flags 36; every one
  of them is also removed by `RemoveMargins` (`kept_by_RemoveMargins` = 0).
  See `parity/REPORT.md` §2.10 and §3.
- The AutoSpill datasets on FlowRepository (FR-FCM-Z2SS/Z2ST/Z2SV) are not
  fetched automatically: flowrepository.org serves a certificate that expired
  in March 2023 with an incomplete chain, so it cannot be downloaded over
  verified TLS. Download them by hand and point `CYTIQ_REAL_DATA` (QC chain) or
  `cytiq autospill` at them.
