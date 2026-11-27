import numpy as np
import pytest

from tridi import _pcd

HEADER = [
    b"# .PCD v0.7 - Point Cloud Data file format",
    b"VERSION 0.7",
    b"FIELDS x y z",
    b"SIZE 4 4 4",
    b"TYPE F F F",
    b"COUNT 1 1 1",
    b"WIDTH 2",
    b"HEIGHT 1",
    b"VIEWPOINT 0 0 0 1 0 0 0",
    b"POINTS 2",
    b"DATA binary",
]


def body(path, n_header):
    return path.read_bytes().split(b"\n", n_header)[n_header]


def rgb_bits(path, n_header):
    return np.frombuffer(body(path, n_header), dtype="<u4").reshape(-1, 4)[:, 3]


def test_header_is_exact(tmp_path):
    p = tmp_path / "a.pcd"
    _pcd.write(p, np.zeros((2, 3)))
    assert p.read_bytes().split(b"\n")[: len(HEADER)] == HEADER


def test_float64_points_are_written_as_float32(tmp_path):
    p = tmp_path / "a.pcd"
    pts = np.arange(6, dtype=np.float64).reshape(2, 3)[:, ::-1]  # not contiguous
    _pcd.write(p, pts)
    assert body(p, len(HEADER)) == np.array([[2, 1, 0], [5, 4, 3]], "<f4").tobytes()


def test_float_colors_are_packed(tmp_path):
    p = tmp_path / "a.pcd"
    _pcd.write(p, np.zeros((1, 3)), np.array([[1.0, 0.5, 0.0]]))
    # 0.5 rounds to 128, not floors to 127
    assert rgb_bits(p, len(HEADER)).tolist() == [0x00FF8000]


def test_uint8_colors_are_packed(tmp_path):
    p = tmp_path / "a.pcd"
    _pcd.write(p, np.zeros((1, 3)), np.array([[255, 128, 0]], dtype=np.uint8))
    assert rgb_bits(p, len(HEADER)).tolist() == [0x00FF8000]


def test_colors_add_the_rgb_field(tmp_path):
    p = tmp_path / "a.pcd"
    _pcd.write(p, np.zeros((1, 3)), np.zeros((1, 3)))
    lines = p.read_bytes().split(b"\n")
    assert lines[2:6] == [
        b"FIELDS x y z rgb",
        b"SIZE 4 4 4 4",
        b"TYPE F F F F",
        b"COUNT 1 1 1 1",
    ]


def test_float_colors_above_one_are_rejected(tmp_path):
    with pytest.raises(ValueError, match="colors"):
        _pcd.write(tmp_path / "a.pcd", np.zeros((1, 3)), np.array([[255.0, 0, 0]]))
