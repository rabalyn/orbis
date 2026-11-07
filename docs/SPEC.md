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
| O | Show or hide stars, planets and the Moon (globe only) |
| F | Switch the frame rate limit between 120 and 90 fps |
| Ctrl-F | Open the search box (see Milestone 7a) |
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
- No shadows cast by terrain or buildings, no clouds.

## Milestone 5b: the sky (done)

### Looking up
- Tilting past the horizon is possible (see Navigation), so the camera can look at the horizon and up
  into the sky from a view point on or above the ground. Tile selection and labels follow what is on screen.

### Sky colour and Sun
- Seen from inside the atmosphere the sky is a blue gradient, lighter at the horizon, that darkens
  with the Sun's altitude to night. A low Sun warms the sky around it and along the horizon.
  The effect fades out with height over about 15 km, giving way to the glow seen from space.
- The Sun is drawn as a small bright disc with a halo, reddened when low, from the ground and from space.
- With S (flat light) the sky stays at its daytime blue and shows no Sun.

### Stars, planets and the Moon
- About 5,000 stars down to magnitude 6 from the HYG database (J2000 positions, magnitudes, B-V colour),
  bundled as a compact table (see PROVENANCE.md). Stars are points whose size and brightness follow
  their magnitude, tinted by colour. They are drawn in one instanced draw.
- Stars are rotated by Greenwich mean sidereal time (Astronomical Almanac formula) and precessed from J2000
  to the date, so they turn with the clock.
- Mercury, Venus, Mars, Jupiter and Saturn come from JPL's approximate Keplerian elements (arcminutes,
  1800-2050) and the Moon from the Astronomical Almanac's low-precision lunar formulae (about 0.3°).
  Planets are points with their magnitudes; the Moon is a disc lit by the Sun, so it shows its phase,
  drawn at least 10 pixels wide. Names are drawn beside them while labels are on (L).
- Faint objects disappear as the sky brightens: at night within the air all of them show, in daylight only
  Venus and the Moon, and from space everything shows, even over the day side.
- O (or `--no-stars` at the start) hides stars, planets, the Moon and their names.
- The stars credit is shown while they are displayed, also in saved images.

### Limitations
- Positions are geocentric: the Moon can be up to 1° off for an observer, and the Moon, planets and
  stars are at infinity, so a camera far from the Earth does not see parallax.
- No Saturn rings in its brightness, no other planets, no constellation lines, no star names, no
  deep-sky objects, no twinkling and no light from the Moon or planets on the ground.
- The sky ignores terrain: a mountain does not hide a planet behind it, and names of planets behind
  a mountain are drawn anyway. There are no clouds or haze.
- The sky is not drawn on the flat map.

## Milestone 6a: other bodies, the Moon first (done)

### Choosing a body
- `--body earth` (the default), `moon`, `mars`, `mercury`, `ceres` or `vesta` selects the body for the whole run. Everything
  follows it: geodesy and altitude, the zoom limits, the whole-body view, the horizon, the tile grid,
  lighting and the sky. Earth's behaviour and requests are unchanged.
- Bodies are entries of an immutable table (`src/body.rs`) holding the semi-axes, the tile grid, whether
  there is air, which data exists, the rotation model, the Sun's direction, the sky objects and the
  credit. Adding a body is mainly a new entry and a source.

### The Moon
- A sphere of the IAU mean radius, 1737.4 km, with latitude and longitude as on Earth (east positive,
  longitude 0 towards the Earth). The HUD names the body.
- Imagery is the LRO Wide Angle Camera global mosaic from NASA Moon Trek, down to about 83 m per pixel
  at the equator (zoom 8). Its tiles are plate carrée (two tiles at zoom 0), so tile addressing and the
  meshes follow the body's grid and no resampling happens. The poles are covered.
- There is no air: no blue sky or glow, a sharp terminator and black shadows with only a trace of
  light on the night side. Stars stay visible by day and night.
