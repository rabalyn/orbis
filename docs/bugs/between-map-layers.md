# Bug: the view ends up between two layers of map when zooming in

Status: **fixed** by the commit "Keep the camera above the surface as drawn, not only above the finest loaded ground". Investigated on 2026-11-07 with the new `--simulate-nav` harness (see below).
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

## Fix (applied)

Navigation now keeps the camera above the higher of the finest loaded ground and the surface as drawn.

- `loader::height_on_mesh` gives the height of a tile mesh with the two triangles per cell that `mesh_indices` draws (a unit test compares it
  with barycentric interpolation of those triangles, and with the bilinear blend that differs by tens of metres on rough cells).
  `Renderer::drawn_ground(draw, lat, lon)` looks the position up in a draw list. `Renderer::ground` is unchanged (the true finest ground),
  so labels, picks, flights' targets, `--simulate-*` and the view point of navigation keep it.
- `Nav::tick_over(dt, ground, floor)`; `Nav::tick` calls it with the same function twice. The view point and the start phase follow `ground`;
  the clearance (2 m) and the altitude used for the near plane follow `floor`. Flights end through the same clearance, so they end above the drawn surface.
- In `Scene::update` the camera is checked again after the tile selection of the same frame (not only against the previous frame's draw list,
  which missed the case of the view moving onto a coarser tile): if it is under the new selection it is lifted and the selection is made again,
  up to three passes. The lift goes back along the view line as before, and straight up if the line climbs more slowly than the slope.
- The log's `drawn_under_cam` now uses the triangle height too (it used the bilinear one, which differs by up to tens of metres from what is drawn).
- `--simulate-nav` prints a last line with the largest rise and fall of the camera height between two frames (rises include the user's own zoom out),
  and the largest rise in a frame where the ground held or lifted the camera.

### Results

| check | before | after |
|---|---|---|
| held state, 11 logged steps (the cache of the first run was gone, so tiles were downloaded again) | `below_drawn` true in steps f4 to f10, camera 4.6 m over the true ground, with the camera about 280 m under the z7 mesh | `below_drawn` false in all 11 steps, no magenta (`hole=0.000`) with `--mark-underlay`; the camera sits 2 m over the drawn surface while the z7 tile is shown (4894 m against `nav_ground` 4742 m, a float of 150 m) |
| natural walk, fresh cache, 141 steps | under the surface in 15 steps (report) and again in 3 steps (f60 to f62) with the first version of this fix, which only looked at the previous frame's tiles | 0 of 141 steps under; 19.1 MB downloaded; the camera floats up to about 190 m over the true ground while 40 to 57 tiles are wanted (steps f15, f16) |
| same walk, warm cache, `--nav-uploads 1` | 0 steps under (it never reproduced warm) | 0 steps under; largest rise while the ground held the camera 25.6 m in one frame |
| held state, largest rise in a frame while the ground held the camera | - | 400.8 m (tilting to 78 degrees with z7 forced on screen; an extreme case made by `--nav-draw-max-z 7`) |
| closest zoom at Everest summit, warm | h 8524.2 m, alt 30.3 m, distance 30 m | identical |
| closest zoom at Netherlands 52.09N 5.12E | h 28.6 m, alt 22.9 m, distance 30 m | identical |

Whole-run rises and falls in camera height (906 frames) come from the user's own wheel input (about 1000 m rises and 9000 m falls, the same in old
and new), so they say nothing about the fix; the last column of the table is the useful number.

### What the user sees, and what remains

- While finer tiles are still on their way in steep country, the camera floats above the true ground by up to the difference between a coarse and the
  fine mesh (150 m in the held state, up to about 190 m in the fresh walk), and then stays at that height: it is never lowered by the loading. Once the
  finer tiles are drawn the floor is the fine ground again, and zooming down to the closest distance works as before (the numbers above).
- When the view turns or moves onto a coarse tile of an area that is not loaded yet, the camera is lifted in one frame (it cannot be eased without being
  under the drawn surface for some frames). In the 120-step walk the largest such rise was 26 m per frame; the extreme test with z7 forced gave 400 m.
  Tiles near the camera are loaded first, which keeps it small in real use. Not tried in a real window.
- The lift is still decided from the selection of the frame, so a view that changes its tile selection after the lift (three passes at most) could
  leave the last pass stale by a few metres. Not seen in any log.
- The fresh-cache walk was run twice with the code as it changed (about 20 MB each time); the second run is the one in the table. The timing of the real
  network decides whether the natural case occurs, so "0 of 141" is one run, not a proof.
