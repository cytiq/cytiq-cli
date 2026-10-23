# cytiq-manifest

The run manifest: one YAML (`.yaml`/`.yml`) or JSON (`.json`) file that
describes an acquisition run. Read by `cytiq preflight`.

```yaml
cytometry: conventional        # or spectral
panel:
  name: T-cell-8c
  markers:
    - { marker: CD3, channel: FL1-A }        # channel optional: matched by $PnS
    - { marker: Live/Dead, required: false }  # required defaults to true
samples:
  - { id: S01, file: fcs/s01.fcs }
controls:                                     # optional
  - { file: fcs/unstained.fcs, dye: Autofluorescence }
  - { file: fcs/cd3_fitc.fcs, dye: FITC, primary_channel: FL1-A }
```

## Public API

| Item | Purpose |
| --- | --- |
| `RunManifest::load` | Read, parse (by extension) and validate; resolves relative paths against the manifest's directory |
| `RunManifest`, `Cytometry`, `Panel`, `Marker`, `SampleEntry`, `ControlEntry` | The document |
| `ControlEntry::is_unstained` | `dye` is `Autofluorescence`, `AF` or `unstained` |
| `ManifestError` | Error type |

## Rules

- Unknown fields are errors, with the line and column for YAML.
- At least one sample and one marker; sample ids and marker names are unique
  (marker names ignoring case); file paths are not empty.
- Spectral runs need an unstained control.
- Files themselves are not opened here; `cytiq preflight` checks them.

## Dependencies

`serde`, `serde-saphyr` (YAML), `serde_json`, `thiserror`.

## Tests

```bash
cargo test -p cytiq-manifest
cargo test -p cytiq-cli --test preflight_cli
```
