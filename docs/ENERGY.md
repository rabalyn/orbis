# Energy layer: design sketch

Phase 1 (plants as bars, build-out over time), fossil plants for comparison, phase 3 (output split nationally, with a day chart of generation against consumption, from SMARD instead of Energy-Charts, whose terms do not allow redistribution) are implemented; [FEATURES.md](FEATURES.md#renewable-energy-in-germany)
describes it. The rest is a proposal. It shows where renewable electricity is produced in Germany, how much is produced at the
clock's moment, and the transmission grid that has to carry it. Germany comes first because its open data is the best in
Europe; other countries follow in the last phase.

## What the viewer sees

- **Plants as bars.** A column stands on the terrain at each plant (or cluster of plants). An outlined "ghost" column is
  the installed capacity; the solid part inside it is the output at the clock's moment. A tall ghost that's mostly empty
  is a big plant producing little right now, so a 5 MW park and a 2 GW offshore cluster can be compared at a glance.
- **Colour by source**: solar, onshore wind, offshore wind, hydro, biomass. Filter switches in the view options, like the
  existing line layers.
- **Clustering by zoom.** From space, Germany would be thousands of overlapping needles. Plants are summed into cells
  whose size follows the zoom level (one bar per cell, sum of capacity and output). Close up, each wind turbine or solar
  park stands alone.
- **Height on screen.** Bar height is scaled with camera distance, so bars stay readable from whole-Earth view down to
  a single park, and a legend gives the MW per unit of height.
- **Time.** The existing clock and time window drive everything:
  - *Output* follows the clock (15-minute steps; Space and the clock speed keys play a day forward).
  - *Build-out*: a plant appears only from its commissioning date and disappears after decommissioning. Dragging the
    year slider from 2000 to 2026 shows the build-out of wind and solar.
- **Measured vs. estimated** is always visible: solid fill for measured per-plant values, hatched or lighter fill for
  estimated ones (see below). The credit line names the sources in view.

### Grid

- **Transmission lines** (380 kV, 220 kV, HVDC; 110 kV when close) draped on the terrain like roads, coloured and sized
  by voltage, plus substations as small markers.
- **Expansion projects** (SuedLink, SuedOstLink, A-Nord, Ultranet and the other BBPlG/EnLAG projects) drawn on top with
  their status: planned, approved, under construction, in operation. With the time slider they switch from dashed to
  solid at their (expected) completion year.
- **The bottleneck, made visible** (later phase):
  - *Curtailment and redispatch*: wind farms that the grid operators throttled show the gap between what the weather
    allowed and what was fed in, for example as a red top segment on the bar.
  - *North–south imbalance*: generation minus estimated load per control area (50Hertz, TenneT, Amprion, TransnetBW),
    drawn as arrows across the area boundaries.
  - *Cross-border flows* as arrows at the national border.

## Data

| What | Source | Update | Terms | Checked |
|---|---|---|---|---|
| Every generating unit in DE: type, capacity, location, commissioning and decommissioning date | Bundesnetzagentur Marktstammdatenregister, full export (`download.marktstammdatenregister.de/Gesamtdatenexport_YYYYMMDD_*.zip`, zipped XML) | daily | Datenlizenz Deutschland – Namensnennung 2.0 | export of 2026-11-07 listed |
| DE generation by type, 15 min, years of history | Energy-Charts API (Fraunhofer ISE), `api.energy-charts.info/public_power?country=de` | about live | CC BY 4.0 | answers; rate-limits quickly (HTTP 429) |
| DE installed capacity by type | Energy-Charts `installed_power` | monthly/yearly | CC BY 4.0 | answers |
| Weather at plant sites (irradiance, wind at 100 m), now and history | Open-Meteo | hourly | CC BY 4.0 | not yet |
| Per-unit output of large units (≥ 100 MW, e.g. offshore parks) | ENTSO-E Transparency Platform, "Actual generation per generation unit" | hourly, delayed | free token, ENTSO-E terms | not yet |
| Wind/solar per control area, curtailment, redispatch measures | netztransparenz.de (the four TSOs) | 15 min / per measure | to check | page reachable |
| Power lines, voltages, substations | OpenStreetMap (`power=line|cable|minor_line`, `voltage=*`, `power=substation`) | daily extracts (Geofabrik) | ODbL | not yet |
| Grid expansion projects and their status | Bundesnetzagentur, netzausbau.de (quarterly monitoring) | quarterly | to check; may need hand-made data | not yet |

OpenInfraMap's vector tiles have everything for the grid in one layer (`power_line` with voltage, `power_substation`,
`power_plant`), but their TileJSON is marked deprecated and there is no published usage policy. They don't fit
[CACHING.md](CACHING.md)'s rule of using only services whose publisher allows it, so extract from OSM ourselves.

