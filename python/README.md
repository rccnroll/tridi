# tridi

Point clouds, meshes and STEP files in a window, from Python: a drop-in for
Open3D's `draw_geometries`, with STEP and nothing else to install.

    pip install tridi

```python
import open3d as o3d
import tridi

pcd = o3d.io.read_point_cloud("scan.pcd")
pcd = pcd.voxel_down_sample(0.01)          # whatever processing you do
tridi.show(pcd, "part.step")               # instead of draw_geometries([pcd])
```

`show()` takes any mix of:

- file paths: pcd, ply, xyz, xyzrgb, pts, glb, gltf, obj, stl, off, step, stp
- an Open3D `PointCloud`, or anything with `.points` (and `.colors`)
- an `(N, 3)` numpy array
- a `(points, colors)` tuple; colors `(N, 3)`, floats in [0, 1] or uint8

It returns when the window is closed. Each item gets its own color and a row
in the legend; shift + click picks a point and prints it on stdout. `H` in
the window, or `tridi --help`, lists the keys. A bad array raises
`ValueError` before any window opens; a window that can't open (no display,
no file readable) raises `tridi.TridiError` with what tridi said.

The window is the tridi program, run as its own process: OpenGL and
OpenCASCADE never load into Python, and the `tridi` command comes with the
package.

## Requirements

Linux x86_64 with a desktop session (Wayland or X11) and the system's OpenGL
drivers: Debian/Ubuntu `libegl1 libgl1-mesa-dri`, Arch `mesa`, Fedora
`mesa-libEGL mesa-dri-drivers`. Any desktop has them already.

## License

MIT or Apache-2.0. The wheel bundles OpenCASCADE's libraries, LGPL-2.1 with
its exception; their license is in `tridi/_bin/licenses/`.

More: https://github.com/rccnroll/tridi
