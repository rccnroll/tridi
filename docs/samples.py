"""The synthetic files behind docs/*.png: python3 docs/samples.py writes them
to ./samples (terrain.pcd split in four tiles is the viewer shot; Nautilus
shows the rest, plus tests/data/assembly.step and open3d_box.glb as box.glb).
Needs numpy."""
import numpy as np, struct, os
rng = np.random.default_rng(7)
out = "samples"
os.makedirs(out, exist_ok=True)

def pcd(path, pts):
    pts = pts.astype(np.float32)
    h = ("# .PCD v0.7\nVERSION 0.7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\n"
         f"WIDTH {len(pts)}\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS {len(pts)}\nDATA binary\n")
    open(path, "wb").write(h.encode() + pts.tobytes())

# rolling terrain with a river valley, like an aerial scan
n = 400_000
xy = rng.uniform(-50, 50, (n, 2))
x, y = xy[:, 0], xy[:, 1]
z = (6*np.exp(-((x-15)**2+(y-10)**2)/300) + 9*np.exp(-((x+20)**2+(y+15)**2)/200)
     + 1.2*np.sin(x/6)*np.cos(y/7) - 4*np.exp(-((y - 8*np.sin(x/12))**2)/20)
     + rng.normal(0, 0.05, n))
pcd(f"{out}/terrain.pcd", np.c_[x, y, z])

# torus knot (2,3) as a thick point cloud
t = rng.uniform(0, 2*np.pi, 150_000)
r = 2 + np.cos(3*t)
c = np.c_[r*np.cos(2*t), r*np.sin(2*t), -np.sin(3*t)]
pcd(f"{out}/knot.pcd", c + rng.normal(0, 0.12, c.shape))

# helix with its own rgb, as .xyzrgb
t = np.linspace(0, 12*np.pi, 60_000)
h = np.c_[np.cos(t)*(1+0.3*np.cos(9*t)), np.sin(t)*(1+0.3*np.cos(9*t)), t/6 + 0.3*np.sin(9*t)]
col = np.c_[(np.sin(t/3)+1)/2, (np.sin(t/3+2.1)+1)/2, (np.sin(t/3+4.2)+1)/2]
np.savetxt(f"{out}/helix.xyzrgb", np.c_[h, col], fmt="%.4f")

# torus knot tube as a binary STL mesh
def tube(N=600, M=24, R=0.35):
    t = np.linspace(0, 2*np.pi, N, endpoint=False)
    def curve(t):
        r = 2 + np.cos(3*t); return np.c_[r*np.cos(2*t), r*np.sin(2*t), -np.sin(3*t)]
    p = curve(t); d = curve(t + 1e-3) - p; d /= np.linalg.norm(d, axis=1)[:, None]
    a = np.cross(d, [0, 0, 1]); a /= np.linalg.norm(a, axis=1)[:, None]; b = np.cross(d, a)
    s = np.linspace(0, 2*np.pi, M, endpoint=False)
    v = p[:, None] + R*(np.cos(s)[None, :, None]*a[:, None] + np.sin(s)[None, :, None]*b[:, None])
    tris = []
    for i in range(N):
        for j in range(M):
            A, B = v[i, j], v[(i+1) % N, j]; C, D = v[(i+1) % N, (j+1) % M], v[i, (j+1) % M]
            tris += [(A, B, C), (A, C, D)]
    return np.array(tris, dtype=np.float32)
tr = tube()
with open(f"{out}/knot-tube.stl", "wb") as f:
    f.write(b"\0"*80 + struct.pack("<I", len(tr)))
    for a, b, c in tr:
        nrm = np.cross(b-a, c-a); nrm /= (np.linalg.norm(nrm) or 1)
        f.write(struct.pack("<12fH", *nrm, *a, *b, *c, 0))

# a torus as OBJ
N, M = 96, 48
u, v = np.meshgrid(np.linspace(0, 2*np.pi, N, endpoint=False), np.linspace(0, 2*np.pi, M, endpoint=False), indexing="ij")
P = np.stack([(3+np.cos(v))*np.cos(u), (3+np.cos(v))*np.sin(u), np.sin(v)], -1).reshape(-1, 3)
with open(f"{out}/torus.obj", "w") as f:
    f.writelines(f"v {a:.4f} {b:.4f} {c:.4f}\n" for a, b, c in P)
    for i in range(N):
        for j in range(M):
            k = lambda i, j: (i % N)*M + (j % M) + 1
            f.write(f"f {k(i,j)} {k(i+1,j)} {k(i+1,j+1)} {k(i,j+1)}\n")

# the viewer shot: the terrain in four tiles, one color each
for k, (sx, sy) in enumerate([(0, 0), (1, 0), (0, 1), (1, 1)]):
    m = ((x >= 0) == sx) & ((y >= 0) == sy)
    pcd(f"{out}/tile_{k+1}.pcd", np.c_[x, y, z][m])
