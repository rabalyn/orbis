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
| Ctrl-S | Name and save the current view as a place (see Milestone 7b) |
| K | Show or hide the list of saved places |
| I | Show or hide the list of imported items |
| E | Show or hide the list of eclipses around the clock's date (see Milestone 9a). In it: Enter sets the clock to the greatest moment, pauses it and flies to the greatest point |
| G | Show or hide the list of map services (see Milestone 8). In it: Enter uses the chosen service as the base layer (again: back to the usual imagery), O as the overlay, + and - change the overlay's opacity, A opens the box to add a service, Delete removes one |
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

## Milestone 7b: saved places and file import (done)

### Saved places

- Ctrl-S opens the search box in a naming mode ("Save place as:"). Enter keeps the current view under that
  name; an empty name becomes the position, for example `46.5000°N 11.8000°E` (prefixed with the body's name
  away from Earth). Escape cancels.
- A place stores the body, position, distance, heading and tilt. On the map the heading and tilt are
  stored as zero and the distance is the one that matches the map's scale.
- K opens a list at the top left. Up and Down choose, Enter flies there (with its heading and tilt on the
  globe, to its position and scale on the map), Delete or Backspace removes, K or Escape closes. While the list
  is open these keys do not move the view; all others work as usual.
- A place on another body than the one being viewed is listed with that body's name; choosing it shows a
  message saying which `--body` to start with.
- The places are one JSON file, `orbis/places.json` under the user's config directory (`--places-file PATH`
  changes it). It is written to a temporary file in the same folder, flushed and renamed over the old one.
- A file that cannot be read (not JSON, an entry damaged, written by a newer version, too large) is renamed to
  `places.json.bak` (or `.bak.1`, ... if that exists), the list starts empty, and a message on screen says so.
  If it cannot be moved aside, nothing is written, so it is not overwritten.
- `--save-place NAME` saves the view before a saved image is made and `--places-panel` draws the list.

### File import

- `--import FILE` (repeatable) reads a file at start-up, and a file dropped on the window is read and the view
  flies to it. The format is recognised from the contents, not the name: GeoJSON (RFC 7946, any geometry,
  Features and FeatureCollections), GPX 1.1 (waypoints, each track segment, routes), KML 2.2 (Placemarks with
  Point, LineString, LinearRing, Polygon with holes and MultiGeometry, inside any Folders and Documents), KMZ
  (a zip; `doc.kml`, else the first `.kml`) and IGC (the B records as one line with the GNSS altitude).
- Each file gives points, lines and polygons with a name (the placemark, feature or waypoint name; for IGC the
  pilot and glider from the H records and the date) and the altitude where the file has one.
- Files are not trusted. A file over 64 MiB is refused, so is a KMZ whose KML would be over 64 MiB once unpacked
  (the unpacking stops at the limit, whatever the zip claims). A KMZ is only read in memory; its entry names are
  never used as paths, and names that climb out of a folder are skipped. At most 50 000 items and 3 million
  positions are kept per file, and the message says if the rest was left out. XML entities other than the
  predefined and numeric ones are not expanded, nesting is limited, and positions that are not on the globe
  are skipped. Damaged files give a message on screen, never a crash.
- I lists the imported items at the top left; Enter flies to the chosen item's bounding box (a point to 4 km).
- Imports are for Earth. On another body a message says so and nothing is read. Files do not name a body, so
  none is assumed other than Earth.

### Showing imports

- Lines and areas are drawn into the tile overlays, with the roads and on top of them, so they follow the
  terrain on the globe and on the map. Lines are red with a dark edge; areas have a translucent yellow fill
  (holes stay open) and a yellow outline; points are white dots with a red ring.
- Names of points are labels on the globe, placed before the map's own labels (at most 3000), and are
  hidden by the label key L. The line key R hides imported lines, areas and dots with the roads.
- Adding a file drops every drawn overlay, so the tiles in view are drawn again with it.
- Lines that cross the antimeridian are cut there, not drawn round the world. Very long lines are thinned to
  points at least half a texel apart. Latitudes beyond 85.05° are drawn at the edge of the map.

### Limitations (7b)

- Altitude is read but not used: lines lie on the terrain, and nothing floats above it.
- Colours, line styles, icons and descriptions in the files are ignored; KML `gx:Track`, NetworkLinks,
  GroundOverlays and Models are skipped; GPX extensions and times are not read.
