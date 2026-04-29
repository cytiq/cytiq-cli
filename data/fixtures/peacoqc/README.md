# PeacoQC Synthetic Fixtures

This directory contains a compact synthetic event table for testing
`cytiq peacoqc`.

- `sample_events.csv`: deterministic event values with channels used to build a
  temporary FCS input in integration tests

The generated test FCS uses FCS 3.1 float32 data with channels:

```text
Time, FSC-A, SSC-A, 355_378/28_uH-A, 405_750/40_vB-A, 355_515/20_uF-A
```

The fluorescence parameters use `$PnS` labels `CD3 BUV395`, `CD4 BV750`, and
`CD8 BUV496`, so tests can pass channels either by detector name, marker label,
or display identifier such as `355_378/28_uH-A [CD3 BUV395]`.

The fixture is kept as CSV so the test data is easy to inspect and update while
still exercising the real CLI against FCS input.
