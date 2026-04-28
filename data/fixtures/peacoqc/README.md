# PeacoQC Synthetic Fixtures

This directory contains a compact synthetic event table for testing
`cytiq peacoqc`.

- `sample_events.csv`: deterministic event values with channels used to build a
  temporary FCS input in integration tests

The generated test FCS uses FCS 3.1 float32 data with channels:

```text
Time, FSC-A, SSC-A, CD3, CD4
```

The fixture is kept as CSV so the test data is easy to inspect and update while
still exercising the real CLI against FCS input.
