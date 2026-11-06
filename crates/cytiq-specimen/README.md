# cytiq-specimen

Specimen planning, summary-only reconciliation, consistency checks and publication.
This is a research/shadow-run interface; `reference_ratio` remains experimental.

## Inputs and ownership

The run manifest owns acquisition paths, IDs and panel assignments. Omitted
`schema_version` means v1; v2 permits repeat acquisitions and `cytometry: cytof`.
The specimen protocol owns source bindings, panel requirements and reconciliation.
`cytiq-tree` owns ontology, panel projections and classifier vocabularies.

`SpecimenLayout` v1 accepts JSON/YAML and rejects unknown fields. All file paths
resolve against the layout. Panels reference projections by **canonical** SHA-256;
source `definition` references identify exact Gating-ML or model.onnx **bytes**.
Every panel must use the same tree/context. Population counts are not inferred
from hierarchy links.

```yaml
schema_version: 1
protocol_id: immune-research-v1
panels:
  P1:
    required: true
    projection: {path: P1.projection.json, sha256: "<canonical projection digest>"}
    sources:
      gates:
        format: gate_membership
        definition: {path: gates.xml, sha256: "<file digest>"}
        bindings:
          - label: All
            target: {kind: population, id: leukocytes, scope: leukocytes}
          - label: T
            target: {kind: population, id: t_cells, scope: leukocytes}
            denominator: All
reconciliation: []
consistency: []
```

Classifier sources use `format: classify_predictions`, a model definition,
`bundle_manifest: {path: manifest.yaml, sha256: "<file digest>"}`, an explicit
`abstain_margin` (defaults to 0), and
`vocabulary: {path: classes.yaml, sha256: "<canonical vocabulary digest>"}`.
Bindings come from that vocabulary. The supplied projection must match all of its
knowledge identities; placeholder tree/projection digests are rejected.

`SpecimenPlan` v1 snapshots candidates and the effective layout. With a single
acquisition a panel selects it automatically; repeats require `--select`.
Non-selected acquisitions retain `not_selected` reasons. No `qc_best` heuristic
or replicate averaging runs implicitly.

## Running a specimen

First freeze definitions and selection; `--output` always names a package directory:

```sh
cytiq specimen validate --manifest run.yaml --layout layout.yaml --specimen S01 --select P1=A02
cytiq specimen plan --manifest run.yaml --layout layout.yaml --specimen S01 --select P1=A02 --output plan/
```

Use existing per-acquisition commands with the same final combined mask for
technical QC and annotations. Conventional flow supports gate/classify; spectral
flow supports gate; CyTOF supports the existing classify recipe. New debarcoding,
EQ-bead normalization and spectral ML recipes are outside this version.

```sh
cytiq qc --manifest run.yaml --sample A02 --mask analysis.parquet --output qc.json
cytiq gate --input a02.fcs --gating-ml gates.xml --mask analysis.parquet \
  --membership membership.parquet --output counts.json --format json
cytiq population observe --plan plan/plan.json --acquisition A02 --source gates \
  --annotations membership.parquet --producer-provenance counts.json.provenance.json \
  --mask analysis.parquet --qc-report qc.json --qc-provenance qc.json.provenance.json \
  --output observations/A02-gates/
cytiq specimen run --plan plan/plan.json --observations observations/A02-gates/ \
  --output results/S01/
```

For ML use `predictions.parquet` and the classifier's `provenance.json` as the
annotation inputs. Repeat `--observations` for every selected panel/source.
`population validate` takes the observe input flags, omits `--output`, and runs
the same validation without publishing a package.

## Reconciliation and QC

Without rules all observations remain independent. Rules are explicit per target:

```yaml
reconciliation:
  - target: {kind: population, id: t_cells, scope: leukocytes}
    strategy:
      kind: reference_ratio
      source: {panel: P2, source: gates}
      reference: {panel: P1, source: gates}
      anchor: {kind: population, id: leukocytes, scope: leukocytes}
consistency:
  - target: {kind: population, id: t_cells, scope: leukocytes}
    max_abs_fraction_difference: 0.05
    severity: warn
```

`independent` retains all source estimates. `prefer_reference` names a
`reference: {panel, source}` and selects its count/fraction without fallback.
`reference_ratio` computes source count × reference anchor / source anchor.
Both anchors must be positive, measured and compatible. It emits a separate
experimental `estimated_count`; all originals remain in `observations`.
Its method references the three contributing observation identities.

Technical failures make observations `qc_rejected`, with supporting counts.
Consistency policies compare fractions only when target/denominator scopes agree;
they never delete samples or rewrite biological measurements. Missing QC policies
remain warnings from the upstream technical report. Missing sources and panels
produce explicit null observations; absent optional panels do not make a specimen
incomplete. Clinical criteria, absolute concentration and LoD/LoQ are not evaluated.

## Packages and orchestration contract

Plans contain `plan.json`; observations contain `event-annotations.json` and
`observations.parquet`; specimen results contain `result.json` and
`results.parquet`. Every package also contains RunRecord v2 `provenance.json` and
completion record v1 `outcome.json`. Readers verify every outcome digest before
using data; the presence of a result file alone does not prove completion.
`SpecimenResult` v1 includes source observations, selected/derived results and
separate technical/completeness/consistency checks.
`SpecimenResult::load` verifies package digests, row identities, reconciliation
references, outcome/QC agreement and logical equality of JSON and Parquet results.

Publication stages and syncs a complete directory before replacement.
`--overwrite` replaces the entire prior package; the default refuses existing
outputs. No cache/resume or implicit retries are implemented. Failures and
interrupted writes must be reconciled by the caller using completion records.
Only `specimen plan` reads raw FCS metadata; `specimen run` reads verified plans
and population packages, so raw files and native annotations can be offline.

After argument parsing the new command families emit one JSON outcome on stdout:

| Code | Meaning |
| --- | --- |
| 0 | Completed successfully; warnings remain visible |
| 1 | Runtime/internal error |
| 2 | Invalid configuration or incompatible/corrupt data |
| 3 | Completed and published, checks failed |
| 4 | Existing output conflict, missing required input, or published incomplete specimen |

`completed` distinguishes published partial/check-failed results from execution
errors. Argument syntax errors use clap's usual code 2 and need not have JSON.
Existing commands retain their previous exit codes. This contract is usable by
the API/compute facade and Prefect without either becoming a runtime dependency.

## Validation and shadow runs

```sh
cargo test -p cytiq-population -p cytiq-specimen
cargo test -p cytiq-engine --test specimen_cli
just verify
just docker-build
```

Fixtures establish software behavior, not clinical validity. For historical shadow
runs, freeze the layout, models, input hashes and legacy version; compare measured
counts separately from derived estimates, missing/rejected states, eligibility,
fractions and QC. Record differences and acceptance bounds per panel/instrument
before promoting experimental reconciliation. The known 3000/10000 versus
6400/20000 case must retain both measured counts and emit 3200 only as an estimate.