### Where output values come from

Real per-plant output is published only for large units. Everything else is estimated, in three steps of quality:

1. **Capacity share**: `plant output = national output of its type × plant capacity / national capacity of that type`.
   This is trivial and the national total is exact, but every solar park shows the same fill.
2. **Weather-weighted share**: weight each plant by its local weather (irradiance for solar, wind at hub height through
   a generic power curve for wind), then scale so the weights still sum to the measured national total. The total stays
   exact and the pattern on the map follows the weather: a sunny south, a windy coast.
3. **Measured where available**: ENTSO-E per-unit values replace the estimate for large units. The TSOs' per-control-area
   figures constrain each area's total in the same way as the national one.

The difference between "what the weather allowed" and "what was fed in" is where curtailment shows. Only redispatch data
from the TSOs is evidence of curtailment. A gap in the estimate alone isn't, so it must be labelled as such.

## Fit with the program

- **Preprocessing tool** (`tools/gen_energy.py`, like `gen_stars.py`): reads the MaStR export and writes a compact
  binary, e.g. `data/energy_de.bin`: per unit lat, lon, type, capacity in kW, commissioning and decommissioning day.
  MaStR has several million units, most of them rooftop solar. Units owned by private persons are published without
  coordinates (postcode/municipality only), so small units are summed per postcode at its centroid. Expected size: a
  few MB. The program asks nothing of MaStR at runtime.
- **Grid tool** (`tools/gen_grid.py`): Geofabrik Germany extract → `osmium tags-filter` for power features → lines with
  voltage, simplified per zoom band → `data/grid_de.bin`. ODbL share-alike applies to that derived file; record it in
  [PROVENANCE.md](PROVENANCE.md).
- **Grid drawing**: the per-tile overlay path already draws imported lines over the terrain on globe and map
  (`overlay.rs`, `import_draw.rs`). Power lines join that path with a `LineStyle` per voltage, and a layer switch in
  `layers.rs` like trails or trains.
- **Bars**: a new mesh beside the buildings, reusing `BuildingVertex`, face shading and cast shadows from
  `buildings.rs`. Unlike buildings, bar height changes every frame (screen scaling, output over time), so draw them
  instanced: one unit prism mesh, plus per-instance position, footprint, ghost height, fill height and colour, updated
  when the clock or the clustering level changes. Building meshes are baked per tile, which would be too slow here.
- **Live feed**: a worker with a disk cache and age, as in `event_feeds.rs` (`MIN_GAP`, caching, a thread so that no frame
  waits). One request per day of data covers a whole day of scrubbing; past days never change and can be cached for
  good. Energy-Charts rate-limits, so cache aggressively and never ask more than once per 15-minute step.
- **Picking**: a click on a bar opens a panel with name, operator, type, capacity, commissioning date, current output and
  whether that value is measured or estimated, like the existing click info.

## Phases

1. **Plants, static.** MaStR tool, bars sized by capacity, type filters, clustering, build-out over the time window. No
   network at runtime. This alone answers "where is it built, and since when".
2. **Grid.** OSM transmission lines and substations by voltage; expansion projects with status as far as data allows.
3. **Output, national split.** Energy-Charts feed, ghost and fill bars, playback over a day (estimate step 1).
4. **Better estimates and bottleneck.** Weather weighting, ENTSO-E per-unit values, per-TSO totals,
   curtailment/redispatch and north–south arrows.
5. **Other countries.** Global Energy Monitor trackers for locations (CC BY 4.0), ENTSO-E for Europe, OpenElectricity
   for Australia (5-minute per-farm values), EIA for the US.

## Open questions

- Show small rooftop solar summed per postcode, or only units from some size up (say 1 MW)? Summing shows how much of
  Germany's solar is on roofs, but adds most of the data volume.
- Ship the generated files in `data/`, or generate them locally with the tools? Both MaStR and OSM licences allow
  shipping with attribution (ODbL requires share-alike on the derived database).
- Whether the expansion projects have usable published geometry, or need simplified hand-drawn corridors.
