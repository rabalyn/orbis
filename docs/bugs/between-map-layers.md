# Bug: the view ends up between two layers of map when zooming in

Status: cause found and demonstrated, not fixed. Investigated on 2026-11-07 with the new `--simulate-nav` harness (see below).
Reported by the user on 2026-11-07 while testing a window build. It was also seen in an older build, so it predates the current milestone.

Short answer: the camera ends up **below the drawn terrain surface**. Navigation takes the ground height from the finest tile that is
loaded, but the screen draws a coarser parent tile until all the finer tiles of its area have arrived. Where the coarse mesh stands higher
than the real ground (valleys, ridge flanks, gorges), a camera held 2 to 30 m above the real ground is under the drawn mesh. Seen from
below, the mesh is the "stretched terrain above", the dark blue band is the base sphere (ocean colour at night), and the straight-edged
piece is a tile edge. It depends on **relief**, not on absolute height.

## What was seen

While zooming in, the view passes through a layer of map and keeps going, and then sits between two layers. It feels as if one layer of map
is drawn at some height instead of at ground level. It happens only sometimes. Seen close to Mount Everest.

Screenshot: [between-map-layers-everest.webp](between-map-layers-everest.webp)

- Top: snow-covered terrain with long vertical streaks where the image is stretched.
- Middle: a wide empty dark blue band, uniformly (1, 3, 21) in RGB, with nothing drawn on it.
- Bottom: a separate piece of terrain with a straight, angular edge against the band.
- Peak labels (Changtse Peak 7543 m, Kellas Rock Peak 7078 m and others) float over the band.

## Reproduction

The bug shows only while navigating, and `--screenshot` normally waits for all tiles, so a new diagnostic drives the real wheel path without
a window. New flags, all listed in `--help`, all used together with `--screenshot FILE.png`:

- `--simulate-nav SCRIPT` runs a script of steps through the same `Scene` mouse routing a window uses (wheel at a pointer, Alt-wheel for
  tilt), from the start view, and prints one line per logged step. Steps: `pX:Y` pointer, `z+N` wheel notches (positive zooms in), `a+N`
  Alt-wheel notches (tilt), `fN` N frames of 1/30 s without waiting for tiles (a log line follows), `s` wait until everything is loaded
  (log), `l` log, `d` list the drawn tiles, `uR` survey (below), `g:NAME` save NAME.png into `--nav-shots`.
- `--nav-shots DIR` where `g:` pictures go; a step that looks wrong (camera under the drawn surface, or over 1% of the picture showing the
  base sphere) also saves `auto-<step>.png` there.
- `--mark-underlay` paints the base sphere under the tiles magenta, so that anything that is not a tile shows up exactly.
- `--nav-uploads N` takes only N finished tiles per frame from the loader (default 12), to make tiles arrive slowly from a warm cache.
- `--nav-draw-max-z Z` draws no tile finer than Z, though they still load and give the ground height.

Each log line has: camera position, height above the ellipsoid (`h`), altitude over the ground used by navigation (`alt`), distance,
tilt, `nav_ground` (what `Renderer::ground` returns: the finest loaded tile), `drawn_under_cam` (mesh height of the tile actually drawn
under the camera), `below_drawn` (camera under that surface), the near plane, tile count and zoom range, flat tiles (all heights 0),
tiles with coarser imagery than their own level, tiles still wanted, the tallest drawn mesh point, the nearest drawn mesh vertex inside the
view (`front`) and how many are nearer than the near plane (`clipped`), the worst open crack between tiles after their skirts, and the
share of pure black and of magenta pixels in the picture. The far plane is infinite (reversed depth), so there is no far distance to log.

### 1. Natural occurrence, fresh cache (network timing decides, not repeatable on demand)