- The Sun's direction comes from the clock through the IAU rotation model of the Moon (pole
  and prime meridian with their periodic terms), so phase, libration and the day and night side follow
  the time. Stars and planets are turned with the same frame. The Earth is a lit disc in the sky with
  its phase and name; the Moon itself is not listed.
- No roads, labels, buildings, national aerial imagery or terrain: R, L and B do nothing there.
- The atlas works for the Moon with the same projections, using its radius and grid.
- The tiles are cached apart from Earth's (`nasa-trek-moon-wac`), come from their own host with its own
  pacing, and the credit is shown while the Moon is in view, also in saved images.

### Limitations
- The Moon is smooth. NASA Moon Trek's LOLA elevation layer is served as 8-bit grey tiles whose height
  scaling the service does not document, about 80 m per step; it was not used rather than guess.
- Zoom 8 is the finest level of the service, so very close views are magnified.
- The imagery is lit by the program's Sun on top of its own illumination, so relief shows less at high
  Sun. The Sun is taken from the Earth's centre, up to 0.15 degrees off for the Moon.
- Libration is included, but the mosaic's frame is the mean Earth direction, not the principal axes.
- The Earth in the Moon's sky is a plain disc without clouds or oceans.
- No switching of body while running; restart with `--body`.

## Milestone 6b: Mars, Mercury, Ceres and Vesta (done)

These are table entries like the Moon's (`src/body.rs`), chosen with `--body mars|mercury|ceres|vesta`.
Each is a smooth body with no roads, labels, buildings, terrain or air, carrying one global
image from NASA's Trek services, a sharp terminator and stars in the sky.

- Shapes: Mars is the IAU 2015 spheroid (3396.19 km equatorial, 3376.20 km polar), Mercury a sphere of
  2439.4 km, Ceres a spheroid of 487.3 and 446.0 km, Vesta a spheroid of 284.5 and 229.0 km (its published
  figure is a triaxial 289, 280 and 229 km; the equatorial axes are averaged).
- Latitude and longitude are as on the Moon: longitude counted east, `--lat` and the HUD in geodetic
  (planetographic) latitude on the spheroid. The imagery is laid out in planetocentric latitude, so
  its rows are converted when the tiles are placed; features sit where the maps put them. Checked by eye:
  Olympus Mons on Mars at 18.65 N, 226.2 E, and Kuiper crater on Mercury at 11.3 S, 31.5 W, appear at the
  middle of the view at those coordinates, and a Ceres crater the size of Occator (19.8 N, 239.3 E) too.
- Imagery (all 256-pixel JPEG, plate carrée, two tiles at zoom 0, from `trek.nasa.gov`, cached apart per body):
  Mars the Viking MDIM 2.1 colour mosaic to zoom 7 (about 330 m per pixel at the equator), Mercury
  the MESSENGER MDIS basemap to zoom 7 (about 330 m), Ceres the Dawn framing camera HAMO colour-shaded mosaic
  to zoom 5 (about 1.3 km) and Vesta the Dawn HAMO mosaic to zoom 6 (about 700 m). The credit of each is shown
  while the body is in view, also in saved images.
- Rotation: the IAU Working Group's pole and prime meridian for each body, with Mars's and Mercury's periodic
  terms, from the 2015 report. The clock is read as TT (UTC plus 69.2 s).
- The Sun's direction: heliocentric positions of Mars and Mercury come from the JPL planet elements already
  used for the planets; Ceres and Vesta use two-body orbits from JPL Small-Body Database elements for the epoch
  2026-11-21. Against JPL Horizons the sub-solar point of Mercury and Mars is within 0.01 degrees at three
  dates in 2026 to 2031, Ceres within 0.6 degrees of longitude and Vesta within 1.4 (see Limitations).
- The sky: the Sun, stars, the other planets and the Earth (a point, with a magnitude), turned with the
  body's frame, so they move with its rotation. The Moon is not drawn from other bodies.
- Only the command line chooses the body; as for the Moon, switching needs a restart.

