"""Print the comma-separated channels of a file's spillover keyword."""

import sys

import flowio

flow = flowio.FlowData(sys.argv[1], ignore_offset_discrepancy=True)
spill = next(value for key, value in flow.text.items() if key.lower() in ("spill", "spillover"))
fields = spill.split(",")
print(",".join(field.strip() for field in fields[1:1 + int(fields[0])]))