```
orbis --cache-dir /tmp/orbis-cold --mark-underlay --no-sun --lat 28.03 --lon 86.96 --distance 60000 --tilt 55 \
      --size 480x300 --screenshot /tmp/walk.png --nav-shots /tmp/walk-shots --timeout 200 \
      --simulate-nav "$(cat docs/bugs/between-map-layers-walk.txt)"
```

The script is a 140-step random walk of wheel and tilt steps (pointer anywhere, mostly zooming in). It downloads about 19 MB. In this run the
camera ended up under the drawn surface from step f13 to f27 (about 15 logged steps) while 50 to 60 tiles were still being fetched, for example:

| step | camera height | drawn surface under it | `nav_ground` | drawn zoom |
|---|---|---|---|---|
| f13 | 4894 m | 4926 m | 4691 m | z2..7 |
| f16 | 4705 m | 4969 m | 4646 m | z2..9 |
| f19 | 4666 m | 4790 m | 4643 m | z2..10 |
| f28 (finer tiles drawn) | 4664 m | 4657 m | 4643 m | z2..12 |

At the worst the camera was 264 m below the drawn mesh. With `--mark-underlay` most of the picture is magenta (the base sphere), with a wedge of
tile seen from below: [between-map-layers-natural-magenta.jpg](between-map-layers-natural-magenta.jpg) (steps f13, f16, f19).
In four fresh-cache runs (Everest summit, Everest north, Kali Gandaki, this walk) the camera went under the surface in two: this walk
(long and deep) and Kali Gandaki (once, by 8 m). The other two zoomed onto summits, where it does not happen (see "relief" below).
Warm-cache walks (three seeds of 120 steps, with and without `--nav-uploads 1`) never produced it, because tiles then arrive too fast.

### 2. Held state, repeatable once the tiles are in the cache

This freezes the state of "the finest tiles have arrived for the camera, the coarse parent is still drawn":

```
orbis --cache-dir /tmp/orbis-cold --nav-draw-max-z 7 --lat 27.846 --lon 87.1596 --distance 30000 --tilt 50 \
      --size 640x400 --screenshot /tmp/hold.png --nav-shots /tmp/hold-shots --timeout 60 \
      --simulate-nav "f20,z+6,f10,z+6,f10,z+6,f10,z+6,f10,z+6,f10,z+3,f40,a-4,f10,g:v1,a-3,f10,g:v2,a-3,f10,g:v3,l"
```

Result: camera 4648 m, drawn z7 surface 4935 m, `nav_ground` 4644 m, so 286 m under the mesh, tilt 78.6 degrees, altitude 4.6 m over the real
ground. The pictures (no magenta, normal colours, night time) show the same look as the user's screenshot: vertical streaks on top, an empty
band, straight tile edges: [between-map-layers-under-surface.jpg](between-map-layers-under-surface.jpg). The band measures (1, 3, 20) here
against (1, 3, 21) in the user's picture. The right-hand picture of the three is the same state with the temporary fix described below.

## Results by place

Method: camera 3000 m over the ground, tilt 40, 640x400, fresh cache, steps `s,l,u3000` (`u3000` compares, on a 41x41 grid within 3 km of the
view point, the mesh height of each zoom level's tile with the finest loaded ground; a camera standing on the finest ground is under a drawn
mesh of that level wherever the difference is positive). Points are only counted where a tile of zoom 10 or finer exists, so there are
about 70 to 130 points at z7 and z9 and only 4 to 25 at z11 and z12. "Undercut" is the largest height by which the coarse mesh stands above
the finest ground, in metres. The camera can be as low as 2 m (clearance) to about 30 m over the ground at the closest zoom.

