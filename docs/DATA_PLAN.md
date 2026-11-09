# Aerial roofs and Our World in Data: plan

Two wishes from 2026-11-09, sorted into milestones that can be built one after another. Each milestone ends in a state
that's worth committing on its own, with its tests, `FEATURES.md`, `PROVENANCE.md` and the help window brought up to
date. Once all are done, this file is deleted, like the plans before it.

## Where things stand

- **Buildings** are OSM footprints with `render_height` from OpenFreeMap's vector tiles at zoom 14
  (`vector::BUILDING_Z`), extruded in `buildings.rs` into one baked mesh per tile, flat-topped, coloured by zone
  (green living, blue business, yellow industry, darker when taller) or the OSM colour. Their vertices carry position
  and colour only; `fs_building` in `globe.wgsl` shades faces from their slope.
- **Aerial imagery** (EOx, national orthophotos, VersaTiles) is laid on the terrain tile by tile in `loader.rs`; the
  roofs in it are hidden under the extruded boxes, whose flat colour says nothing about what is really there.
- **Our World in Data is not used.** Germany's data is finer than anything OWID has: plants from the
  Marktstammdatenregister, areas and population from BKG, quarter-hourly generation and load from SMARD.
- **What OWID has** (CC BY 4.0; third-party sources keep their terms and are named in each file's metadata):
  - `catalog.ourworldindata.org/energy/owid_energy/owid_energy.csv`: one row per country and year, 129 columns:
    population, GDP, generation, demand, net imports, carbon intensity and emissions of electricity, generation and
    share by source (coal, gas, oil, nuclear, hydro, wind, solar, bioenergy, other renewables), primary energy.
    Electricity for about 200 countries up to 2025 (Ember, Energy Institute, EIA). 8.7 MB.
  - The chart API `ourworldindata.org/grapher/<slug>.csv` with `csvType=filtered`, `country=~DEU`, `time=…`,
    `useColumnShortNames=true`, and `.metadata.json` for sources and licences. Old slugs redirect (301).
    `electricity-mix` with `frequency=monthly&metric=generation&source=<solar|wind|…|total>` gives **monthly**
    generation by source for 88 countries and regions (Ember), up to June 2026.
- **Checked by hand:** Ember gives Germany 500.5 TWh generated and 520.3 TWh used in 2025, 59.1 % renewable,
  330 g CO₂/kWh. SMARD's public net generation is lower, since it leaves out generation for the owners' own use; that
  gap is why `FEATURES.md` says shares of rooftop solar and industrial plants run low. OWID's population for Germany
  (84.1 million, UN estimate) is less exact than BKG's 83.5 million census-based count, so BKG stays.

## Milestone 1: aerial roofs

Not a second toggle beside the buildings: aerial roofs and zone colours are two ways of colouring the same buildings,
so they are one choice.

- **Building colours** in the view options: *Aerial roofs* (new, the default) or *Zones* (as now). `B` keeps showing or
  hiding the buildings; the choice is saved with the settings (`buildings_colour`).
- **Roofs** show the imagery of the ground they stand on: a roof vertex carries its position in the building tile, and
  the roof is textured from a picture of that tile composed from the best imagery the loader has for it (orthophotos
  where a country has them, else EOx/VersaTiles), at about 1 m per pixel (zoom 17, so 8×8 imagery tiles per building
  tile; coarser where finer is missing). The picture comes through the same fetcher and disk cache as the ground, in
  the worker that bakes the mesh, so no frame waits for it. Until it is there, roofs show their zone colour.
- **Walls** get one plain colour per building, the roof's average darkened, so a red-roofed house has warm walls and a
  grey block grey ones; the existing wall shading and the darker base stay.
- Shadows, the ghost pipeline of the energy bars and the energy bars themselves are untouched.
- Tests: roof texture coordinates span the tile and stay in it for clipped footprints; walls use the averaged colour;
  the composed picture places each imagery tile at the right spot (a synthetic cache, as in the loader tests); the
  setting round-trips. A screenshot of the Lincoln-Siedlung in Darmstadt (`--goto`-style start at 49.8451 N,
  8.6426 E, 473 m) before and after goes to `screenshots/`.
- Limitations to write down: orthophotos other than true orthophotos show tall roofs shifted away from the image
  centre, so a roof can show a bit of its own wall or the street; the walls are not photographs.
- Done when: the Darmstadt view shows the real roofs (flat grey roofs, solar panels, the red roofs of the older blocks)
  and switching back to *Zones* restores today's look exactly.

## Milestone 2: OWID data, built in

- `tools/gen_owid.py` downloads, or reads from given files, the OWID energy CSV and the monthly `electricity-mix`
  series (generation by source and total, demand where present), keeps only the columns the program uses, and writes
  `data/owid_energy.bin` (compact binary, like the other `gen_*` tools; aim for under 1 MB). Per country (ISO 3166
  alpha-3): yearly population, generation, demand, net imports, carbon intensity, generation by source; monthly
  generation by source where Ember has it. Regions and OWID aggregates (`OWID_*`) are left out apart from the World.
- **Country outlines and centres** for the later milestones come from Natural Earth admin-0 at 1:50m (public domain),
  simplified, keyed by the same ISO code (with Natural Earth's known exceptions such as France and Norway handled),
  in the same file or a sibling `data/countries.bin`.
- `src/owid.rs` reads it: lookups by country and year or month, the latest value at or before a date, and what each
  figure's source is.
- Nothing is requested at runtime. Rebuilding is one command, as for `energy_de.bin`.
- `PROVENANCE.md` gets the sources with their licences (OWID CC BY 4.0; Ember CC BY 4.0; Energy Institute and EIA as
  named in OWID's metadata; Natural Earth public domain); the credit line names OWID when its data is in view.
- Tests: the reader on a small fixture written by the tool; Germany's 2025 figures as given above; a country with
  annual but no monthly data; months and years before the first value.

## Milestone 3: checking and completing Germany's figures

- **Check**: `tools/check_owid.py` sums SMARD's quarter hours (from the program's disk cache or fresh) into months
  and years per source and lists them against Ember's monthly figures for Germany, with the ratio. The result, and
  what explains the gaps (own use, net against gross, pumped storage), goes into `ENERGY.md`. A test keeps the
  program's mapping of SMARD series to OWID sources in step.
- **Own use**: an option "Include generation for own use" (on by default) scales the estimated output of each source
  so that its monthly sum matches Ember's for that month: the quarter-hour shape stays SMARD's, the level becomes
  Ember's. The status line and the bar box say which is shown. For months Ember has not published yet, the factor of
  the latest month of the same season (the same month a year earlier) is used and the display says so.
- **Carbon intensity**: the status line and the day chart gain the carbon intensity of Germany's electricity at the
  clock's quarter hour, from the SMARD mix and per-source emission factors (IPCC lifecycle medians, documented in
  `PROVENANCE.md`), calibrated so that the monthly average matches Ember's figure. The day chart draws it as a thin
  line on its own axis.
- **Year at a glance** in the bar box for Germany's states and above: the year's renewable share and its change over
  the year before, from Ember.
- Tests: scaling keeps the shape and hits the monthly sum; the season fallback; intensity of an all-wind and an
  all-lignite quarter hour; the calibration.

## Milestone 4: countries of the world

- A new coarsest level, **Countries**, above States (a little above 1,500 km, with the same hysteresis): one bar per
  country, standing at its centre, its outline drawn on the ground as the other areas are. Germany's bar at this level
  is Germany's state bars summed, so it fits in.
- **What a country bar shows**: the average power generated by each source in the clock's month (TWh per month into
  GW), stacked in the existing colours and order, fossil and nuclear dashed in the second column as now; the white use
  column beside it is demand. Where Ember has no monthly figures, the year's average stands in, drawn lighter, and
  the box says so. There is no capacity ghost for other countries: OWID does not have per-source capacity for all of
  them; this is said in the display.
