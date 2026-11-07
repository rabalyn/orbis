# Feature spec

Goal: a Linux desktop viewer for Earth and other bodies, as a 3D globe and a flat atlas,
fed from open map data. Behaviour is specified here in our own words. See
[PROVENANCE.md](PROVENANCE.md) for the rules on how it is built.

## Milestone 1: the globe (done)

### Display
- WGS84 globe from whole-Earth view down to street scale, with satellite imagery on real terrain.
- Detail loads progressively. A coarse tile stays visible until all of its finer
  replacements are ready, so the view never shows holes while loading.
- A simple atmosphere glow, and plain colour where no imagery exists (the poles).
- Always-visible data credits, plus the view point's coordinates and the camera altitude.
- The display says whether tiles are still loading or waiting to retry, or all have loaded.

### Navigation
The view is defined by a view point on the ground, a distance, a heading and a tilt.

| Input | Action |
|---|---|
| Drag | Move the globe, keeping the grabbed point under the pointer; it glides after release |
| Right-drag or Alt-drag | Turn around the view point (horizontal), tilt (vertical) |
| Shift-drag | Climb or sink (vertical), look around from the same spot (horizontal) |
| Scroll | Zoom around the point under the pointer |
| Alt-scroll / Shift-scroll | Tilt / look around |
| Double-click, Shift-double-click | Fly one level closer or farther, centring on that point |
| Arrow keys | Fly forward, back and sideways; speed builds while held |
| Alt-arrows | Tilt and turn around the view point |
| Shift-arrows | Climb, sink and look around |
| Ctrl-Up, Ctrl-Down | One level closer or farther |
| Ctrl-0 | Whole Earth, or the whole map on the map |
| Ctrl-Alt-N | Straighten up, one step per press: north up, straight down, whole body, equator centred |
| Escape | Stop a flight or a gliding movement |
| R | Show or hide roads and boundaries |
| L | Show or hide labels |
| B | Show or hide 3D buildings (globe only) |
| M | Switch between the globe and the flat map, keeping place and scale |
| P, Shift-P | On the map: next or previous projection |
| [ and ] | On the map: move the central meridian west or east by 15°, or 1° with Shift |
| Space | Pause or resume the clock |
| - and = | Slow down or speed up the clock (1, 10, 60, 600 times, then hourly up to monthly per second) |
| , and . | Step the clock back or forward by one hour, or one day with Shift |
| T | Set the clock back to the real time, running at normal speed |
| S | Switch between sunlight and flat, always-lit imagery |
| F | Switch the frame rate limit between 120 and 90 fps |
| Ctrl-Q | Quit |

An arrow key keeps its meaning while held, even if modifiers change. Moving by hand ends a
flight. Manual flight holds altitude and only rises when the ground comes within 2 m.

Tilt orbits the view point down to 85° from straight down. Tilting further raises the head about
the camera, which stays where it is, through the horizon up to straight up. Tile selection and labels
follow what the screen shows, so the ground may fill only part of it.

### Data and cache
- Tiles are fetched only when a view needs them, on background threads, nearest and coarsest first.
- Tiles persist in a disk cache and work offline once cached. Oldest-used tiles are removed
  above the size limit, and anything used in the last six hours is kept.
- A server that answers with an overload or server error is paused rather than flooded. Failed tiles are retried with growing delays.

## Milestone 2: roads and labels (done)

### Lines
- Roads, railways, ferries, and country and state boundaries, draped over the terrain.
- Detail follows the zoom level: motorways first, minor roads later, footpaths only when close.
- Tunnels and disputed boundaries are dashed.

### Labels
- Countries, states, cities, towns, villages, districts, seas, lakes, peaks with their
  heights, and road names or numbers. Road labels follow their road's direction and stay upright.
- Names use the language from `--lang` or the locale, falling back to the local name. Any
  script the system's fonts cover is shown.
- Labels never overlap and stay clear of the on-screen panels. They do not hang over the
  edge of the globe, and mountains hide what is behind them. The more important label wins.

### Failure behaviour
Map data is independent of imagery. If its server is unreachable, the globe keeps working
without lines and labels, and they appear once the server answers again.

### Limitations
- Lines are drawn into per-tile images, so their width steps slightly between zoom levels,
  and they soften when very close.
- Road labels are straight, not curved along the road.
- Label quality follows OpenStreetMap, including occasional wrong or vandalised entries.

## Milestone 3: 3D buildings (done)

- Buildings rise from OpenStreetMap footprints with their mapped heights, colours,
  courtyards and raised parts such as stepped towers.
- They stand on the terrain: roofs stay level, and walls reach down to the low side of a slope.
- In an 800-pixel-high view looking straight down, they appear somewhere between 25 and 15 km
  from the ground. Larger windows load detail, and so buildings, from further away.
- Only outward faces are drawn, so a camera inside a building sees out.
- Edges are anti-aliased, where the GPU supports it.
- A chosen start distance counts from the ground, not from sea level.

### Limitations
- Flat roofs and plain colours: no textures, windows or real roof shapes.
- Height and colour are only as good as the map data, and buildings without a height get a default.
- Buildings end along a line in the far distance, because height data exists only at one zoom level.
- The camera does not collide with buildings, and labels show through them.
- A building crossing a tile edge can show a small step if the ground differs between its two parts.
- Lighting follows the sun (see Milestone 5a).