| place | ground in the area (m) | relief (m) | undercut z7 | z9 | z11 | z12 | camera under the mesh? |
|---|---|---|---|---|---|---|---|
| Everest 27.99N 86.93E | 3590 to 8309 | 4719 | 639 | 478 | 81 | 23 | yes in valleys: seen at 27.846N 87.16E; no on the summit (the coarse mesh is lower than the peak) |
| Changtse / Kellas 28.04N 86.96E | 3109 to 7146 | 4037 | 1009 | 695 | 122 | 25 | by survey yes; camera on the summit: no |
| Kali Gandaki 28.69N 83.64E | 1030 to 6984 | 5955 | 1211 | 446 | 20 | 14 | yes, seen once by 8 m in a fresh-cache run |
| Matterhorn 45.98N 7.66E | 339 to 4387 | 4048 | 1157 | 499 | 57 | 2 | by survey yes |
| Denali 63.07N 151.01W | 313 to 6131 | 5818 | 719 | 171 | 21 | - | by survey yes |
| Yosemite Valley 37.75N 119.59W | 532 to 3611 | 3079 | 774 | 280 | 65 | 8 | by survey yes |
| Changthang 33.0N 88.0E (about 4700 to 5700 m) | 4731 to 5738 | 1006 | 157 | 83 | 29 | 3 | marginal: only at coarse levels or low z11 |
| Salar de Uyuni 20.13S 67.49W (3650 m) | 3654 to 4759 | 1105 | 176 | 53 | 0.5 | 0.5 | not at z11 or finer |
| Netherlands 52.09N 5.12E (0 to 25 m) | -6 to 26 | 32 | 10 | 5 | 0.4 | 0.3 | no (below the 2 m clearance except at z7) |

Only the Everest east case and the Kali Gandaki case were seen in a rendered view; the other "by survey" entries are not rendered and rest on
the measured height difference. The "ground" and "relief" columns come from the finest loaded tiles in the 6 km square, not from a map.
The hold test with the camera at the Everest and Changtse summits showed no under-surface frame (the coarse mesh is lower than the peak, the
camera floats above it); the tests for the other places were not completed (each took several minutes and the summit result made them
uninformative).

Answer: it depends on **relief**, not on absolute height. The plateaus at 3650 m (Uyuni) and 4800 m (Changthang) have little mismatch (0.5 and
29 m at z11) while the sea-level hilly Yosemite and the Alps have a lot, and flat ground at any height has none. The height difference between
a coarse and a fine mesh grows with the relief inside one vertex spacing of the coarse mesh, so it is large in the Himalaya, where tile
meshes are far apart in height, and small on plains. The sign matters too: coarse meshes cut peaks and fill valleys, so the camera is under the mesh
on valley floors, gorges and lower slopes and above it on summits.

## Demonstrated cause

1. `Renderer::ground` (used by `Nav::tick` for the camera's minimum height, the start phase and the near plane) returns the height of the
   finest *loaded* tile at a position.
2. `View::select` replaces a tile by its children only when all visible children are loaded, so a coarse parent is drawn while a finer tile
   under the camera already exists. This is deliberate (no holes), and tiles near the camera are loaded first.
3. The coarse parent's mesh height at the camera position differs from the fine one (survey above: up to 80 to 120 m at z11 in the Himalaya,
   hundreds of metres at z7 to z9). Where the parent is higher, the camera, kept 2 to 30 m over the fine ground, is below the drawn surface.
4. Tile drawing is two-sided (no face culling), so from below the mesh is seen from behind. Above the camera is the underside of the
   coarse tile; around it the skirts hang down; beyond the tile's edge, down to the base sphere at -12 000 m, nothing is drawn.

Tests of the cause (each of which could have failed):

- The base sphere is the band: with `--mark-underlay` the band turns magenta in the natural run (steps f13 to f27); without it the band
  measures (1, 3, 20) against (1, 3, 21) in the user's picture. The sphere uses the ocean colour (17, 31, 72), darkened by night shading.
  It is not the clear colour (that is black and only shows beyond the globe).
- The streaks are the skirts: with a temporary local patch making the skirts 1 mm deep instead of 2% of the tile width, the vertical
  coloured streaks vanish from the same held state and only the underside of the tile remains
  ([between-map-layers-skirts.jpg](between-map-layers-skirts.jpg), left with skirts, right without; top row and bottom row are two tilts).
  Skirts of coarse tiles are large (about 1.5 km at z9, 780 m at z10), and they take the colour of the tile's edge pixels, hence the streaks.
