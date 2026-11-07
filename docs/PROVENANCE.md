# Provenance

Orbis (working name) is an independent Rust program for Linux with a similar
purpose to Kugelblitz (© 2026 Tim Pritlove). It is not affiliated with or endorsed by its
author. This file records how it is built, so the boundary stays clear.

## Rules

- The original binaries are not decompiled or disassembled, and their strings and
  symbols are not used as a source of design.
- No code, shaders, help text, images, icons or name are taken from the original.
- The original's curated list of map services is not copied. Each source is evaluated from
  its publisher's own documentation.
- The Kugelblitz tile server is not used.

All three data sources below are public services that the original also uses or offers,
which the earlier survey and the help text show. Facts about public services are not its
code, but choosing the same sources is not independent of it, so this is recorded here.

## What the original was used for

Before these rules were set, the bundle was unpacked and its metadata, strings, symbol
names and linked frameworks were surveyed to judge feasibility. Nothing was decompiled.
That survey is not a design input.

The design inputs are:

- the published product page and the app's user help, read for what the app does,
  and restated in our own words in [SPEC.md](SPEC.md)
- public standards and format documentation
- general geodesy and real-time rendering practice

## Data sources in use

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Imagery | [EOxCloudless](https://cloudless.eox.at), 2024 mosaic | CC BY-NC-SA 4.0 for non-commercial use, commercial use needs an EOX licence | `EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2024)` |
| Terrain | [Mapterhorn](https://mapterhorn.com) | Open-data sources listed on its [attribution page](https://mapterhorn.com/attribution) | `© Mapterhorn https://mapterhorn.com/attribution` |
| Roads and labels | [OpenFreeMap](https://openfreemap.org), OpenMapTiles schema, OpenStreetMap data | Free with no key or view limit, attribution required, bulk offline downloads limited | `OpenFreeMap © OpenMapTiles Data from OpenStreetMap` |

Aerial imagery for close views comes from national and regional services, listed with their
terms in [CACHING.md](CACHING.md). Their credits appear on screen while their imagery is in
view. Terms were read from basemap.at, the Swiss federal geoportal and cartes.gouv.fr, and from
the capability documents of the PDOK, Bavaria, NRW, Lower Saxony and Berlin services, IGN Spain's licence
document, and VersaTiles' sources page, which lists a licence per country. USGS products are
listed as US public domain from general knowledge; its terms page could not be loaded.

### Stars and planets

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Star positions, magnitudes and colours | [HYG database v4.4](https://www.astronexus.com/projects/hyg) by David Nash (compiled from Hipparcos, Yale Bright Star and Gliese catalogues), file `data/hyg/CURRENT/hyg_v44.csv.gz` in the [archive](https://codeberg.org/astronexus/hyg) | Creative Commons Attribution-ShareAlike 4.0, stated on the project page, in the archive's README and its LICENSE file. Redistribution and adaptation are allowed with credit, and adaptations under the same licence | `Stars: HYG database by David Nash, CC BY-SA 4.0` |

`data/stars.bin` is a derived table of 5,070 stars of magnitude 6.0 or brighter, six bytes each (right
ascension, declination, magnitude and colour index, quantised), made with `tools/gen_stars.py` from the
HYG file above. It is an adaptation of the HYG database, so it is under CC BY-SA 4.0 too. Compiling it
into the program is a collection and does not change the licence of the program's own code.

The planets use the orbital elements of JPL's "Keplerian elements for approximate positions of the major
planets" (Table 1, 1800-2050, https://ssd.jpl.nasa.gov/planets/approx_pos.html), the Moon uses the
Astronomical Almanac's low-precision lunar formulae, and sidereal time and precession follow the Almanac
and Meeus, "Astronomical Algorithms". These are published formulae and constants, not data files.

The credits must stay visible wherever the data is displayed, including saved images.
Label text comes from the system's installed fonts; none are bundled.
EOX's license page does not explicitly address its public WMTS endpoint, and Mapterhorn's
pages state no usage conditions for its public tiles. Recheck both before publishing.