- On the flat map points are dots without names, as the map shows no labels.
- A point's name is centred on its dot, so it hides the dot while it is shown.
- A name that collides with another is left out, as for map labels.
- The format is recognised from the contents; XML in UTF-16 or other non-UTF-8 encodings is not read.
- There is no way to remove an import short of restarting. Imports are not remembered between runs.
- Dropping and the keys were not driven by hand here; the logic is unit-tested and the display was
  checked in saved images only.

## Milestone 8: map services (done)

### Adding services
- A service is an XYZ address template (with `{z}`, `{x}` and `{y}` or `{-y}`; `{s}` becomes `a`; `--service-tms` counts rows from the south), a WMTS service or a WMS 1.1.1 or 1.3.0 service.
- `--add-service ADDRESS` adds one to the library and exits; the box opened with A in the list does the same (the address, then a layer name if wanted). A WMS or WMTS address is asked for its capabilities document once, and the kind is detected from it. A service with several layers is added with its first usable layer, or the one named by `--service-layer`; the other layer names are printed.
- WMTS is used in RESTful or key-value form, in web-mercator matrix sets (GoogleMapsCompatible and the like: square tiles, 2^z tiles across, top left corner at the corner of the Mercator square). If a layer is offered only in other sets, the message names them and says only web-mercator sets are used.
- WMS pictures are requested with GetMap in EPSG:3857 (or an alias such as EPSG:900913), with `CRS` in 1.3.0 and `SRS` in 1.1.1. The BBOX is east, north in both versions for this system.
- Only http and https addresses are used, without user names or passwords. No cookies or credentials are sent. A document that is HTML, an error report, malformed or not WMTS or WMS is refused with a plain message.
- The library is `orbis/services.json` in the config folder (`--services-file`), written atomically; a file that cannot be read is left alone. Each service has a name, address, the attribution the service itself gives, and a credit the user may declare (`--service-name`, `--service-credit`).

### Showing services
- `--service NAME` replaces the imagery of the Earth view with a service, `--service-overlay NAME` lays one over it at `--service-opacity` (default 0.6). NAME is a name from the library or the directory, matched ignoring case, or any part of it if only one fits. In the window, G lists services; the base and the overlay are chosen there and the opacity changed with + and -.
- Terrain, roads, labels and buildings are unchanged. Where a service has no tiles (outside its bounds or zoom range) the usual imagery shows. Beyond a service's highest zoom its finest tiles are enlarged. With nothing chosen, imagery and requests are as before.
- Credits follow what is in view: the credit of a service in view appears as "Map service: ...", the user's own wording first, else the service's attribution, else its host name. The usual credits stay listed.
- Tiles are cached under a folder of their own for each service. A service gets two requests at once, with the pacing and pausing used for other servers, and is never asked through LAN cache servers.
- A service that answers with errors, web pages instead of pictures, or pictures of the wrong size is reported in the list ("N failed: ..."); tiles already shown stay, and the bad answer is not kept or asked for again. If an overlay fails, the tile shows without it.

### Directory
- `data/services.json` lists public services whose publishers' own documents allow this use (see PROVENANCE.md for each, and for those left out). `--list-services` prints it. Choosing a directory entry in the list adds it to the library.

### Limitations
- Services show on Earth only, and a change of service redraws the globe's tiles from the cache or the network, so the view flashes empty for a moment.
- XYZ `{s}` always takes the first subdomain. WMS layers have no zoom limit of their own: 18 is used unless the layer states a smallest scale. Time and other WMTS dimensions are fixed at the default found when the service was added.
- Only EPSG:3857 is used; services offering only other systems are refused. Only PNG, JPEG and WebP pictures are read.
- The credit of the usual imagery stays on screen while a service replaces it.
- There is no clipboard paste in the address box. The panel, the keys and adding in the window were not driven by hand here; the logic is unit-tested and the display was checked in saved images.

## Milestone 9a: computed eclipses (done)

