# Orbis

A globe viewer for Linux, written in Rust with `wgpu`. It shows satellite imagery on real
terrain with roads, labels and 3D buildings, from whole-Earth view down to street scale.
"Orbis" is a working name.

This is an independent program with a similar purpose to Kugelblitz, not a port of it.
See [docs/PROVENANCE.md](docs/PROVENANCE.md) for how it is built and
[docs/SPEC.md](docs/SPEC.md) for its features, controls and later milestones.

## Run

Requires a Rust toolchain and a Vulkan-capable GPU.

```bash
cargo run --release
```

Start somewhere specific:

```bash
cargo run --release -- --lat 46.5 --lon 11.8 --distance 25000 --tilt 70 --heading 30
```

Write an image without opening a window:

```bash
cargo run --release -- --lat 46.5 --lon 11.8 --distance 25000 --tilt 70 --screenshot alps.png
```

F switches the frame rate limit between 120 and 90 fps, up to what the display can show.

Names appear in your locale's language; `--lang de` picks another one. R, L and B show or
hide roads, labels and buildings. Dense city views used about 2 GB of graphics memory at 1280x800; larger windows load more.

`--help` lists all options. If your desktop reserves Alt-drag for moving windows, use
right-drag to turn and tilt.

## Data and cache

Tiles are stored in `~/.cache/orbis/tiles`, which is limited to 10 GiB. Imagery is
Sentinel-2 at 10 m per pixel, so close-up views look soft.

The imagery is free for non-commercial use only, and its credit must stay visible. See
[docs/PROVENANCE.md](docs/PROVENANCE.md) before publishing or using this commercially.

## Test

```bash
cargo test
```
