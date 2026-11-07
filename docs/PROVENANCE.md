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

The credits must stay visible wherever the data is displayed, including saved images.
Label text comes from the system's installed fonts; none are bundled.
EOX's license page does not explicitly address its public WMTS endpoint, and Mapterhorn's
pages state no usage conditions for its public tiles. Recheck both before publishing.
