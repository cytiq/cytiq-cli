"""Minimal FCS 3.1 float32 writer/reader helpers shared by the parity generators."""

from __future__ import annotations

from pathlib import Path

import flowio
import numpy as np

PNR = 262144


def write_fcs(path: Path, data: np.ndarray, channels: list[str], labels: list[str] | None = None,
              pnr: int = PNR, extra_keywords: dict[str, str] | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = np.asarray(data, dtype=np.float32)
    metadata = {f"p{i + 1}r": str(pnr) for i in range(len(channels))}
    metadata.update(extra_keywords or {})
    with open(path, "wb") as handle:
        flowio.create_fcs(handle, data.ravel(order="C").tolist(), channels,
                          opt_channel_names=labels, metadata_dict=metadata)


def read_fcs(path: Path) -> tuple[np.ndarray, list[str]]:
    flow = flowio.FlowData(str(path), ignore_offset_discrepancy=True)
    data = np.reshape(np.asarray(flow.events, dtype=np.float64), (-1, flow.channel_count))
    names = [flow.text[f"p{i + 1}n"] for i in range(flow.channel_count)]
    return data, names
