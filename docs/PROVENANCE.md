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

### The Moon

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Imagery | LRO Wide Angle Camera global mosaic (`LRO_WAC_Mosaic_Global_303ppd_v02`), served by [NASA Moon Trek](https://trek.nasa.gov/moon/) as a WMTS layer. Its capabilities document, `https://trek.nasa.gov/tiles/Moon/EQ/LRO_WAC_Mosaic_Global_303ppd_v02/1.0.0/WMTSCapabilities.xml`, gives the layout: 256-pixel JPEG tiles, plate carrée matrix set with 2 by 1 tiles at level 0 and levels 0 to 8, tiles at `{z}/{row}/{col}.jpg` | NASA media usage guidelines (nasa.gov, "NASA Images and Media Usage Guidelines"): NASA content, including texture maps, is generally not subject to copyright in the United States; use for educational or informational purposes including computer graphical simulations and web pages is allowed, NASA should be acknowledged as the source, and no endorsement may be implied. Moon Trek's own terms page could not be found, and the service states no request limits, so requests are kept gentle (two at once, cached on disk). Recheck before publishing | `Moon: LROC WAC mosaic, NASA/GSFC/Arizona State University, via NASA Moon Trek https://trek.nasa.gov/moon/` |

The Moon's rotation uses the IAU Working Group on Cartographic Coordinates and Rotational Elements
report (Archinal et al., Celestial Mechanics and Dynamical Astronomy 109, 101-135, 2011, and the 2015
report, 130, 22, 2018), whose Moon constants were checked against NASA NAIF's `pck00011.tpc`. Its mean
radius, 1737.4 km, is from the same report. Earth's size in the lunar sky uses 6371 km. These are
published constants, not data files.

NASA Moon Trek also serves LOLA elevation (`LRO_LOLA_DEM_Global_128ppd_v04`) as 8-bit grey PNG
tiles with no documented height scale, so it is not used.

The credits must stay visible wherever the data is displayed, including saved images.
Label text comes from the system's installed fonts; none are bundled.
EOX's license page does not explicitly address its public WMTS endpoint, and Mapterhorn's
pages state no usage conditions for its public tiles. Recheck both before publishing.

### Mars, Mercury, Ceres and Vesta

| Data | Source (capabilities document under `https://trek.nasa.gov/tiles/<Body>/EQ/<layer>/1.0.0/WMTSCapabilities.xml`) | Credit shown in the app |
|---|---|---|
| Mars imagery | NASA Mars Trek WMTS layer `Mars_Viking_MDIM21_ClrMosaic_global_232m`: 256-pixel JPEG, default028mm plate carrée set with 2 by 1 tiles at level 0, levels 0 to 7 | `Mars: Viking MDIM 2.1 colour mosaic, NASA/JPL/USGS, via NASA Mars Trek https://trek.nasa.gov/mars/` |
| Mercury imagery | NASA Mercury Trek layer `Mercury_MESSENGER_MDIS_Basemap_BDR_Mosaic_Global_166m`, levels 0 to 7 | `Mercury: MESSENGER MDIS basemap, NASA/JHUAPL/Carnegie Institution of Washington/USGS, via NASA Mercury Trek https://trek.nasa.gov/mercury/` |
| Ceres imagery | NASA Ceres Trek layer `Ceres_Dawn_FC_HAMO_ClrShade_DLR_Global_60ppd_Oct2016`, levels 0 to 5 | `Ceres: Dawn FC HAMO colour mosaic, NASA/JPL-Caltech/UCLA/MPS/DLR/IDA, via NASA Ceres Trek https://trek.nasa.gov/ceres/` |
| Vesta imagery | NASA Vesta Trek layer `Vesta_Dawn_FC_HAMO_Mosaic_Global_74ppd`, levels 0 to 6 | `Vesta: Dawn FC HAMO mosaic, NASA/JPL-Caltech/UCLA/MPS/DLR/IDA, via NASA Vesta Trek https://trek.nasa.gov/vesta/` |

The layer names were found by trying the Trek tile directory (which returns 403 for listing) and confirmed by
their capabilities documents, which also give the coordinate system (the IAU2000 or ESRI code of each body). Terms
are those for the Moon: NASA media usage guidelines (NASA content, including texture maps, is generally not
subject to copyright in the United States; acknowledge NASA as the source; no endorsement may be implied). The Trek
sites' own terms pages could not be found and no request limits are stated, so requests are kept gentle (two at
once, cached on disk, host pacing shared with the Moon). The credit lines name the data products and their
teams from general knowledge of these missions, not from a credit text published by Trek, so recheck them before
publishing. The Trek layers also offer other products (THEMIS and MOLA for Mars, elevation for all) that were not
used.

Rotation constants and radii are from the IAU Working Group report (Archinal et al., Celestial Mechanics and
Dynamical Astronomy 130, 22, 2018) as listed in NASA NAIF's `pck00011.tpc`, which was read for this.
Orbital elements of Ceres and Vesta are osculating elements for 2026-11-21 (JD 2461200.5) from the JPL Small-Body
Database API (`ssd-api.jpl.nasa.gov/sbdb.api`). The tests quote results from the JPL Horizons API
(`ssd.jpl.nasa.gov/api/horizons.api`) as reference values; they are facts, not data files. Mars's and Mercury's
orbits use the JPL approximate planet elements already listed above. These are published constants, not data files.
