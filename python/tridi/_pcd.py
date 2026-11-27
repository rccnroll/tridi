"""Clouds from memory written as binary PCD files, the way tridi reads them.

The fields are PCL's: x y z as little-endian float32, and the colors, if
any, as `rgb`: the bits 0x00RRGGBB of a uint32 stored in a 4-byte float
field. tridi (tridi-formats' `unpack_rgb`) reads those bits whatever the
field's TYPE says, as PCL and Open3D write them.
"""

import numpy as np


def write(path, points, colors=None):
    """Writes `points` (N, 3) and optional `colors` (N, 3) to `path`.

    Colors are floats in [0, 1] (Open3D's) or integers in [0, 255].
    Raises ValueError for anything else; the caller checks shapes.
    """
    pts = np.asarray(points, dtype="<f4")
    n = len(pts)
    fields = ["x", "y", "z"]
    if colors is None:
        data = np.ascontiguousarray(pts)
    else:
        fields.append("rgb")
        rgb = _bytes(np.asarray(colors)).astype(np.uint32)
        packed = (rgb[:, 0] << 16) | (rgb[:, 1] << 8) | rgb[:, 2]
        data = np.empty((n, 4), dtype="<u4")
        data[:, :3] = pts.view("<u4")
        data[:, 3] = packed
    k = len(fields)
    header = "\n".join(
        [
            "# .PCD v0.7 - Point Cloud Data file format",
            "VERSION 0.7",
            "FIELDS " + " ".join(fields),
            "SIZE" + " 4" * k,
            "TYPE" + " F" * k,
            "COUNT" + " 1" * k,
            f"WIDTH {n}",
            "HEIGHT 1",
            "VIEWPOINT 0 0 0 1 0 0 0",
            f"POINTS {n}",
            "DATA binary",
            "",
        ]
    )
    with open(path, "wb") as f:
        f.write(header.encode())
        f.write(data.tobytes())


def _bytes(c):
    """Colors as 0..255 integers: floats must be in [0, 1], integers in [0, 255]."""
    if np.issubdtype(c.dtype, np.floating):
        if c.size and (np.nanmin(c) < 0.0 or np.nanmax(c) > 1.0):
            raise ValueError("float colors must be in [0, 1]; pass 0..255 as uint8")
        return np.rint(np.nan_to_num(c) * 255.0)
    if c.size and (c.min() < 0 or c.max() > 255):
        raise ValueError("integer colors must be in [0, 255]")
    return c
