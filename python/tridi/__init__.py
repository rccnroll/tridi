"""tridi: point clouds, meshes and STEP files in a window, from Python.

    import tridi
    tridi.show("scan.pcd", pcd, (points, colors), "part.step")

The window is the tridi program, run as a separate process: it never loads
OpenGL or OpenCASCADE into Python, and a crash there can't take Python
down. Clouds from memory reach it as temporary binary PCD files.
"""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

from . import _pcd

__all__ = ["TridiError", "show"]


class TridiError(RuntimeError):
    """The tridi window failed; `stderr` holds what it said."""

    def __init__(self, message, stderr):
        super().__init__(message)
        self.stderr = stderr


def show(*items):
    """Shows `items` in one window and returns when it's closed.

    Each item is a file path (str or os.PathLike), an Open3D PointCloud or
    anything with `.points` (and optionally `.colors`), an (N, 3) array, or
    a (points, colors) tuple. Colors are (N, 3): floats in [0, 1] or uint8.

    Raises ValueError for a bad cloud before any window opens, and
    TridiError if tridi fails (no file could be shown, no display).
    Shift + click prints the picked points on stdout, as the tridi command
    does.
    """
    clouds = [_cloud(it, i) for i, it in enumerate(items)]
    with tempfile.TemporaryDirectory(prefix="tridi-") as tmp:
        args, n = [], 0
        for it, cloud in zip(items, clouds):
            if cloud is None:
                args.append(os.fspath(it))
                continue
            n += 1
            path = os.path.join(tmp, f"cloud-{n}.pcd")
            _pcd.write(path, *cloud)
            args.append(path)
        proc = subprocess.Popen(
            [str(_binary()), *args], stderr=subprocess.PIPE, text=True
        )
        try:
            _, err = proc.communicate()
        except BaseException:
            # Ctrl+C or anything else while the window is open: it goes too
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
            raise
    sys.stderr.write(err)
    if proc.returncode != 0:
        raise TridiError(f"tridi exited with {proc.returncode}", err)


def _cloud(item, i):
    """None for a path, else (points, colors or None), checked."""
    if isinstance(item, (str, os.PathLike)):
        return None
    if isinstance(item, tuple):
        if len(item) != 2:
            raise ValueError(f"item {i}: a tuple is (points, colors)")
        points, colors = item
    elif hasattr(item, "points"):
        points, colors = item.points, getattr(item, "colors", None)
    else:
        points, colors = item, None
    points = np.asarray(points)
    if points.ndim != 2 or points.shape[1] != 3:
        raise ValueError(f"item {i}: points must be (N, 3), got {points.shape}")
    if len(points) == 0:
        raise ValueError(f"item {i}: no points")
    if colors is not None:
        colors = np.asarray(colors)
        # Open3D's colors are empty when the cloud has none
        if colors.size == 0:
            colors = None
        elif colors.ndim != 2 or colors.shape[1] != 3 or len(colors) != len(points):
            raise ValueError(
                f"item {i}: {len(points)} points, {len(colors)} colors: one (r, g, b) per point"
            )
    return points, colors


def _binary():
    """The tridi program inside this package."""
    exe = Path(__file__).parent / "_bin" / "tridi"
    if not exe.exists():
        raise FileNotFoundError(
            "this tridi package has no binary: install a wheel, not the source"
        )
    return exe
