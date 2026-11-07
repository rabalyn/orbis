# Bug: the view ends up between two layers of map when zooming in

Status: open, not yet investigated.
Reported by the user on 2026-11-07 while testing a window build. It was also seen in an older build, so it predates the current milestone.

## What was seen

While zooming in, the view passes through a layer of map and keeps going, and then sits between two layers. It feels as if one layer of map is drawn at some height instead of at ground level. It happens only sometimes, and the user hasn't yet checked whether it depends on the location.

Seen close to Mount Everest. No exact coordinates or reproduction steps are known.

Screenshot: [between-map-layers-everest.webp](between-map-layers-everest.webp)

## What the screenshot shows

- Top: snow-covered terrain, seen as if from below or at a grazing angle, with long vertical streaks where the image is stretched.
- Middle: a wide empty dark blue band with nothing drawn on it.
- Bottom: a separate piece of terrain with a straight, angular edge against the blue band, in the shape of a tile's outline.
- Peak labels (Changtse Peak 7543 m, Kellas Rock Peak 7078 m and others) float over the blue band.

## Check requested by the user

1. Reproduce near Mount Everest.
2. Try other mountains and valleys, to see whether it only happens at altitude.
3. Compare with flat plateaus at different heights.
4. Each check is a zoom in together with a camera tilt up and down, to see whether the view ends up between layers.
