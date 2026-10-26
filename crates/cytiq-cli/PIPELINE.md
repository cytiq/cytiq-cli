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
```

## Signal state

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
| `transform` | compensated or unmixed channels | error when a selected channel's spillover is still pending (`--allow-uncompensated`), or the file is already transformed (`--allow-retransform`) |
| `clean` | raw scatter and viability channels | error |
| `peacoqc` | compensated, transformed channels | warning |
| `flowsom` | transformed channels | warning |
| `gate` | what the Gating-ML dimensions say | error when a dimension would compensate or transform a channel again |

Gating-ML strategies applied to `sample.t.fcs` must use `uncompensated`
dimensions without `transformation-ref` on the compensated and transformed
channels. Otherwise apply the strategy to the raw `sample.fcs`, which `cytiq
gate` compensates and transforms itself.
