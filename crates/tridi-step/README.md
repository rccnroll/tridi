# tridi-step

STEP files (.step, .stp) tessellated by the system's OpenCASCADE, 7.6 or later: each solid comes back as triangles, placed where the assembly puts it, with the file's colors.

```rust
let skipped = tridi_step::load(Path::new("part.step"), [0.5; 3], Purpose::View, &|| false, |m| {
    println!("{} triangles", m.positions.len() / 3);
})?;
```

`load` hands each solid to the closure as soon as it's done, as a `StepMesh`: `positions` and `normals`, three vertices per triangle with no index, and `colors` (sRGB 0..1, one per vertex) only if the file has any; the `[0.5; 3]` stands in for the faces without one. It returns how many solids it gave up on. `Purpose::View` makes a fine pass, then a coarse one for what it missed; `Purpose::Thumbnail` a coarse one, and whatever came in 15 s. Return true from `stop` (the window was closed) and the work is killed: `load` returns `StepError::Cancelled`.

## A helper process

OpenCASCADE runs in a small helper program, not in your process: a part can take minutes to heal, C++ can crash, memory can run away, and a thread can't be stopped where a process can be killed. The helper is killed once no solid has come for a while, or when `stop` says so, and dies with your program.

The helper is built with the crate and embedded in it: there is nothing to install next to your binary and nothing to call at startup. On first use it runs from an in-memory file; where the system forbids that (`vm.memfd_noexec = 2`), from a copy in `~/.cache/tridi/` (or the temp dir).

## OpenCASCADE

Building needs its headers and libraries; point `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR` at them if they aren't where the distributions put them.

| | build | run |
|---|---|---|
| Arch, Fedora | `opencascade` | `opencascade` |
| Debian, Ubuntu | `libocct-data-exchange-dev libocct-ocaf-dev libocct-modeling-algorithms-dev libocct-modeling-data-dev libocct-foundation-dev` | `libocct-data-exchange-7.6 libocct-ocaf-7.6 libocct-modeling-algorithms-7.6 libocct-modeling-data-7.6 libocct-foundation-7.6` (`-7.6t64` on Ubuntu 24.04) |

Linux only.