- The camera under the mesh is the cause of the look: with a temporary local patch that gave navigation the height of the *drawn* tile
  instead of the finest loaded one, the same held state stayed above the surface in all 6 steps (camera 4955 to 5307 m against a drawn surface of
  4935 to 4913 m) and the pictures are normal (right picture of the under-surface image). Without the patch it was under in all 6 (by 270 to 286 m).
  Both patches were local and are reverted; nothing of them is committed.

## Ruled out

- Near-plane clipping: near is 0.1 of the altitude, at least 0.5 m. In three random walks of 120 steps at Everest (warm cache, with and
  without slow tile arrival) the nearest mesh vertex inside the view was never closer than 7.5 times the near plane, and no vertex was clipped.
  At the under-surface frames the camera is below the surface, not beside it.
- Depth precision: the depth buffer is 32-bit float with reversed, infinite far plane. No z-fighting or clipping was seen in any step.
- Tiles drawn flat at 0 m beside tiles at 7000 m: no drawn tile had all heights 0 in any log (the `flat` column). The loader builds a tile only
  when both imagery and elevation are in hand; a flat surface comes only from a "missing" answer for elevation, which did not happen here.
- Imagery and elevation at different levels: every drawn tile had its own imagery (`coarse_imagery` 0 in all logs), and the elevation of
  a tile is always taken from its own area. The streaks are skirts, not a mismatch of uv.
- Holes in the selection (a visible tile treated as hidden): with `--mark-underlay` the magenta share stayed 0 in three warm random walks, three
  full tilt sweeps at five zoom levels, and the fresh-cache runs, apart from the under-surface frames.
- Cracks between neighbouring tiles of different level: the worst step beyond the skirts was 1014 m at one point of the fresh-cache walk, but
  those were thin slits of a few edge points (at most 10); they never made a band.
- Back-face culling: tile drawing has no culling, so it does not hide the problem, and it is what makes the underside visible. Not a cause.
- The camera dips slightly (down to 1.5 to 5.6 m under) in warm runs because the collision uses a bilinear height on the mesh grid while the
  mesh is triangulated. This is a separate small effect, not the band.

## Not shown

- The exact frame of the user's screenshot. The look, the colour of the band and the streaks agree, but the bottom piece with a tile-shaped edge
  and its high-resolution imagery would need a state where a fine tile is drawn next to the camera and the coarse neighbour stands above it.
  That is a hypothesis; it is consistent with the mechanism but not reproduced.
- Real window timing. The natural cases are from the harness, whose frame steps are 1/30 s with 15 ms of real sleep.

## Proposed fix (not applied)

Give navigation the height of what is drawn. In `Scene::update`, pass `Nav::tick` a ground function that takes the mesh height of the tile
in the previous frame's draw list that contains the position (the same grid lookup `Renderer::ground` already does), and use the larger of
that and the finest loaded height, so the camera is above both. The near plane's altitude follows from the same function.

- Scope: one new method on `Renderer` (a `ground` that takes the draw list), the closure in `Scene::update`, about 20 lines. It can be checked
  with the harness: the held-state command above must show `below_drawn=false` and no magenta in every step.
- Risk: when finer tiles replace the coarse ones, the surface drops by up to the undercut (about 100 m in the Himalaya at one level), and
  the camera, which `Nav` lifts at once but lowers with its smoothing, follows at the speed of `follow_ground` (6 per second). So there is a short
  visible float over the ground when zooming in while loading, instead of a view from below. The start phase and the min-distance of 30 m are
  not affected. A different option (drawing the finest loaded tile under the camera over its coarse parent) would remove the float but give overlapping
  meshes and depth fighting, so it is worse.
- Not enough on its own for the plateaus' small effects, which need none.
