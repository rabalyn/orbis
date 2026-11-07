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

The globe is lit by the Sun at the current time (shown in UTC). Space pauses the clock, - and = change its speed, comma and period step it, T returns to now, and S switches to flat, always-lit imagery. `--time 2026-06-21T12:00:00Z` starts at another time. Tilt past the horizon to look up at the sky, with stars, planets and the Moon; O hides them (`--no-stars`).

Ctrl-F opens a search box: type a place name or coordinates such as `46.5, 11.8`, press Enter, choose with Up and Down, and Enter again to fly there. Names are looked up on OpenStreetMap's Nominatim, only when you press Enter.

Ctrl-S saves the current view as a named place, and K lists the saved places to fly to or delete (kept in `orbis/places.json` in your config folder). `--import FILE`, or dropping a file on the window, shows the contents of a GeoJSON, GPX, KML, KMZ or IGC file on the Earth; I lists its items.

E lists the solar and lunar eclipses around the clock's date, computed in the program: Enter jumps to the greatest moment, with the central line drawn and the shadow darkening the globe (`--eclipse 2024-04-08`).

G lists map services (WMS, WMTS or XYZ) to use as the Earth's imagery or to lay over it. Add one with `--add-service ADDRESS` (kept in `orbis/services.json`), and choose with `--service NAME` or `--service-overlay NAME`; `--list-services` shows your library and a built-in directory of public services.

The second icon (or V) opens the view options: map lines, names, scene, globe or flat map, projection, imagery and frame rate. The question-mark icon in the bottom right corner (or H, ? or F1) shows a window listing all keyboard and mouse controls; Escape closes it. For saved images, `--show-help`, `--show-options`, `--help-scroll N` and `--simulate-click X,Y` show them, scroll and click.

`--body moon|mars|mercury|ceres|vesta` shows that body instead of the Earth, lit by the Sun at the clock's time, with its own imagery and the planets in its sky.

A 3Dconnexion SpaceMouse is used when found, through `spacenavd` or `/dev/hidraw*` (for the latter, a udev rule such as `KERNEL=="hidraw*", ATTRS{idVendor}=="256f", TAG+="uaccess"` gives your user access). It flies, turns and tilts the view, and its two buttons show the whole body and straighten up. It has not yet been tried on a real device. `--no-spacemouse` turns it off and `--spacemouse-sensitivity N` scales its speed.

`--help` lists all options. If your desktop reserves Alt-drag for moving windows, use
right-drag to turn and tilt.

View options are saved in `settings.json`: in `./config/` when the working directory has that folder (the committed `config/settings.json` holds the defaults and is what a checkout of the source uses), otherwise in `orbis/` in your config folder, or in the folder given with `--config-dir`. Flags apply to one run and are not saved; a `--screenshot` ignores saved options unless `--config-dir` is given. Saved places and map services stay in your own config folder.

## Data and cache

Orbis tries hard not to burden the public services it uses. Tiles are kept in
`~/.cache/orbis/tiles`, limited to 10 GiB, and a server that reports trouble is left alone
for 15 minutes. Machines on a LAN can share tiles:

```bash
cargo run --release -- --serve-cache
```

```bash
cargo run --release -- --cache-server HOST:47813
```

Imagery is Sentinel-2 at 10 m per pixel worldwide, and aerial photos near the ground in
the USA and much of Europe, including Germany, France, Spain, Austria and Switzerland.
[docs/CACHING.md](docs/CACHING.md) has the details.

The base imagery is free for non-commercial use only, and credits must stay visible. See
[docs/PROVENANCE.md](docs/PROVENANCE.md) before publishing or using this commercially.

## Test

```bash
cargo test
```
