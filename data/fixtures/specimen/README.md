# Specimen contract fixtures

`tree.json`, `P1.projection.json`, `P2.projection.json` and `gates.xml` are small
synthetic software fixtures. The gates intentionally overlap and include an empty
population. They do not assert biological or clinical validity. Their format is
the existing cytiq-tree v2 contract; there is no new engine-owned ontology schema.

`facscanto2-p1.projection.json` and `flowcyt-bm.projection.json` are copied unchanged
from cytiq-tree revision `6979697dd898387a841571edb78969ce737c8076`, `examples/`.
Their SHA links are checked by the runtime reader. The pinned schema set lives
in `crates/cytiq-population/contracts`.

CLI tests reuse existing FCS/ONNX fixtures. For the synthetic CyTOF model, the
original fixture has placeholder knowledge digests; tests repack its unchanged
ONNX and feature recipe against complete synthetic knowledge artifacts in a
temporary directory. Original fixtures and expected model results are untouched.

Reconciliation tests independently specify 3000/10000 and 6400/20000 counts and
expect the separate reference-ratio estimate 3200. Other cases cover missing
panels, zero anchors, biological disagreement and absence of fallback/roll-up.