### Finding eclipses
- Solar and lunar eclipses are computed in the program from published formulae, with no network and no data file. The Sun comes from the truncated VSOP87 series for the Earth and the Moon from the truncated ELP-2000/82 series, both as printed in Meeus, "Astronomical Algorithms" (about 10 arcseconds for the Moon, 1 for the Sun), with nutation, the Sun's aberration and ΔT from the polynomial of Espenak and Meeus.
- The scan visits every New and Full Moon in a range and keeps those where the Moon's shadow (solar) or the Earth's shadow (lunar) touches the other body. Solar eclipses are total, annular, hybrid or partial; lunar ones total, partial or penumbral. Each has its greatest moment (UT) and magnitude; solar ones also the gamma, the greatest eclipse point and the length of totality or annularity there; lunar ones the contact durations and the point below the Moon.
- Checked against the NASA/GSFC catalogs of Espenak: times of greatest eclipse within a minute for solar eclipses (3 s for 2024-04-08) and two minutes for lunar ones; magnitudes within 0.003 (solar) and 0.02 (lunar); the eclipses of 2023 to 2026 found exactly, with their kinds, and the right number for 2018. The central line of 2024-04-08 lies within 3 km of the NASA table's points.

### Showing them
- E lists the eclipses from one year before the clock's date to two years after, with the nearest chosen. Enter sets the clock to the greatest moment, pauses it, flies to the greatest eclipse point (for a lunar eclipse, the point below the Moon) and shows the details in a notice. `--eclipse DATE` does this for the eclipse nearest DATE and `--eclipses-panel` shows the list in a saved image.
- The central line of a solar eclipse is drawn over the terrain like an imported line, from where the axis of the shadow first meets the Earth to where it leaves it.
- While the clock is within an eclipse the shadow of the Moon darkens the Earth: the penumbra gradually, the umbra (or the antumbra of an annular eclipse) strongly. It follows the clock, so the shadow can be watched crossing the globe at a high clock speed. During a lunar eclipse the Moon in the sky is dimmed, and turns dark copper in the umbra.
- The list is shared by events of other kinds: `EventList::set_source` replaces one source's events, and choosing one flies to its place (and sets the clock if the event says so).

### Limitations
- The list and the shadow are for the Earth only. Non-central total and annular eclipses are given the kind of the umbra but have no central line, and the penumbra's limits are not drawn.
- The shadow's darkness follows a simple rule for how much of the Sun is covered, and ignores clouds, the atmosphere's scattering and the terrain. The shadow is a smooth function of the position and does not show shadows of the mountains.
- ΔT is the published prediction, some 4 seconds above the measured value for 2024; this moves a shadow by about 2 km. Beyond a few centuries from now ΔT cannot be predicted and the times grow uncertain.
- The Moon in the sky is still the low-precision geocentric one of Milestone 5b, up to 1° off for a person on the ground, so a solar eclipse shows it beside the Sun, not over it. The key and the panel were not driven by hand here; the logic is unit-tested and the display was checked in saved images.

## Later milestones

Order and scope are open, and depend on which features are wanted.

| # | Feature | Notes |
|---|---|---|
| M9b | Events | Earthquakes, eruptions, tsunamis and fireballs, added to the list of M9a |
| M10 | SpaceMouse | Through HID or `spacenavd` |
| M11 | Help dialog and clickable interface | A help icon toggles a floating window in a corner that lists the keyboard shortcuts, scrollable when they don't fit. Brings the mouse-driven interface layer that M12 and M16 also use |
| M12 | View options and saved settings | A dialog to show or hide streets, cities, trails, trains and 3D buildings, to switch map projection, and to choose what the globe shows. Every setting is saved in the application's configuration directory; during development, a committed configuration directory in the working directory takes priority |
| M13 | Information on click | Clicking a place on the map, such as a city name, peak or sea, opens a box with relevant data from Wikipedia and Wikidata |
| M14 | Smooth flight on double-click | Double-clicking land or a label flies there with an accelerating, smooth automatic flight |
| M15 | Cast shadows | Terrain and buildings cast shadows calculated from their height and the Sun's position |
| M16 | Time slider | A slider for travelling in time, moving the Sun, Moon and planets continuously, and with them shadows, day and night, and solar and lunar eclipses. Depends on M11, and on M15 for shadows. Dates beyond the range of the planet formulae (about 1800 to 2050) are limited or marked as less accurate |

Out of scope: Apple Vision Pro streaming, Quick Look and Share extensions, Sparkle updates.
