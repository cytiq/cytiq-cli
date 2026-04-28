# AutoSpectral Synthetic Fixtures

This directory contains a small spectral-flow fixture set for testing
`cytiq autospectral`.

- `unstained.fcs`: autofluorescence/background control
- `fitc.fcs`: single-stain FITC control
- `pe.fcs`: single-stain PE control
- `sample.fcs`: fully stained sample containing FITC, PE, and AF signal
- `controls.csv`: minimal controls manifest (`filename,dye`)
- `samples.csv`: minimal samples manifest (`filename`)

The FCS files use FCS 3.1 float32 data with channels:

```text
FSC-A, SSC-A, FITC, PE, AF
```

The fixture intentionally does not include `detectors.csv`; tests exercise
automatic detector inference from shared non-scatter channels.
