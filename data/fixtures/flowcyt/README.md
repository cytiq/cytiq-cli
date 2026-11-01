# FlowCyt fixtures

Two unmodified files from the FlowCyt author archive: Bini, Mojarrad, Liarou,
Matthes and Marchand-Maillet, *FlowCyt: A Comparative Study of Deep Learning
Approaches for Multi-Class Classification in Flow Cytometry*, CHIL 2024.
Archive: Yareta, doi:10.26037/yareta:54dmqf5nj5d2hoyaprnzh52rz4, licensed
[CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/). Members
`researchdata/flowcyt_benchmark/raw/<name>`.

| File | SHA-256 | What it is |
| --- | --- | --- |
| `Case12_P.fcs` | `61835b0c89cc3f60f5d65172b0845ac5f9920d92e86f27390a686f337fa3fad5` | Kaluza export, Beckman Coulter Navios: FCS 3.0, 15 big-endian float32 channels, 3 mast cells, `$SPILLOVER` over FL1–FL10, `$CYT=Kaluza` with `$OriginalInstrument=Navios` |
| `Case30_P.fcs` | `0f4a15bc6029a497422397f2b39e354020d3437b24f68d7e755cb7057b2b7bc2` | flowCore re-export of compensated values: FCS 3.0, 149 events, R-style channel names (`CD14.FITC`), `flowCore_$PnRmax` keywords, no instrument keywords, negative fluorescence values |

The archive's B gates are flowCore re-exports like `Case30_P.fcs`, with
names such as `FL1.INT_CD14.FITC`; the other gates are Kaluza exports like
`Case12_P.fcs`. cytiq-ml's FlowCyt importer documents how it reads both.
