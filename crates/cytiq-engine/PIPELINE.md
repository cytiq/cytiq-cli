# Pipeline

The [pipeline diagram](../../README.md#pipeline) shows the commands and their
backing crates. The [conventional chain](#conventional-chain) below follows
its stage order and input checks. `gate` and `classify` also take the raw FCS
and compensate and transform it themselves.

Shared library crates sit under every stage and are not drawn: `cytiq-fcs`
reads and writes all FCS files, `cytiq-mask` reads and writes the masks,
`cytiq-density` holds the kernel density and FSC/SSC gate used by `autospill`,
`autospectral`, `clean` and `peacoqc`, `cytiq-manifest` parses the run
manifest for `preflight` and `qc`, and `cytiq-provenance` writes the
provenance record of every command (see [cytiq-provenance](../cytiq-provenance/README.md)).

## Conventional chain

```bash
cytiq meta       --input sample.fcs --output sample.meta.json
cytiq stats      --input sample.fcs --output sample.stats.json
cytiq spillqc    --input samples/ --output spillqc/     # optional: flags likely $SPILL errors for review
cytiq compensate --input sample.fcs --output sample.comp.fcs
cytiq transform  --input sample.comp.fcs --output sample.t.fcs
cytiq peacoqc    --input sample.t.fcs --output sample.peacoqc.fcs --channels CD3,CD4,CD8
cytiq clean      --input sample.fcs --output clean/     # raw scatter and viability
cytiq mask combine --mask sample.comp.masks.parquet --mask sample.peacoqc.masks.parquet \
  --mask clean/masks.parquet --output sample.analysis.parquet
cytiq qc         --manifest run.yaml --sample S01 --mask sample.analysis.parquet --output qc.json
cytiq gate       --input sample.t.fcs --gating-ml gates.xml --output counts.csv \
  --mask sample.analysis.parquet --membership membership.parquet
cytiq flowsom    --input sample.t.fcs --output flowsom.csv --channels CD3,CD4,CD8 \
  --mask sample.analysis.parquet
cytiq plot       --input sample.fcs --x FSC-A --y SSC-A --output fsc_ssc.raw.svg
cytiq plot       --input sample.fcs --mask sample.analysis.parquet \
  --x FSC-A --y SSC-A --output fsc_ssc.qc.svg            # kept events, removed ones over them
cytiq plot       --input sample.t.fcs --mask sample.analysis.parquet --gating-ml gates.xml \
  --population T --gate CD4T --gate CD8T --type contour --output t_cd4_cd8.svg
cytiq plot       --input sample.t.fcs --color-by flowsom.csv --type scatter \
  --x CD3 --y CD19 --output clusters.svg
```

`plot` needs only an FCS file and, optionally, masks, Gating-ML and class
labels. It never fails because a QC check failed: a mask that removes every
event renders an annotated plot. Workflows should therefore run it after
`mask combine` whatever `qc`'s exit status, because a failed sample is the one
most worth inspecting. `--config plots.yaml` renders a panel's plots of one
sample in one run (see [cytiq-plot](../cytiq-plot/README.md#batch-file)).

## Signal state

For batch normalization, train a panel-specific model from QC-filtered controls
or comparable aggregates, then apply it before analysis:

```bash
cytiq cytonorm train --manifest batches.yaml --output cytonorm-model/
cytiq cytonorm apply --input sample.peacoqc.fcs --model cytonorm-model/ \
  --batch day2 --output sample.norm.fcs
```

Training masks belong to individual manifest entries. Apply preserves every
input event and its lineage, so existing analysis masks remain valid. Acquisition
QC, margin checks and raw-space gates run before normalization. Gates evaluated
after normalization must be designed in the normalized value space. Classifier
bundle v3 must pin the normalization model ID; v1/v2 reject normalized inputs.
See the [manifest and QC contract](../cytiq-cytonorm/README.md).

`compensate` writes log-amplified input (`$PnE` other than `0,0`) as linear
values; `gate` and `clean` linearise it as they read. Stages that read stored
values (`transform`, `peacoqc`, `flowsom`, `cytoscan`, `autospill` and
`autospectral`) refuse log-amplified parameters. Run them on the output of
`compensate`.

Each stage reads the `CYTIQ_COMPENSATION`, `CYTIQ_UNMIXING` and
`CYTIQ_TRANSFORM` keywords earlier stages wrote and checks its input:

| Stage | Expects | Otherwise |
| --- | --- | --- |
| `autospill`, `autospread`, `autospectral`, `cytoscan` | raw files | error |
| `preflight` | raw files; for samples of a `compensated` or `unmixed` panel, also files `cytiq compensate` or `cytiq autospectral` processed, untransformed | failed check |
| `compensate` | raw matrix channels | error when already compensated, unmixed, or a matrix channel is transformed |
| `spillqc` | raw matrix channels; with `--preprocessed`, compensated and transformed values as stored | error when already compensated, unmixed, or a matrix channel is transformed (without `--preprocessed`) |
| `transform` | compensated or unmixed channels | error when a selected channel's spillover is still pending (`--allow-uncompensated`), or the file is already transformed (`--allow-retransform`) |
| `clean` | raw scatter and viability channels | error |
| `peacoqc` | compensated, transformed channels | warning |
| `flowsom` | transformed channels | warning |
| `flowsom apply` | saved marker and preprocessing contract | error on mismatch; never refits scale |
| `cytonorm train/apply` | compatible panel and complete transformed, compensated/unmixed state | error on mismatch, repeated normalization or unknown batch |
| `classify` v3 | explicitly declared normalization ID or `none`, plus the existing feature contract | error before inference on mismatch |
| `gate` | what the Gating-ML dimensions say | error when a dimension would compensate or transform a channel again |
| `plot` | any; transformed channels are drawn on a linear axis labelled in raw decades | error when `--x-scale`/`--y-scale` asks for `logicle` or `arcsinh` on a transformed channel, or a gate dimension would compensate or transform a channel again |

Gating-ML strategies applied to `sample.t.fcs` must use `uncompensated`
dimensions without `transformation-ref` on the compensated and transformed
channels. Otherwise apply the strategy to the raw `sample.fcs`, which `cytiq
gate` compensates and transforms itself.

`CYTIQ_NORMALIZATION` records the normalizer and batch. `transform` rejects
normalized input even with `--allow-retransform`. Normalized marker columns no
longer have acquisition margin bounds; do not rerun acquisition margin filtering
on them. Compensation/unmixing and the original transform metadata remain recorded.
