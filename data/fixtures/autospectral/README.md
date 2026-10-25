# AutoSpectral Synthetic Fixtures

This directory contains a small spectral-flow fixture set for testing
`cytiq autospectral`.

- `unstained.fcs`: autofluorescence/background control; its autofluorescence
  varies from event to event along one spectrum (20, 12, 8 scaled by
  0.75-1.25), since spectra are slopes against the peak detector
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