### Limitations
- Mars has no air: no blue-brown sky, no dust haze and soft dusk. A thin atmosphere needs its own scattering
  colours and was left out; Mars looks as the Moon does, with black shadows and visible stars by day.
- Vesta's published prime meridian is the Dawn "Claudia double prime" system (Archinal 2018); the Trek layer
  names the IAU coordinate system of Vesta without more detail. Only the poles could be checked by eye (the
  Rheasilvia basin sits at the south pole), not Vesta's or Ceres's longitudes against a named feature.
- Vesta's longitude differs by 1.0 to 1.4 degrees from Horizons, partly from its triaxial figure, which
  is a spheroid here; its shadow edge and horizon are slightly off. Ceres differs by up to 0.6 degrees
  from Horizons, from the orbit and the assumed frame.
- The orbits of Ceres and Vesta are two-body and drift from the real ones by up to 0.25 degrees from the
  Sun within three years of the epoch and more beyond; refresh the elements in `src/planetary.rs` with a newer
  epoch for years far from 2026.
- Imagery is not terrain: nothing rises above the spheroid. The flat map (atlas) places the rows in geodetic
  latitude, so Vesta in particular appears distorted there.
- The planets are points with approximate magnitudes (no phase for the Earth). Phobos, Deimos and the other
  moons are not drawn, and the Moon is not drawn from Mars. The Sun is a point source with no angular size.
- The imagery is lit on top of its own illumination, as for the Moon. Ceres' layer is coloured by elevation.

## Milestone 7a: search (done)

Ctrl-F opens a search box at the top left, under the same text path as the other panels.

- While it is open it takes every key: typed text (including non-ASCII letters and input-method
  commits), Backspace, Up and Down to choose a result, Enter to search or to fly to the chosen
  result, Escape to close. No other command runs meanwhile; the mouse still works.
- Coordinates are recognised locally, with no request, on any body: "46.5, 11.8", "46.5N 11.8E",
  "N 46 30 E 11 48", "46°30'N 11°48'E", with seconds too. Latitude comes first unless letters
  say otherwise. Enter flies there, to 10 km.
- Anything else is a place name, sent to OpenStreetMap's Nominatim on Enter only (never while typing).
  Requests are at least 1.1 s apart, run on a background thread, and go through the same
  server pauses as tiles. The box says "Searching...", "No place found." or that the server
  cannot be reached. Answers are cached on disk under the cache directory, by query and language,
  and an identical query is answered from there; with `--offline` only cached queries work.
- Up to eight results are listed, in the language of `--lang`. Enter flies to the chosen one with the
  existing flight, ending at a distance that fits its bounding box, north up, looking straight down.
  On the flat map the view moves there directly.
- While results are shown the credit `Search: Nominatim, © OpenStreetMap contributors` is in the panel
  and in the credits, saved images included. The panel is kept clear of labels.
- On bodies other than Earth, names are not searched: the box says so and suggests coordinates.
- `--search TEXT` does the same without a window: it flies to the first result (or the coordinates)
  before `--screenshot`. `--search-panel` also draws the open panel with its results.
  `--geocoder-url URL` switches the service.

### Limitations

- The built-in font draws only ASCII and Latin-1; other letters in the box or in results show as `?`,
  though they are searched for correctly.
- No editing within the text (no cursor movement, selection or paste); Backspace deletes the last character.
- Results are cached without an expiry, other than the cache's size limit.
- The user agent names the program but gives no contact address.
- The window's key handling was not driven by hand here; the logic is unit-tested.

## Later milestones

Order and scope are open, and depend on which features are wanted.

| # | Feature | Notes |
|---|---|---|
| M7b | Saved places | KML, KMZ, GeoJSON, GPX and IGC import |
| M8 | Map service library | User-added WMS, WMTS and XYZ, with an independently built directory |
| M9 | Events | Computed eclipses, plus earthquakes, eruptions, tsunamis and fireballs |
| M10 | SpaceMouse | Through HID or `spacenavd` |

Out of scope: Apple Vision Pro streaming, Quick Look and Share extensions, Sparkle updates.