- **Height** follows the existing rule (fixed in the landscape, scale from the 98th percentile of the level, given as
  the height of 10 GW).
- A **click** on a country bar opens the box: the month's generation by source as bars, demand, net imports, the
  renewable share and carbon intensity, and a small line of the renewable share over the years, all from Ember via
  OWID, credited.
- **Flat map**: countries shaded by the renewable share of the clock's month (or year), with a small legend, while
  the energy layer is on. Bars stay globe-only.
- The time window's year slider drives this level too; before the first value a country shows no bar.
- Tests: the level ladder with the new band and its hysteresis; monthly against annual fallback; the GW conversion
  (days per month); Germany's country bar equals the sum of its states; the map shading scale.

## Milestone 5: country facts in the information box

- A click on a country's name or ground, with the energy layer off too, adds a **Data** tab beside Article and
  Infobox in the information box: population, electricity per person, renewable share, carbon intensity and net
  imports for the clock's year, each with a small line over the years and its source.
- Only bundled data from milestone 2; no new requests. Countries are matched by Wikidata's ISO 3166 code (P298)
  where the box already has the Wikidata item, else by the country outline under the clicked point.
- Tests: matching by code and by outline; a country without data shows no tab; figures for a year beyond the data
  use the latest year and say so.

## Not in this plan

- Google's Photorealistic 3D Tiles: closed data behind a paid key whose terms forbid caching, which this program
  depends on.
- Roof shapes from Overture Maps or German LoD2 models: a separate step after milestone 1, if the flat roofs bother.
- Fetching OWID at runtime: the bundled file is rebuilt with the program, as for the other `data/` files.
- Other OWID topics (health, economy, disasters): the energy figures fit what the program already shows; the Data
  tab in milestone 5 is where more could go later.
