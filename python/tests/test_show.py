import os
import signal
import threading
import time
from types import SimpleNamespace

import numpy as np
import pytest

import tridi


@pytest.fixture
def fake(tmp_path, monkeypatch):
    """A stand-in for the tridi binary: records its arguments, copies the
    files it was given, and does what `mode` says (exit 0, fail, or hang)."""
    out = tmp_path / "out"
    out.mkdir()

    def make(mode="ok"):
        exe = tmp_path / "tridi"
        tail = {
            "ok": "exit 0",
            "fail": "echo boom >&2; exit 1",
            "hang": f"echo $$ > {out}/pid; exec sleep 30",
        }[mode]
        exe.write_text(
            "#!/bin/sh\n"
            f'printf "%s\\n" "$@" > {out}/argv\n'
            f'for f in "$@"; do [ -f "$f" ] && cp "$f" {out}/; done\n'
            f"{tail}\n"
        )
        exe.chmod(0o755)
        monkeypatch.setattr(tridi, "_binary", lambda: exe)
        return out

    return make


def argv(out):
    return (out / "argv").read_text().splitlines()


def test_paths_and_clouds_in_order(fake):
    out = fake()
    tridi.show("a.pcd", np.zeros((2, 3)), "b.step")
    a = argv(out)
    assert a[0] == "a.pcd" and a[2] == "b.step"
    assert os.path.basename(a[1]) == "cloud-1.pcd"
    assert (out / "cloud-1.pcd").read_bytes().startswith(b"# .PCD")
    assert not os.path.exists(os.path.dirname(a[1])), "the temp dir is gone"


def test_open3d_like_object_is_accepted(fake):
    out = fake()
    tridi.show(SimpleNamespace(points=np.zeros((2, 3)), colors=np.ones((2, 3))))
    assert b"FIELDS x y z rgb" in (out / "cloud-1.pcd").read_bytes()


def test_open3d_cloud_without_colors(fake):
    out = fake()
    # Open3D gives an empty colors array when there are none
    tridi.show(SimpleNamespace(points=np.zeros((2, 3)), colors=np.zeros((0, 3))))
    assert b"FIELDS x y z\n" in (out / "cloud-1.pcd").read_bytes()


def test_points_and_colors_tuple(fake):
    out = fake()
    tridi.show((np.zeros((1, 3)), np.ones((1, 3))))
    assert b"rgb" in (out / "cloud-1.pcd").read_bytes()


@pytest.mark.parametrize(
    "item, msg",
    [
        ((np.zeros((2, 3)), np.ones((3, 3))), "2 points, 3 colors"),
        (np.zeros((0, 3)), "no points"),
        (np.zeros((2, 2)), r"\(N, 3\)"),
    ],
    ids=["color_count_must_match", "empty_cloud_is_rejected", "points_must_be_n_by_3"],
)
def test_bad_input_raises_before_spawning(fake, item, msg):
    out = fake()
    with pytest.raises(ValueError, match=msg):
        tridi.show(item)
    assert not (out / "argv").exists()


def test_failure_raises_with_stderr_and_cleans_up(fake):
    out = fake("fail")
    with pytest.raises(tridi.TridiError) as e:
        tridi.show(np.zeros((1, 3)))
    assert "boom" in e.value.stderr
    assert not os.path.exists(os.path.dirname(argv(out)[0]))


def test_interrupt_kills_child_and_cleans_up(fake):
    out = fake("hang")
    # what Ctrl+C sends
    threading.Timer(0.5, os.kill, (os.getpid(), signal.SIGINT)).start()
    start = time.monotonic()
    with pytest.raises(KeyboardInterrupt):
        tridi.show(np.zeros((1, 3)))
    assert time.monotonic() - start < 10, "not waited out"
    pid = int((out / "pid").read_text())
    time.sleep(0.1)
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)
    assert not os.path.exists(os.path.dirname(argv(out)[0]))
