# Synthetic v2 classifier contract fixtures

All events and models here are synthetic. The class vocabulary and panel metadata
come from `cytiq-tree` reference contracts. These fixtures test software behavior
and do not establish biological classification performance.

`catboost/` and `lightgbm/` contain six-class ONNX bundles for 192 synthetic FlowCyt
profile events in `input.fcs`; each `*.expected.json` records native predictions.
`binary/` contains a Blast-phenotype v2 bundle for the existing parent directory's
synthetic `raw.fcs`, with a saved threshold of 0.4.

Regenerate with the sibling ML repository's
`tests/integration/multiclass_worker.py` modes `catboost`, `lightgbm`, `binary`.
Copy portable ONNX/manifest/classes/panel/mapping files, expected probabilities and
input FCS. Keep native weights and timing experiments outside the fixture tree.