## Milestone 4: atlas (done)

- M switches to a flat map of the Earth and back, or `--atlas PROJECTION` starts with it. The same
  imagery, the same coarse-before-fine tile replacement and the same road and boundary lines are used.
- Projections: Equal Earth, Mollweide, Hammer-Aitoff, Mercator and Plate Carrée (`equal-earth`,
  `mollweide`, `hammer-aitoff`, `mercator`, `plate-carree`). P cycles through them.
- The central meridian is the longitude at the middle of the map. `--central-meridian DEG` sets it, and
  `[` and `]` move it. Tiles that cross the seam on the far side are cut there, so nothing wraps wrongly.
- Every tile is drawn as a fine grid whose corners are projected exactly, so the picture is correct over
  the whole map. Nothing is drawn outside the map's outline.
- Mercator ends at 85.05° north and south, which is also where the imagery ends, so its map is square.
  The other projections show the polar caps in a plain colour (ice in the south, ocean in the north).
- Drag moves the map, scroll zooms around the pointer, double-click centres and zooms in (Shift zooms out).
  Dragging stops at the edge of the map and at the poles. Zooming out stops when the whole map fits.
- Tiles are chosen for the scale on screen, about one texel per pixel, and only those in the window.
  At most 700 tiles are selected for a view.
- Credits and the status panel stay. The coordinates are those of the point at the middle of the window,
  and the status gives the projection, the central meridian and the metres per pixel at the equator.
- Switching keeps the place and the scale; the whole-Earth globe gives the whole map.
- `--lat`, `--lon` and `--distance` apply to the map as well: a `--distance` of the whole Earth or more shows the whole map.

### Limitations
- Labels, 3D buildings and terrain relief are not shown on the map.
- The world is not repeated sideways: the map has one edge on each side, and dragging stops there.
- Arrow keys do nothing on the map, and there is no glide after a drag.
- The metres per pixel in the status are true only at the equator; other latitudes differ by projection.
- Zooming past the finest imagery magnifies its texels, at most twofold.

## Caching, tile servers and imagery detail

See [CACHING.md](CACHING.md): server timeouts, the local cache, the LAN cache server, and
sharper imagery near the ground.

## Frame rate

- The frame rate is limited to 120 fps (the default) or 90 fps, chosen with F or `--fps`.
- The on-screen display shows the limit, the measured rate and the display's refresh rate.
- Frames are drawn only when something changes, so an idle window uses next to nothing.
- A window cannot be shown faster than its display refreshes: on a 60 Hz monitor it runs at 60 fps.
- `--benchmark SECONDS` turns the view and prints frame timings, for measuring without input.

## Milestone 5a: time, sun, day and night (done)

### Clock
- A simulated clock starts at the real time and is shown in UTC in the top-right display, with
  "paused" or the speed when it is not running at normal speed.
- It can be paused, sped up or slowed down, stepped by hours or days, and set back to now.
- `--time ISO8601` sets the start (`2026-06-21T12:00:00Z`, with or without an offset or a
  time; without one it is UTC); `--time-speed N` sets clock seconds per real second.
  Saved images (`--screenshot`) show one moment, with the clock paused.

### Sun and lighting
- The Sun's place comes from the Astronomical Almanac's low-precision formulae, good to about 0.01°
  of arc from 1950 to 2050. The subsolar point and the direction to the Sun are in the Earth-fixed frame.
- The globe, its terrain, 3D buildings and the flat map are lit by the Sun. Day turns to night
  over a soft terminator with twilight, reaching full night when the Sun is about 14° below the horizon.
  Low sunlight is warm.
- Terrain is shaded by its slope, so slopes facing away from a low Sun fall into shadow. Buildings
  are shaded by the direction each face points. Cast shadows are not computed.
- The night side stays dimly readable. There are no city lights.
- The atmosphere glow is bright on the day side, warm at dusk and faint at night.
- With S (or `--no-sun`) everything is shown flat and fully lit, so imagery can be read at night.
- The picture is redrawn only when the Sun has moved 0.05° (about 12 s at normal speed), so an
  idle window uses almost no CPU. Speeds of 600 times and above redraw continuously.

### Limitations
- Imagery already contains the light of the day it was taken, so lit slopes are lit twice.
- The clock ignores leap seconds and the difference between UT and terrestrial time (under 70 s).
- No shadows cast by terrain or buildings, no clouds, no sunrise colours in the sky from the ground.
- Stars, planets and looking up into the sky are still to come.

## Later milestones

Order and scope are open, and depend on which features are wanted.

| # | Feature | Notes |
|---|---|---|
| M5b | Sky | Stars, planets, looking up into the sky (time, sun, day and night are done) |
| M6 | Other bodies | Moon, Mars, Mercury, Ceres, Vesta |
| M7 | Search and saved places | Geocoding, plus KML, KMZ, GeoJSON, GPX and IGC import |
| M8 | Map service library | User-added WMS, WMTS and XYZ, with an independently built directory |
| M9 | Events | Computed eclipses, plus earthquakes, eruptions, tsunamis and fireballs |
| M10 | SpaceMouse | Through HID or `spacenavd` |

Out of scope: Apple Vision Pro streaming, Quick Look and Share extensions, Sparkle updates.
