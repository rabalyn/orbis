# Orbis

A globe viewer for Linux, written in Rust with `wgpu`. It shows satellite imagery on real terrain with roads, labels and
3D buildings, from whole-Earth view down to street scale, lit by the Sun at any date, with the sky above it. "Orbis" is a working name.

This is an independent program with a similar purpose to Kugelblitz, not a port of it. The current version is an alpha.

## Documentation

- [docs/FEATURES.md](docs/FEATURES.md): everything the program does, all controls, and its limitations
- [docs/CACHING.md](docs/CACHING.md): how it treats public servers, the disk and LAN caches, and imagery sources
- [docs/PROVENANCE.md](docs/PROVENANCE.md): where data and methods come from, their terms and credits

## Run

Requires a Rust toolchain and a Vulkan-capable GPU.

```bash
cargo run --release
```

It opens on Germany's energy bars at noon on 1 November 2026, the clock paused; see [Start view](docs/FEATURES.md#start-view).

Start somewhere specific:

```bash
cargo run --release -- --lat 46.5 --lon 11.8 --distance 25000 --tilt 70 --heading 30
```

Write an image without opening a window:

```bash
cargo run --release -- --lat 46.5 --lon 11.8 --distance 25000 --tilt 70 --screenshot alps.png
```

`--help` lists all options. Press H, ? or F1 in the window for all keyboard and mouse controls. If your desktop reserves Alt-drag for moving windows, use right-drag to turn and tilt.

## What it does

| Feature | How to reach it |
|---|---|
| Move, zoom, tilt, and look up into the sky | Drag, scroll, right-drag or Alt-drag |
| Fly to a place or label | Double-click |
| Wikipedia article and Wikidata facts for a place; population and electricity for a country | Click |
| Roads, names and 3D buildings | R, L and B; V for each kind |
| Roofs from the aerial imagery, or buildings coloured by zone | V, Building colours |
| View options, saved between runs | V, or the second icon |
| Flat map in five projections | M, then P |
| Sun, day and night, cast shadows | Follow the clock; S for flat, always-lit imagery |
| Clock | Space pauses, - and = change its speed, comma and period step it, T returns to now; `--time 2026-06-21T12:00:00Z` |
| Time window: sliders for year, day and time of day, 1800 to 2050 | C, or the clock icon |
| Stars, planets and the Moon | O hides them |
| Search for a place or coordinates | Ctrl-F |
| Saved places | Ctrl-S saves, K lists |
| GeoJSON, GPX, KML, KMZ and IGC files | `--import FILE` or drop on the window; I lists |
| Electricity of the world's countries by source and month, Germany's plants down to each site, output and use | W; on the flat map, countries shaded by renewable share |
| Solar and lunar eclipses, computed in the program, and earthquake, eruption, tsunami and fireball feeds | E; `--eclipse 2024-04-08` |
| Map services (WMS, WMTS or XYZ) as imagery or overlay | G; `--add-service ADDRESS`, `--list-services` |
| The Moon, Mars, Mercury, Ceres and Vesta | `--body moon` |
| 3Dconnexion SpaceMouse, not yet tried on a real device | Used when found; see [FEATURES.md](docs/FEATURES.md#spacemouse) for permissions |

Names appear in your locale's language; `--lang de` picks another one. Dense city views used about 2 GB of graphics memory at 1280x800, and larger windows load more.

## Configuration

View options are saved in `settings.json`: in `./config/` when the working directory has that folder (the committed `config/settings.json` holds the defaults, and is what a checkout of the source uses), otherwise in `orbis/` in your config folder, or in the folder given with `--config-dir`. Flags apply to one run and are not saved. A `--screenshot` ignores saved options unless `--config-dir` is given. Saved places and map services stay in your own config folder.

## Data and cache

Orbis tries hard not to burden the public services it uses. Tiles are kept in `~/.cache/orbis/tiles`, limited to 10 GiB, and a server that reports trouble is left alone for 15 minutes. Machines on a LAN can share tiles:

```bash
cargo run --release -- --serve-cache
```

```bash
cargo run --release -- --cache-server HOST:47813
```

Imagery is Sentinel-2 at 10 m per pixel worldwide, and aerial photos near the ground in the USA and much of Europe, including Germany, France, Spain, Austria and Switzerland.

The base imagery is free for non-commercial use only, and credits must stay visible. See [docs/PROVENANCE.md](docs/PROVENANCE.md) before publishing or using this commercially.

## Browser version

The same program also runs in a web browser, as one self-contained `index.html` (about 16 MB) that can be put on GitHub Pages or any static host.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130   # must match wasm-bindgen in Cargo.lock
./web/build.sh                                      # writes dist/index.html
```

To publish it on GitHub Pages ([rabalyn/orbis](https://github.com/rabalyn/orbis), served at https://rabalyn.github.io/orbis/), commit, then run:

```bash
./web/publish.sh            # --dry-run to push nothing; --help for all options
```

It builds the last commit in a clean checkout, pushes the page, waits for the deploy and checks that the live file is the one built. It stops if tracked files have uncommitted changes. Exit status 0 means published or already up to date, and the last line says which.

It uses WebGPU where the browser has it, and WebGL 2 otherwise. Options go in the address, as for the command line: `index.html?lat=46.5&lon=11.8&distance=25000&tilt=70&time=2026-06-21T10:00:00Z`.

What differs from the desktop program:

- Tiles are kept in memory for the visit only, and view options are not saved.
- No LAN cache sharing, SpaceMouse, file import, `--screenshot` or `--record`.
- A server must allow requests from other sites. The fireball feed (NASA/JPL) does not, and some national aerial imagery may be missing for the same reason.
- Labels have Latin, Greek and Cyrillic letters only (bundled Noto Sans, see `web/fonts/OFL.txt`); a browser does not let a page use the system's fonts.
- Everything runs on the page's one thread, between frames, so loading is slower than on the desktop.

A published page sends every visitor's requests straight to the public servers, without the shared cache, so check their usage policies before sharing a link widely. The imagery credits must stay visible, as on the desktop.

## Test

```bash
cargo test
```
