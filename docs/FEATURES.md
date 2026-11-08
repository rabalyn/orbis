# Orbis features

Orbis is a Linux desktop viewer for the Earth and other bodies, as a 3D globe and a flat map, fed from open
data. This document describes what the program does now. `orbis --help` lists every command line option,
[CACHING.md](CACHING.md) covers data traffic and the imagery sources, and [PROVENANCE.md](PROVENANCE.md)
records where data and methods come from.

Contents: [Controls](#controls) · [Globe](#globe) · [Map lines and names](#map-lines-and-names) ·
[3D buildings](#3d-buildings) · [Flat map](#flat-map) · [Clock](#clock) · [Sun and light](#sun-and-light) ·
[Cast shadows](#cast-shadows) · [Time window](#time-window) · [Sky](#sky) · [Other bodies](#other-bodies) ·
[Search](#search) · [Saved places](#saved-places) · [File import](#file-import) · [Map services](#map-services) ·
[Eclipses](#eclipses) · [Event feeds](#event-feeds) · [Information on click](#information-on-click) ·
[Smooth flight](#smooth-flight) · [Interface](#interface) · [View options and settings](#view-options-and-settings) ·
[SpaceMouse](#spacemouse) · [Frame rate](#frame-rate) · [Saved images](#saved-images) ·
[How far this is tested](#how-far-this-is-tested)

## Controls

The view is defined by a view point on the ground, a distance, a heading and a tilt.

| Input | Action |
|---|---|
| Drag | Move the globe, keeping the grabbed point under the pointer; it glides after release |
| Right-drag or Alt-drag | Turn around the view point (horizontal), tilt (vertical) |
| Shift-drag | Climb or sink (vertical), look around from the same spot (horizontal) |
| Scroll | Zoom around the point under the pointer |
| Alt-scroll / Shift-scroll | Tilt / look around |
| Click | Open the information box for the label or ground under the pointer (Wikipedia and Wikidata); with the box open, close it instead |
| Double-click, Shift-double-click | Fly smoothly to the label or ground point under the pointer: one level closer (a place gets a distance that suits it), or with Shift one level farther |
| Arrow keys | Fly forward, back and sideways; speed builds while held |
| Alt-arrows | Tilt and turn around the view point |
| Shift-arrows | Climb, sink and look around |
| Ctrl-Up, Ctrl-Down | One level closer or farther |
| Ctrl-0 | Whole Earth, or the whole map on the map |
| Ctrl-Alt-N | Straighten up, one step per press: north up, straight down, whole body, equator centred |
| SpaceMouse | Push or pull: climb or sink; slide: move across the ground; twist: turn; tilt: tilt, and on through the horizon; button 1: whole body (Ctrl-0); button 2: straighten up (Ctrl-Alt-N). On the map: slide pans, push or pull zooms |
| Escape | Stop a flight or a gliding movement; or, while the time window, the information box, the view options or the help window is open, close it |
| R | Show or hide all map lines: streets, trails, trains and boundaries (if any is shown they all go, otherwise they all come back) |
| L | Show or hide all names: place names and other names (the same way) |
| B | Show or hide 3D buildings (globe only) |
| M | Switch between the globe and the flat map, keeping place and scale |
| P, Shift-P | On the map: next or previous projection |
| [ and ] | On the map: move the central meridian west or east by 15°, or 1° with Shift |
| Space | Pause or resume the clock |
| - and = | Slow down or speed up the clock (1, 10, 60, 600 times, then hourly up to monthly per second) |
| , and . | Step the clock back or forward by one hour, or one day with Shift |
| T | Set the clock back to the real time, running at normal speed |
| C | Show or hide the time window |
| S | Switch between sunlight and flat, always-lit imagery |
| O | Show or hide stars, planets and the Moon (globe only) |
| F | Switch the frame rate limit between 120 and 90 fps |
| Ctrl-F | Open the search box |
| Ctrl-S | Name and save the current view as a place |
| K | Show or hide the list of saved places |
| I | Show or hide the list of imported items |
| E | Show or hide the list of eclipses and events around the clock's date. In it: Enter sets the clock to the greatest moment, pauses it and flies to the greatest point, and 1 to 5 show or hide quakes, eruptions, tsunamis, fireballs and eclipses |
| G | Show or hide the list of map services. In it: Enter uses the chosen service as the base layer (again: back to the usual imagery), O as the overlay, + and - change the overlay's opacity, A opens the box to add a service, Delete removes one |
| V | Show or hide the view options window |
| H, ?, F1 | Show or hide the help window |
| Ctrl-Q | Quit |

An arrow key keeps its meaning while held, even if modifiers change. Moving by hand ends a flight.

Tilt orbits the view point down to 85° from straight down. Tilting further raises the head about the camera,
which stays where it is, through the horizon up to straight up. Tile selection and labels follow what the
screen shows, so the ground may fill only part of it.

Manual flight holds altitude and only rises when the ground comes within 2 m. "The ground" is the higher of the
finest terrain loaded at that spot and the surface as drawn there, so the view is never under what is drawn,
even while a coarse tile is still shown for want of its finer neighbours. Once the finer tiles are drawn the two
agree and the view may come down to 2 m. Loading never lowers the camera, it only holds it up.

### Limitations
- While tiles load in steep country, the camera can float above the true ground, by up to the difference between
  a coarse and a fine mesh: about 100 m at one zoom level in the Himalaya, more at coarser levels. It stays there
  until the finer tiles are drawn.
- When the view turns or moves onto a coarse tile of an area not yet loaded, it is lifted at once rather than eased,
  because it must not be under the drawn surface in any frame: up to 26 m in one frame in a 120-step walk, and
  400 m in a test that forces tiles of zoom 7 on screen.

## Globe

- A WGS84 globe from whole-Earth view down to street scale, with satellite imagery on real terrain.
  `--exaggeration N` scales the terrain's height.
- Detail loads progressively. A coarse tile stays visible until all of its finer replacements are ready, so the
  view never shows holes while loading.
- Where the terrain service has no tile at a zoom level, the nearest coarser terrain tile is used.
- Imagery is Sentinel-2 at 10 m per pixel worldwide, with sharper aerial photos near the ground where national
  services offer them (see [CACHING.md](CACHING.md)).
- A simple atmosphere glow, and plain colour where no imagery exists (the poles).
- Data credits are always visible, along with the view point's coordinates and the camera altitude.
- The display says whether tiles are still loading or waiting to retry, or all have loaded. It also shows how much
  was downloaded, how much came from cache, and which servers are timed out.
- A chosen start distance counts from the ground, not from sea level.
- Tiles are fetched only when a view needs them, on background threads, nearest and coarsest first. They persist in
  a disk cache and work offline once cached.
- A server that reports trouble is paused rather than flooded, and failed tiles are retried with growing delays.

## Map lines and names

### Lines
- Roads, railways, ferries, and country and state boundaries, draped over the terrain.
- Detail follows the zoom level: motorways first, minor roads later, footpaths only when close. Trails appear from tile
  zoom 15, service roads at 15, minor roads at 12, rail at 10 and ferries at 8.
- Tunnels and disputed boundaries are dashed.

### Names
- Countries, states, cities, towns, villages, districts, seas, lakes, peaks with their heights, and road names or
  numbers. Road labels follow their road's direction and stay upright.
- Names use the language from `--lang`, the saved language or the locale, falling back to the local name. Any script
  the system's fonts cover is shown.
- Labels never overlap and stay clear of the on-screen panels. They do not hang over the edge of the globe, and
  mountains hide what is behind them. The more important label wins.

### Layers
- Six independent switches in the view options: streets (all road classes and ferries), trails (classes path and
  track), trains (rail and transit), boundaries, cities and towns (country, state, city, town, village, suburb and
  smaller place names), and other names (water, peaks and street names). Street names also need streets on.
- Flags: `--no-streets`, `--no-trails`, `--no-trains`, `--no-boundaries`, `--no-cities`, `--no-names`, and for whole
  groups `--no-roads` and `--no-labels`.
- Switching layers asks nothing of a server: overlays are redrawn from decoded tiles held in memory or the disk cache.

### When the map server is unreachable
Map data is independent of imagery. If its server is unreachable, the globe keeps working without lines and labels,
and they appear once the server answers again.

### Limitations
- Lines are drawn into per-tile images, so their width steps slightly between zoom levels, and they soften when very close.
- Road labels are straight, not curved along the road.
- Label quality follows OpenStreetMap, including occasional wrong or vandalised entries.
- At coarser views a layer switch can change nothing visible, since the layer isn't in those tiles yet. Boundaries are drawn for
  country and state levels only. Park and other area names are not decoded. Trails and streets are told apart by the
  data's class only: a footway tagged as a path counts as a trail.

## 3D buildings

- Buildings rise from OpenStreetMap footprints with their mapped heights, colours, courtyards and raised parts such as
  stepped towers.
- They stand on the terrain: roofs stay level, and walls reach down to the low side of a slope.
- In an 800-pixel-high view looking straight down, they appear somewhere between 25 and 15 km from the ground. Larger
  windows load detail, and so buildings, from further away.
- Only outward faces are drawn, so a camera inside a building sees out. Edges are anti-aliased where the GPU supports it.
- Each face is lit by the direction it points, and buildings cast and receive shadows.
- B or `--no-buildings` hides them.

### Limitations
- Flat roofs and plain colours: no textures, windows or real roof shapes.
- Height and colour are only as good as the map data, and buildings without a height get a default.
- Buildings end along a line in the far distance, because height data exists only at one zoom level.
- The camera does not collide with buildings, and labels show through them.
- A building crossing a tile edge can show a small step if the ground differs between its two parts.

## Flat map

- M switches to a flat map and back, `--atlas PROJECTION` starts with it and `--globe` starts on the globe whatever is saved. The map uses the same imagery, the same
  coarse-before-fine tile replacement and the same road and boundary lines. It works on every body.
- Projections: Equal Earth, Mollweide, Hammer-Aitoff, Mercator and Plate Carrée (`equal-earth`, `mollweide`,
  `hammer-aitoff`, `mercator`, `plate-carree`). P cycles through them.
- The central meridian is the longitude at the middle of the map. `--central-meridian DEG` sets it, and `[` and `]` move
  it. Tiles that cross the seam on the far side are cut there, so nothing wraps wrongly.
- Every tile is drawn as a fine grid whose corners are projected exactly, so the picture is correct over the whole map.
  Nothing is drawn outside the map's outline.
- Mercator ends at 85.05° north and south, which is also where the imagery ends, so its map is square. The other
  projections show the polar caps in a plain colour (ice in the south, ocean in the north).
- Drag moves the map, scroll zooms around the pointer, double-click flies smoothly to centre the point and zoom in (Shift
  zooms out). Dragging stops at the edge of the map and at the poles. Zooming out stops when the whole map fits.
- Tiles are chosen for the scale on screen, about one texel per pixel, and only those in the window, at most 700.
- Credits and the status panel stay. The coordinates are those of the point at the middle of the window, and the status gives
  the projection, the central meridian and the metres per pixel at the equator.
- Switching keeps the place and the scale; the whole-Earth globe gives the whole map. `--lat`, `--lon` and `--distance` apply as
  well: a `--distance` of the whole Earth or more shows the whole map.

### Limitations
- Names, 3D buildings, terrain relief, the sky and cast shadows are not shown on the map. Imported points are dots without names.
- The world is not repeated sideways: the map has one edge on each side, and dragging stops there.
- Arrow keys do nothing on the map, and there is no glide after a drag.
- The metres per pixel in the status are true only at the equator; other latitudes differ by projection.
- Zooming past the finest imagery magnifies its texels, at most twofold.

## Clock

- A simulated clock starts at the real time and is shown in UTC in the top-right display, with "paused" or the speed when
  it is not running at normal speed.
- It can be paused, sped up or slowed down, stepped by hours or days, set back to now, or moved with the [time window](#time-window).
- `--time ISO8601` sets the start (`2026-06-21T12:00:00Z`, with or without an offset or a time; without one it is UTC), and
  `--time-speed N` sets clock seconds per real second. Saved images show one moment, with the clock paused.
- The picture is redrawn only when the Sun has moved 0.05° (about 12 s at normal speed), so an idle window uses almost no CPU.
  Speeds of 600 times and above redraw continuously.

The clock ignores leap seconds and the difference between UT and terrestrial time (under 70 s) for the Sun's place; eclipses
take that difference into account.

## Sun and light

- The Sun's place comes from the Astronomical Almanac's low-precision formulae, good to about 0.01° from 1950 to 2050.
- The globe, its terrain, 3D buildings and the flat map are lit by the Sun. Day turns to night over a soft terminator with
  twilight, reaching full night when the Sun is about 14° below the horizon. Low sunlight is warm.
- Terrain is shaded by its slope, so slopes facing away from a low Sun fall into shade.
- The night side stays dimly readable. There are no city lights.
- The atmosphere glow is bright on the day side, warm at dusk and faint at night.
- With S (or `--no-sun`) everything is shown flat and fully lit, so imagery can be read at night.

### Limitations
- Imagery already contains the light of the day it was taken, so lit slopes are lit twice.
- No clouds.

## Cast shadows

- Terrain and 3D buildings cast shadows. A shadow is found from the height of what casts it and the direction of the Sun, so it
  points away from the Sun, is long when the Sun is low (a point of height `h` shades the ground `h / tan(elevation)` away) and
  moves as the clock runs.
- Only direct sunlight is lost in a shadow, and the sky light stays. Slope shading, twilight and an eclipse's shadow combine with it
  without darkening anything twice.
- This is for the Earth in practice: the other bodies are smooth and have no buildings, so nothing there casts a shadow. The dark
  crater walls seen on the Moon are in the imagery (a view of Tycho is pixel-identical with shadows on and off).
- Mountains outside the view cast into it: terrain tiles already loaded towards the Sun are used, up to 300 km along the Sun's rays
  (down to a Sun about 2° high for the highest peaks), and buildings that are loaded but off screen cast as well. Nothing is
  requested for shadows alone.
- Shadows reach up to 600 km from the camera, in three bands of detail that blend into each other. At the far end they fade out over
  the last 15%, and when the nearest ground is over 300 km away they fade out as a whole. From 600 km there is no shadow computation at all.
- No shadows are computed when the Sun is below the horizon everywhere in view, in the flat, always-lit mode (S, `--no-sun`), on
  the flat map, when the camera looks only at the sky, or when they are switched off.
- The setting is "Cast shadows" in the view options window, saved with the other options. `--no-shadows` starts without them. There is no key.
- Shadows are only redrawn when a frame is drawn anyway. Their edges are soft by about a texel and do not shimmer while the camera moves,
  because each band's grid is fixed in the world.

### Limitations
- Imagery already contains the shadows of the day it was taken, so a shadow can fall on ground already dark, or leave a sunlit patch dark in the picture.
- With a low Sun, a shadow on Earth only about halves the brightness, because the sky light stays and low sunlight is weak.
- Detail is limited by the tiles in memory. A mountain behind the camera casts only as finely as the tiles that happen to be loaded
  there; if none are, it casts nothing. Buildings cast only when their tiles have been loaded for a view of them. A shadow can therefore
  appear or sharpen as tiles arrive.
- Shadows are as fine as a texel of their band: about 1.2 m for a view of Manhattan from 1.5 km, and about a thousandth of the distance
  farther away. A camera a few metres above the ground gets shadows only out to a few hundred metres.
- Shadows from things beyond 300 km towards the Sun are lost, as are those from the sphere under the tiles (before any tile has loaded).
  Trees and other things not in the terrain or the building meshes cast nothing.
- The shadow maps take 50 MB of graphics memory, made when shadows are first drawn and freed when they are turned off.

## Time window

- Key C, or the clock icon next to the options and help icons at the bottom right, shows or hides the window. It sits above the
  credits, beside the view options window.
- It has three sliders, since one bar cannot be centuries wide and minute exact: the year (1800 to 2050), the day of that year
  (1 January to 31 December, shown as a date) and the time of day (00:00 to 23:59 UTC, in whole minutes). Each moves only its own part of
  the date. Changing the year keeps month, day and time, and 29 February becomes 28 February in a year without one. A heading shows the full
  date and time in UTC, with the seconds when the clock is paused or not at normal speed.
- Dragging a thumb sets the clock directly, so the Sun, Moon, planets, stars, cast shadows, day and night and the eclipse shadow follow during
  the drag. Grabbing the thumb does not move it. A press on the bar beside it moves the thumb there and goes on as a drag. The mouse wheel over
  a slider steps it by one year, one day or ten minutes.
- While a thumb is held the clock does not advance. On release it goes on as it was, running or paused, at the same rate.
- The window also has Now (as T), Pause or Run (as Space), and on the Earth Previous eclipse and Next eclipse, which set the clock to the
  greatest moment of the closest eclipse before or after the clock and pause it. The window does not fly the view; use E for that.
- A line under the buttons names the eclipse while one is going on, such as "Total solar eclipse, greatest 18:17", and says so when there is none.
  On other bodies there are no eclipse buttons or line.
- The sliders stop at 1800 and 2050. The clock can still reach other dates with `--time` or the clock keys. Then the year thumb sits at its
  nearer end, and the window warns "Outside 1800-2050: planet positions are approximate". The clock line at the top right says
  "approximate: outside 1800-2050" for such dates, also with the window closed and in saved images. Before 1950 the window carries a milder note
  that the Sun is less exact, because the Sun's formula is stated for 1950 to 2050 only.

### Limitations
- Only the sky and the light move. Imagery, terrain, buildings, roads and names stay as they are: a map tile shows the Earth as it was on the day it
  was taken, whatever date the clock has, and the buildings and roads are today's.
- Outside 1800 to 2050 the positions get worse in a way that is not measured here, and the Earth's turn (the difference between UT and the uniform time
  the formulae use) comes from published polynomials that are a prediction for the future. The Sun's position for 1850 was only checked against the
  equation of time at a single date.
- The time of day has no seconds and no 24:00; the clock keys can still set any moment. The wheel stops at the ends of the year, the day and the day's
  minutes, and does not carry over.
- Eclipse times use the Earth's rotation as modelled, so they are less certain the farther they are from the present. There is no figure for this.
- The window's lines are fixed text, and on very small screens it can be cut off at the top.

## Sky

Tilting past the horizon looks up into the sky, from a view point on or above the ground.

### Sky colour and Sun
- Seen from inside the atmosphere the sky is a blue gradient, lighter at the horizon, that darkens with the Sun's altitude to night. A low Sun warms the
  sky around it and along the horizon. The effect fades out with height over about 15 km, giving way to the glow seen from space.
- The Sun is a small bright disc with a halo, reddened when low, from the ground and from space.
- With S (flat light) the sky stays at its daytime blue and shows no Sun.

### Stars, planets and the Moon
- About 5,000 stars down to magnitude 6 from the HYG database. They are points whose size and brightness follow their magnitude, tinted by colour.
- Stars are turned by Greenwich mean sidereal time and precessed from J2000 to the date, so they turn with the clock.
- Mercury, Venus, Mars, Jupiter and Saturn come from JPL's approximate Keplerian elements (arcminutes, 1800-2050), and are points with their magnitudes.
- On the Earth the Moon's place comes from the same long series as the eclipses and is seen from the camera, with parallax, so in a solar eclipse it covers
  the Sun for those in the path. It is a disc lit by the Sun, so it shows its phase, drawn at least 10 pixels wide. The covered Sun dims the day sky and the stars
  show in totality; the corona is a plain glow. During a lunar eclipse the Moon is dimmed, and turns dark copper in the umbra.
- Moon and planets are refreshed whenever the clock has moved by a quarter of a second or the camera by 100 m, which looks continuous at any clock speed and costs nothing while idle.
- Names are drawn beside planets and the Moon while labels are on (L).
- Faint objects disappear as the sky brightens: at night within the air all of them show, in daylight only Venus and the Moon, and from space everything shows, even over the day side.
- O (or `--no-stars`) hides stars, planets, the Moon and their names.
- The stars' credit is shown while they are displayed.

### Limitations
- Planets and stars are geocentric and at infinity, so a camera far from the Earth does not see parallax. The Moon is geocentric on other bodies.
- No Saturn rings in its brightness, no other planets, no constellation lines, no star names, no deep-sky objects, no twinkling and no light from the Moon or planets on the ground.
- The sky ignores terrain: a mountain does not hide a planet behind it, and names of planets behind a mountain are drawn anyway. There are no clouds or haze.

## Other bodies

`--body earth` (the default), `moon`, `mars`, `mercury`, `ceres` or `vesta` selects the body. Everything follows it: geodesy and altitude, the zoom limits, the whole-body view,
the horizon, the tile grid, lighting and the sky. The choice in the view options takes effect at the next start.

| Body | Shape | Imagery (from NASA's Trek services) | Finest detail at the equator |
|---|---|---|---|
| Moon | Sphere, 1737.4 km | LRO Wide Angle Camera mosaic | About 83 m per pixel (zoom 8) |
| Mars | Spheroid, 3396.19 and 3376.20 km | Viking MDIM 2.1 colour mosaic | About 330 m (zoom 7) |
| Mercury | Sphere, 2439.4 km | MESSENGER MDIS basemap | About 330 m (zoom 7) |
| Ceres | Spheroid, 487.3 and 446.0 km | Dawn HAMO colour-shaded mosaic | About 1.3 km (zoom 5) |
| Vesta | Spheroid, 284.5 and 229.0 km (the published 289 by 280 by 229 km, with the equatorial axes averaged) | Dawn HAMO mosaic | About 700 m (zoom 6) |

- All have longitude counted east, and `--lat` and the display in geodetic latitude. The Moon's longitude 0 faces the Earth. Imagery is laid out in planetocentric latitude, and its rows
  are converted when tiles are placed, so features sit where the maps put them. Checked by eye: Olympus Mons on Mars at 18.65 N, 226.2 E, Kuiper crater on Mercury at 11.3 S, 31.5 W, and a Ceres crater the size of Occator at 19.8 N, 239.3 E.
- There is no air: no blue sky or glow, a sharp terminator and only a trace of light on the night side. Stars stay visible by day and night.
- Rotation uses the IAU Working Group's pole and prime meridian for each body, with periodic terms for the Moon, Mars and Mercury. So phase, libration and the day and night side follow the time.
- The Sun's direction on Mars and Mercury comes from JPL's planet elements, and on Ceres and Vesta from two-body orbits of JPL Small-Body Database elements for 2026-11-21. Against JPL Horizons, the sub-solar point of Mercury and Mars is within 0.01° at three dates in 2026 to 2031, Ceres within 0.6° of longitude and Vesta within 1.4°.
- The sky shows the Sun, stars and the other planets, turned with the body's frame. On the Moon the Earth is a lit disc with its phase and name; elsewhere it is a point with a magnitude.
- There are no roads, names, buildings, national aerial imagery or terrain: R, L and B do nothing there, and nothing casts a shadow. The flat map and the time window work. Search takes coordinates only, and map services, imports, eclipses, events and article lookups are for the Earth.
- Each body's tiles are cached apart and requested gently from `trek.nasa.gov`. The credit is shown while the body is in view.

### Limitations
- Bodies are smooth, so nothing rises above the sphere or spheroid, and relief shows only as far as it is in the imagery. Moon Trek's LOLA elevation layer has 8-bit tiles whose height scaling the service does not document, so it isn't used.
- Very close views are magnified, because the zooms above are the finest levels of the services.
- Imagery is lit by the program's Sun on top of its own illumination, so relief shows less at high Sun. Ceres' layer is coloured by elevation.
- On the Moon the Sun is taken from the Earth's centre, up to 0.15° off. Libration is included, but the mosaic's frame is the mean Earth direction, not the principal axes. The Earth is a plain disc without clouds or oceans.
- Mars has no thin atmosphere here: no blue-brown sky, dust haze or soft dusk. It looks as the Moon does.
- Vesta's prime meridian is the Dawn "Claudia double prime" system, and the Trek layer names the IAU system without more detail. Only the poles could be checked by eye (the Rheasilvia basin sits at the south pole), not Vesta's or Ceres's longitudes against a named feature.
- The orbits of Ceres and Vesta are two-body, and drift by up to 0.25° from the Sun within three years of the epoch and more beyond. For years far from 2026, refresh the elements in `src/planetary.rs`.
- The flat map places rows in geodetic latitude, so Vesta in particular appears distorted there.
- The planets are points with approximate magnitudes (no phase for the Earth). Phobos, Deimos and other moons are not drawn, and the Moon is not drawn from Mars. The Sun is a point source with no angular size.
- The body cannot change in a running program; start again with `--body`.

## Search

Ctrl-F opens a search box at the top left.

- While it is open it takes every key: typed text (including non-ASCII letters and input-method commits), Backspace, Up and Down to choose a result, Enter to search or to fly to the chosen result, Escape to close. The mouse still works.
- Coordinates are recognised locally, with no request, on any body: "46.5, 11.8", "46.5N 11.8E", "N 46 30 E 11 48", "46°30'N 11°48'E", with seconds too. Latitude comes first unless letters say otherwise. Enter flies there, to 10 km.
- Anything else is a place name, sent to OpenStreetMap's Nominatim on Enter only, never while typing. Requests are at least 1.1 s apart. The box says "Searching...", "No place found." or that the server cannot be reached. Answers are cached on disk by query and language; with `--offline` only cached queries work.
- Up to eight results are listed, in the language of `--lang`. Enter flies to the chosen one, ending at a distance that fits its bounding box, north up, looking straight down. On the flat map the view moves there directly.
- While results are shown the credit `Search: Nominatim, © OpenStreetMap contributors` is in the panel and in the credits.
- On bodies other than Earth names are not searched: the box says so and suggests coordinates.
- `--search TEXT` flies to the first result (or the coordinates) at start, and `--geocoder-url URL` switches the service.

### Limitations
- The built-in font draws only ASCII and Latin-1; other letters in the box or in results show as `?`, though they are searched for correctly.
- No editing within the text (no cursor movement, selection or paste); Backspace deletes the last character.
- Results are cached without an expiry, other than the cache's size limit.
- The user agent names the program but gives no contact address.

## Saved places

- Ctrl-S opens the search box in a naming mode ("Save place as:"). Enter keeps the current view under that name; an empty name becomes the position, for example `46.5000°N 11.8000°E` (prefixed with the body's name away from Earth). Escape cancels.
- A place stores the body, position, distance, heading and tilt. On the map the heading and tilt are stored as zero and the distance is the one that matches the map's scale.
- K opens a list at the top left. Up and Down choose, Enter flies there, Delete or Backspace removes, K or Escape closes. While the list is open these keys do not move the view; all others work as usual.
- A place on another body than the one being viewed is listed with that body's name; choosing it shows which `--body` to start with.
- The places are one JSON file, `orbis/places.json` under the user's config directory (`--places-file PATH` changes it), written through a temporary file and a rename.
- A file that cannot be read (not JSON, an entry damaged, written by a newer version, too large) is renamed to `places.json.bak` (or `.bak.1`, ... if that exists), the list starts empty, and a message on screen says so. If it cannot be moved aside, nothing is written, so it is not overwritten.
- `--save-place NAME` saves the view at start.

## File import

- `--import FILE` (repeatable) reads a file at start-up, and a file dropped on the window is read and the view flies to it. The format is recognised from the contents, not the name:
  GeoJSON (RFC 7946, any geometry, Features and FeatureCollections), GPX 1.1 (waypoints, each track segment, routes), KML 2.2 (Placemarks with Point, LineString, LinearRing, Polygon with holes and MultiGeometry, inside any Folders and Documents), KMZ (a zip; `doc.kml`, else the first `.kml`) and IGC (the B records as one line with the GNSS altitude).
- Each file gives points, lines and polygons with a name (the placemark, feature or waypoint name; for IGC the pilot and glider and the date) and the altitude where the file has one.
- I lists the imported items at the top left; Enter flies to the chosen item's bounding box (a point to 4 km).
- Lines and areas are drawn with the roads and on top of them, so they follow the terrain on the globe and on the map. Lines are red with a dark edge; areas have a translucent yellow fill (holes stay open) and a yellow outline; points are white dots with a red ring.
- Names of points are labels on the globe, placed before the map's own labels (at most 3000), and are hidden by L. R hides imported lines, areas and dots with the roads.
- Lines that cross the antimeridian are cut there, not drawn round the world. Very long lines are thinned to points at least half a texel apart. Latitudes beyond 85.05° are drawn at the edge of the map.
- Imports are for the Earth. On another body a message says so and nothing is read.

Files are not trusted:
- A file over 64 MiB is refused, and so is a KMZ whose KML would be over 64 MiB once unpacked (unpacking stops at the limit, whatever the zip claims).
- A KMZ is only read in memory. Its entry names are never used as paths, and names that climb out of a folder are skipped.
- At most 50 000 items and 3 million positions are kept per file, and a message says if the rest was left out.
- XML entities other than the predefined and numeric ones are not expanded, nesting is limited, and positions that are not on the globe are skipped.
- Damaged files give a message on screen, never a crash.

### Limitations
- Altitude is read but not used: lines lie on the terrain, and nothing floats above it.
- Colours, line styles, icons and descriptions in the files are ignored. KML `gx:Track`, NetworkLinks, GroundOverlays and Models are skipped, and GPX extensions and times are not read.
- A point's name is centred on its dot, so it hides the dot while it is shown. A name that collides with another is left out, as for map labels.
- XML in UTF-16 or other non-UTF-8 encodings is not read.
- An import can't be removed short of restarting, and imports are not remembered between runs.

## Map services

G lists map services, for use as the Earth's imagery or laid over it.

### Adding
- A service is an XYZ address template (with `{z}`, `{x}` and `{y}` or `{-y}`; `{s}` becomes `a`; `--service-tms` counts rows from the south), a WMTS service, or a WMS 1.1.1 or 1.3.0 service.
- `--add-service ADDRESS` adds one to the library and exits, and the box opened with A in the list does the same (the address, then a layer name if wanted). A WMS or WMTS address is asked for its capabilities document once, and the kind is detected from it. A service with several layers is added with its first usable layer, or the one named by `--service-layer`; the other layer names are printed. `--service-max-zoom N` asks for no finer tiles than N (the default for XYZ is 19).
- WMTS is used in RESTful or key-value form, in web-mercator matrix sets. If a layer is offered only in other sets, the message names them.
- WMS pictures are requested with GetMap in EPSG:3857 or an alias such as EPSG:900913.
- Only http and https addresses are used, without user names or passwords. No cookies or credentials are sent. A document that is HTML, an error report, malformed or not WMTS or WMS is refused with a plain message.
- The library is `orbis/services.json` in the config folder (`--services-file`), written atomically; a file that cannot be read is left alone. Each service has a name, address, the attribution the service itself gives, and a credit the user may declare (`--service-name`, `--service-credit`).

### Showing
- `--service NAME` replaces the imagery of the Earth view with a service, and `--service-overlay NAME` lays one over it at `--service-opacity` (default 0.6). NAME is a name from the library or the directory, matched ignoring case, or any part of it if only one fits.
- Terrain, roads, labels and buildings are unchanged. Where a service has no tiles the usual imagery shows, and beyond a service's highest zoom its finest tiles are enlarged.
- The credit of a service in view appears as "Map service: ...": the user's own wording first, else the service's attribution, else its host name. The usual credits stay listed.
- Tiles are cached under a folder of their own for each service. A service gets two requests at once, with the usual pacing and pausing, and is never asked through LAN cache servers.
- A service that answers with errors, web pages instead of pictures, or pictures of the wrong size is reported in the list ("N failed: ..."). Tiles already shown stay, and the bad answer is not kept or asked for again. If an overlay fails, the tile shows without it.

### Directory
`data/services.json` lists public services whose publishers' own documents allow this use (see [PROVENANCE.md](PROVENANCE.md) for each, and for those left out). `--list-services` prints it. Choosing a directory entry in the list adds it to the library.

### Limitations
- Services show on Earth only, and a change of service redraws the globe's tiles from the cache or the network, so the view flashes empty for a moment.
- XYZ `{s}` always takes the first subdomain. WMS layers have no zoom limit of their own: 18 is used unless the layer states a smallest scale. Time and other WMTS dimensions are fixed at the default found when the service was added.
- Only EPSG:3857 is used; services offering only other systems are refused. Only PNG, JPEG and WebP pictures are read.
- The credit of the usual imagery stays on screen while a service replaces it.
- There is no clipboard paste in the address box.

## Eclipses

- Solar and lunar eclipses are computed in the program, with no network and no data file. The Sun comes from the truncated VSOP87 series and the Moon from the truncated ELP-2000/82 series, both as printed in Meeus, "Astronomical Algorithms" (about 1 and 10 arcseconds), with nutation, the Sun's aberration and ΔT from the polynomial of Espenak and Meeus.
- Solar eclipses are total, annular, hybrid or partial; lunar ones total, partial or penumbral. Each has its greatest moment (UT) and magnitude. Solar ones also have the gamma, the greatest eclipse point and the length of totality or annularity there. Lunar ones have the contact durations and the point below the Moon.
- Accuracy against the NASA/GSFC catalogs of Espenak: times of greatest eclipse within a minute for solar eclipses (3 s for 2024-04-08) and two minutes for lunar ones; magnitudes within 0.003 (solar) and 0.02 (lunar); the eclipses of 2023 to 2026 found exactly, with their kinds, and the right number for 2018. The central line of 2024-04-08 lies within 3 km of the NASA table's points.
- E lists the eclipses from one year before the clock's date to two years after, with the nearest chosen. Enter sets the clock to the greatest moment, pauses it, flies to the greatest eclipse point (for a lunar eclipse, the point below the Moon) and shows the details in a notice. `--eclipse DATE` does this for the eclipse nearest DATE.
- The central line of a solar eclipse is drawn over the terrain like an imported line, from where the axis of the shadow first meets the Earth to where it leaves it.
- While the clock is within an eclipse the shadow of the Moon darkens the Earth. The light that is left is the fraction of the Sun's disc that stays uncovered, so the penumbra darkens gradually and the umbra (or the antumbra of an annular eclipse) strongly, and the sky light dims with it. It follows the clock, so the shadow can be watched crossing the globe at a high clock speed.

### Limitations
- The list and the shadow are for the Earth only. Non-central total and annular eclipses are given the kind of the umbra but have no central line, and the penumbra's limits are not drawn.
- The shadow's darkness is the uncovered area of the Sun's disc, and ignores clouds, the atmosphere's scattering and the terrain.
- The shadow is faint when seen from far away.
- ΔT is the published prediction, some 4 seconds above the measured value for 2024; this moves a shadow by about 2 km. Beyond a few centuries from now ΔT cannot be predicted and the times grow uncertain.

## Event feeds

- Opening the events panel (E) asks four public feeds for events around the clock's date: earthquakes (USGS), significant eruptions and tsunamis (NOAA NCEI) and fireballs (NASA/JPL CNEOS). Nothing is requested at start-up, unless `--event-feed quakes|eruptions|tsunamis|fireballs` asks for one feed.
- Answers are kept on disk: ten minutes for the live earthquake feed, a day for the others. `--offline` uses whatever is stored, of any age, and a stale answer is also used when a service cannot be reached. A feed that cannot be reached, or whose answer cannot be read, shows one plain line in the panel and the rest keeps working.
- The panel lists events within 90 days of the clock, with the eclipses, by date, kind, magnitude and place. Keys 1 to 5 show or hide earthquakes, eruptions, tsunamis, fireballs and eclipses. Enter sets the clock to the event, pauses it and flies there, and the notice tells more.
- Events within 15 days of the clock's date are marked on the globe, with a size by magnitude (volcanic explosivity, radiated energy for fireballs) and a colour by kind. The markers follow the clock, and a hidden kind has none. `--no-markers` or the view options hide them. The credit of a feed is shown while its markers are.
- Quakes of magnitude 4.5 and above are used for dates in the last 20 days, and magnitude 5 and above for other dates (at most 500).

### Limitations
- Events are fetched for the clock's date when the panel is opened or the flag is given; moving the clock far away and not opening the panel again shows no new events.
- The eruptions database holds significant eruptions only and gives dates with no time, so the clock is set to the start of the day. Tsunami and eruption dates before 1800 are ignored.
- Markers are drawn like imported points, so they are on the Earth only and have no labels or click. Only the panel's choice shows an event's details.

## Information on click

- A single click on the map asks about what is under the pointer: a placed label if there is one, else the ground point. Nothing is asked at start, on hover or while moving. The checkbox "Information on click" in the view options (saved, default on) or `--no-info` switches it off: no box and no requests.
- The box is a panel at the bottom left, above the credits, with a close button and Escape. It opens at once, with "Looking up..." until the answer comes. With the box open, a click on the map only closes it (a double click still flies, and closes it). Where it would overlap the help or the options window, the one opened last stays.
- A label is looked up in Wikipedia in the interface's language, and in English if the first search has no match. The article is accepted only if:
  - its title is the label's name (a bracketed or comma qualifier such as "Paris, Texas" is allowed);
  - it is not a disambiguation page;
  - its own coordinates, on Earth, lie within a radius that fits the label kind (peak 3 km, quarter 4 km, village 6 km, town 12 km, city 30 km, lake 100 km, state 1000 km, sea 2500 km, country 3000 km);
  - no second place of that name is in range and not clearly apart (within three times the nearest one's distance plus a kilometre);
  - for countries and seas, it is of that kind in Wikidata. A country or sea whose article has no coordinates is accepted on name alone only with that check.
- Labels of streets are never looked up. Without a confident match the box says "No article found for NAME." and nothing is guessed.
- Small places (peak, quarter, village, town, city) that the word search does not list are also looked for among the articles around the label (at most 10 km) and matched the same way.
- Open ground shows the coordinates and up to five articles nearby, nearest first, with their distances. The radius follows the altitude (0.4 times it, from 1 to 10 km). A row opens that article in the box.
- The article box shows the title, the one-line description, facts from Wikidata where there are some (type, country or countries, population with the year, elevation, area, official website, time zone), then the article's introduction as plain text, scrollable, and the credit. Text only: no images.
- A button opens the article in the default browser (`xdg-open`, with the address from the API checked to be a Wikipedia address and started without a shell). A failure is shown in the box.
- A click makes at most five requests, one after another and at least 0.3 s apart. All answers are kept on disk for a week. With `--offline` only kept answers are used, and otherwise the box says so. A stale answer is used when the service cannot be reached.
- On other bodies the box shows the coordinates and says that information is available for Earth only; nothing is asked. On the flat map a click gives the ground list.

### Limitations
- The box uses the built-in font (ASCII and Latin-1, with accents of Latin Extended-A folded to plain letters and common punctuation simplified). Other scripts (Cyrillic, Greek, CJK, Arabic, Hebrew, Vietnamese and so on) show as `?`, so articles in those languages are largely unreadable. Pronunciation guides are dropped.
- Wikipedia's word search ranks by words, so a place with a common name may be found only by the search around the label, which reaches 10 km. A village label whose article lies farther, or has no coordinates, gets no article, and neither do labels that differ from the article's title (an exonym, a local spelling).
- Wikipedia's coordinates for countries and seas are rough, hence the large radii.
- The facts are taken as Wikidata states them. Population is the newest statement that is not deprecated, which may be old or an estimate.
- The public Wikidata query service throttles clients. When it asks to wait, articles still show without facts, but countries and seas, which need the class check, show a message instead.
- No marker is drawn at the selected place.
- The user agent has no contact address, which Wikimedia's policy asks for (see [PROVENANCE.md](PROVENANCE.md)).

## Smooth flight

- A double click on the globe flies the view to what is under the pointer. A placed label wins (the most important one where they overlap); otherwise it is the ground point. On the sky or in space nothing happens.
- Plain ground is centred and the view comes one level closer (half the distance); with Shift it goes one level farther (double).
- A label is centred and the view ends at a distance that fits its kind: a country 4000 km, a sea 2000 km, a state 1200 km, a city about 12 to 37 km (bigger cities farther), a town 15 km, a village 6 km, a district 3 km, a peak 5 km, a lake 60 km, a road 20 km (on Earth; other bodies scale it). A click never takes the view farther out than it is: closer than the fit, it only centres. Shift on a label goes one level farther.
- The path is that of van Wijk and Nuij, "Smooth and efficient zooming and panning" (IEEE InfoVis 2003), with their recommended rho of the square root of 2. Over a long hop the view climbs, crosses and sinks again, and the flight starts and ends at rest. It lasts 1.5 to 6 seconds, longer for a longer path. It goes the short way over the antimeridian and reaches the poles.
- Heading and tilt stay the user's on short hops. High over a long hop the tilt eases towards straight down, so the globe does not swing, and the user's tilt returns at the end. The camera stays above the terrain and works on every body.
- Any drag, wheel turn, flying key, SpaceMouse movement or Escape ends the flight at once, with no jump. A double click during a flight starts a new one from the view as it is.
- While flying, only the tiles of the view the flight lands on are requested. The views passed on the way are drawn from what is already there, and names, lines and buildings are fetched after landing. A long flight costs no more downloads than arriving there directly.
- On the flat map a double click does the same pan and zoom with the same curve; there are no labels to pick.
- Search, saved places and Ctrl-0 use a simpler automatic flight of their own.

### Limitations
- Intermediate views of a long flight show what is cached; where nothing is, they show the coarsest imagery.
- A label's distance depends only on its kind and the importance it was drawn with, not on the size of the place.
- On the flat map a flight is a straight pan in map coordinates; near the edge of a projection it is clamped to the map.

## Interface

### Panels and clicks
- Floating panels hold labels, buttons, checkboxes, choices, sliders and scrolling lists. Hovered and pressed buttons are drawn lighter, and a button clicks when pressed and released while the pointer is on it.
- A press, a release or a wheel turn over a panel belongs to the panel and does not move the globe or the map. A drag that began on the globe or map stays with it, even across a panel, and a scroll bar drag stays with the bar.
- Drawing happens only when something visible changed: the pointer moving onto or off a button, a scroll, a toggle, a resize. Moving the pointer over the map causes no redraw.
- A press and release on the map further than 5 pixels apart, or held longer than 0.3 s, is a drag; otherwise a click. A second click within 0.4 s and 6 pixels of the first is a double click. A single click is reported only once that time has passed, so a double click never also gives a single one.
- Credits stay fully readable while their data is shown, in the window and in saved images: no panel is laid out over them, and credits wrap in the width the row of icons leaves. A panel that does not fit above them shrinks and scrolls.

### Help window
- A question-mark icon sits in the bottom right corner, always shown. Clicking it, or pressing H, ? or F1, shows or hides the window of keyboard and mouse controls. Its close button and Escape hide it.
- The window sits above the icon and lists the controls in groups, with keys in one column and what they do in the other (stacked when the window is narrow). It shrinks with the window, and a list that does not fit scrolls with the wheel and shows a scroll bar.

### Limitations
- On a small window a tall panel can cover the readout at the top right.
- Panels have no keyboard focus, and the wheel scrolls three lines per notch with no smoothing. Panel text uses the 8x8 bitmap font, scaled by whole numbers.
- The icons have no tooltips.

## View options and settings

### View options window
The second icon at the bottom right, between the clock and the question mark, and the key V open it; Escape or its close button closes it.

- It has checkboxes for the six map layers, 3D buildings, sunlight, cast shadows, stars and planets, event markers and information on click. It also offers a choice of globe or flat map, a projection (usable on the flat map only), a body, the Earth's imagery (the usual imagery, or a service of the user's library) and the frame rate limit. Clicking acts at once and is saved.
- The window sits at the bottom left and the help window at the bottom right. Where they would overlap, opening one closes the other.
- Keys that change one of these options go through the same path, and the window shows the live state, whichever way it changed.

### Saved settings
- Options are kept in `settings.json`: in the folder given with `--config-dir`, else in `./config/` if the working directory has that folder (a source checkout: the committed `config/settings.json` holds the defaults), else in `orbis/` in the user's config folder. Saved places and services stay in the user's folder and never go in `./config/`.
- Order of precedence: defaults, then saved settings, then the command line. A flag applies to that run only and is not written back. Only what the user changes in the window is written, one field at a time, through a temporary file and a rename.
- A value that is missing, of the wrong kind or unknown is the default for that field, with a warning on standard error, and other keys are kept when the file is written. A file that is not JSON is moved to `settings.json.bad`.
- A `--screenshot` ignores saved settings and saves none, so that pictures are reproducible, unless `--config-dir` is given.
- Saved: the map layers, 3D buildings, sunlight, cast shadows, stars and planets, event markers, information on click, globe or flat map, projection, frame rate limit, the body for the next start, the library service used as the base imagery, and a language code for names (empty follows the system; set in the file only). Not saved: the camera, the clock and which windows are open.

### Limitations
- The body's choice takes effect at the next start, and the window says so.
- Only services in the user's library are listed as imagery; the directory is reached through the services list (G). Overlays and opacity are not in the window.
- Language is not in the window, since names are decoded under it when the program starts.

## SpaceMouse

- A background thread looks for a 3Dconnexion SpaceMouse. It first tries the `spacenavd` socket at /var/run/spnav.sock, and if that is absent, the HID devices /dev/hidraw* whose vendor and product are 3Dconnexion's (vendor 0x256f, or 0x046d with a 3Dconnexion product number). It never affects start-up, and shows nothing when there is no device or no permission to read it.
- With no device, the only activity is a look every 5 seconds. A device that goes away stops the view at once, and one that comes back is found again within 5 seconds. A brief notice says "SpaceMouse connected" with the source.
- The window is woken only when the device's reading changes. While the cap is held away from its centre, frames are drawn continuously.
- Readings inside a small dead zone (6% of full deflection) count as zero, and the rest follow a curve that is finer than linear for small pushes and reaches full speed at full deflection.
- Push or pull the cap to climb or sink, in proportion to the height above ground. Sliding moves across the ground, faster the higher the view. Twisting turns the view and tilting tilts it, up to the limits and, past the horizon, into the sky. All speeds are per second, so they do not depend on the frame rate. The view stays at least 2 m above the ground.
- Moving the device ends a flight. Button 1 shows the whole body, and button 2 straightens up (on the globe only). On the flat map only slide (pan) and push or pull (zoom) are used.
- `--no-spacemouse` does not look for a device, and `--spacemouse-sensitivity N` scales all speeds (default 1).

### Limitations
- It has never been tried on a real device. The socket protocol and the HID reports come from public descriptions and are checked with byte fixtures and a fake socket. The axis signs and the HID report layout of the various models are assumptions; if a direction is wrong, it is a sign in `src/spacemouse_nav.rs` or `hid_to_frame` in `src/spacemouse.rs`.
- Reading /dev/hidraw usually needs permission. A udev rule such as `KERNEL=="hidraw*", ATTRS{idVendor}=="256f", TAG+="uaccess"` (and the same with `046d` for older models), or a running `spacenavd`, provides it. When `spacenavd` runs, the HID path is not used.
- Only the first matching HID device is used, and its buttons beyond the first two do nothing. The device's LEDs and its own settings are not touched.
- The first frame after a long rest moves by up to 0.1 s of motion at once.

## Frame rate

- The frame rate is limited to 120 fps (the default) or 90 fps, chosen with F, `--fps` or the view options. The display shows the limit, the measured rate and the display's refresh rate.
- Frames are drawn only when something changes, so an idle window uses next to nothing.
- A window cannot be shown faster than its display refreshes: on a 60 Hz monitor it runs at 60 fps.
- `--benchmark SECONDS` turns the view and prints frame timings, for measuring without input.

## Saved images

`--screenshot FILE.png` draws once all tiles have arrived (at most `--timeout` seconds), writes the image and exits. It can show every state of the interface, which is also how the program is tested without a window:

| Option | Effect |
|---|---|
| `--size WxH` | Size of the image |
| `--show-help`, `--help-scroll N`, `--show-options`, `--show-time` | Open those windows, and scroll the help |
| `--search-panel`, `--places-panel`, `--imports-panel`, `--eclipses-panel`, `--events-panel`, `--services-panel` | Show those lists |
| `--simulate-click X,Y`, `--simulate-drag X1,Y1,X2,Y2`, `--simulate-double-click X,Y` | Send input through the same code as the mouse, once the picture has loaded, so it lands on what the picture shows. Clicks and drags are repeatable; clicks are done first |
| `--flight-stop-at T` | Take the image T seconds into a simulated flight |
| `--simulate-nav SCRIPT`, `--nav-uploads N`, `--nav-draw-max-z Z`, `--nav-shots DIR`, `--mark-underlay` | Diagnostics for the view and tile loading: drive the view in steps, print what is drawn, and paint the base sphere so holes show |

With `--config-dir DIR`, a click on an option writes `DIR/settings.json`, and another run with the same DIR shows the same state.

## Recording the window

`--record DIR` keeps what the window shows, to look at a fault afterwards. DIR has to be new or empty, so that no earlier recording is overwritten.

- `DIR/frames/NNNNNN.png` is each drawn frame, as it was on screen but without the mouse pointer. The window only draws when something changes, so a still view gives no frames.
- `DIR/frames.jsonl` has one JSON line per drawn frame (time, view, clock, tiles, pointer and image), and one per click, wheel turn, key, resize or change of focus. Keys pressed while the search or add box is open are noted without saying which, so that what is typed stays out of it. The lines are written as they happen, so they survive a crash.
- `--record-limit GIB` (default 4) deletes the oldest images beyond that size, so the end of a long session is always kept. The log keeps all of it.

Copying and encoding run beside the drawing. When they fall behind, a frame loses its image: its log line then has `"image": null`, or a later `lost_image` line names it. The count is printed when the window closes. Recording can't go with `--screenshot` or `--serve-cache`.

## How far this is tested

The logic is covered by unit tests, which use neither the network nor a GPU, and the display is checked in saved images. Automated checks can't drive a real window, which leaves these with no automated check:

- Mouse and keyboard input in a real window, including dragging sliders, file drops, the "open article" button and clicks on high-resolution screens (the routing behind them is tested, also at scale 2).
- The feel of a flight: its speed, duration and tilt easing.
- The way shadows look while the clock or the camera moves.
- A real SpaceMouse (see [SpaceMouse](#spacemouse)).
