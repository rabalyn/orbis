# Energy layer: design sketch

Phase 1 (plants as bars, build-out over time), fossil plants for comparison, phase 3 (output split nationally, with a day chart of generation against consumption, from SMARD instead of Energy-Charts, whose terms do not allow redistribution) are implemented; [FEATURES.md](FEATURES.md#electricity-of-countries-and-germanys-plants)
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
| States and provinces of the world but Germany (fine units merged into regions): outline, ISO 3166-2 code, name, country | Natural Earth admin-1 states and provinces 1:10m (`github.com/nvkelso/natural-earth-vector`, `geojson/ne_10m_admin_1_states_provinces.geojson`), via `tools/gen_world_states.py` into `data/states_world.bin` | occasional | public domain | built 2026-11-09 |
| Power plants of the world but Germany (operating, mothballed, retired; ≥ 1 MW, solar and wind outside Europe and the US ≥ 10 MW): type, fuel, capacity, start and retirement year, owner, position, gem.wiki page | Global Energy Monitor, Global Integrated Power Tracker, September 2026 release (spreadsheet behind a download form; the user downloads it), via `tools/gen_world_plants.py` into `data/plants_world.bin`; units with TransitionZero IDs (CC BY-NC) left out | GEM's trackers update a few times a year | CC BY 4.0 | built 2026-11-09; 60,028 sites in 198 countries |
| Population of those states | Wikidata, P1082 of each state's item (`query.wikidata.org/sparql`), gaps estimated by area | occasional | CC0 | built 2026-11-09; 619 of 3,299 estimated |
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

Beyond Germany (`world_output.rs`), each plant of Global Energy Monitor's list gets a **monthly** estimate of step 1's
kind, from Ember instead of SMARD: `plant output = plant capacity × capacity factor`, where the factor of a country,
source and month is Ember's generation of that source in the month (the year's average power where Ember has no months)
over the capacity GEM lists for that country and source running in that month, but never above a plausible monthly
ceiling for the plant's kind (`world_output::MAX_FACTOR`): solar 0.30, wind on land 0.55, wind at sea 0.65, hydro 0.90,
pumped storage 0.30, thermal plants and nuclear 0.95. Ember's sources set the groups (wind not split by land and sea,
coal not into lignite and hard coal, hydro with pumped storage, bioenergy with the other renewables); within a group all
plants run at one factor, a kind with a lower ceiling stopping at it. GEM's list misses rooftop solar, small units and
TransitionZero's rows, so Ember's figure beyond what the listed plants can make at their ceilings is a per-country
**gap**, "not in plant list", shown in the country's box and the state's, never spread onto plants; plants and gap add
up to Ember's figure. In June 2025 Japan's solar plants reach the ceiling and 60% of its solar generation is in the gap
(rooftops), the United States' 21%; France's nuclear runs at 55%, with no gap. The ceilings are upper bounds, so where
the list's capacity suffices (India's and China's solar, at about 0.296), missing units can still lift the listed
plants' factor up to the ceiling, and the gap is a floor on what the list misses.

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

## Check against Ember

`python3 -I tools/check_owid.py [--fetch DIR]` sums SMARD's quarter hours into calendar months (German time) per source
and sets them against Ember's monthly figures for Germany in `data/owid_energy.bin`. It reads the weekly files from the
program's disk cache (`~/.cache/orbis/tiles/power`) and fetches the rest into DIR, three requests a second at most. Its
table `MAPPING` of SMARD series to Ember's sources is the program's `power_feed::owid_series`; a test keeps them in step.
SMARD's monthly files (`{filter}_DE_month_{start}.json`), which the program reads for its factors, match the quarter-hour
sums exactly (checked for 2025, all thirteen series).

Ember / SMARD, run on 2026-11-09. Coal is lignite and hard coal, oil is "other conventional", total leaves out pumped
storage (which Ember does not count), demand is set against SMARD's Netzlast; SMARD and Ember in TWh generated:

| Period | Solar | Wind | Hydro | Bio | Other ren. | Coal | Gas | Oil | Total | Demand | SMARD | Ember |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2024 | 1.140 | 0.986 | 1.271 | 1.138 | 4.146 | 0.987 | 1.304 | 1.218 | 1.092 | 1.056 | 427.3 | 466.7 |
| 2025 | 1.185 | 0.983 | 1.271 | 1.133 | 3.767 | 0.985 | 1.296 | 1.239 | 1.099 | 1.049 | 428.0 | 470.6 |
| 2025-01 | 1.567 | 0.987 | 1.268 | 1.138 | 3.950 | 0.977 | 1.206 | 1.281 | 1.085 | 1.023 | 41.6 | 45.1 |
| 2025-06 | 1.136 | 0.982 | 1.407 | 1.124 | 3.967 | 0.988 | 1.696 | 1.211 | 1.124 | 1.094 | 33.6 | 37.8 |
| 2025-11 | 1.511 | 0.980 | 1.171 | 1.156 | 3.312 | 0.978 | 1.164 | 1.214 | 1.081 | 1.029 | 37.8 | 40.8 |
| 2026, Jan–Jun | 1.198 | 0.979 | 1.201 | 1.166 | 3.990 | 0.978 | 1.245 | 1.223 | 1.095 | 1.050 | 226.4 | 247.9 |

What explains the gaps:

- **Own use.** Ember takes Germany's monthly gas and solar from Energy-Charts precisely to capture captive (industrial)
  and distributed (rooftop) generation, and the other fuels from Agora Energiewende; SMARD's public net generation leaves
  out what is used where it is made. Hence solar +19% over the year and far more in winter (+57% in January, when
  SMARD's metered parks are a smaller part), gas +30% (industrial combined heat and power), bioenergy +13%, oil and
  other gases +24%. Wind and coal, almost all fed into the public grid, are within 2%, Ember slightly lower.
- **Other renewables** are about four times SMARD's "Sonstige Erneuerbare" (3.6 against 1.0 TWh in 2025): Ember's
  category is wider (sewage, landfill and mine gas reported apart from biomass), and SMARD's is small, so the ratio
  says little about the total.
- **Hydro** +27%: SMARD's run-of-river and storage series misses small plants. Pumped storage (8 to 11 TWh a year in
  SMARD) is in neither total.
- **Demand** is 5% above the Netzlast: Ember's demand is its generation plus net imports, so it includes own use too.

**Ember's months against its year.** Ember's twelve months of 2025 add up to 470.6 TWh, its year to 500.5 TWh (6.4%
more; 2024: 466.7 against 496.0; 2023: 480.1 against 506.7). The two come from different Ember datasets: the year from
"Yearly Electricity Data Europe", built from Eurostat's annual statistics, which are **gross** generation (including
what power stations use themselves to run, their auxiliary consumption), with gas and solar from the Energy Institute and
wind from IRENA; the months from "Monthly Electricity Data", which Ember reports as **net** generation for the EU. Ember's
methodology says so ("In the EU, we report net generation for monthly data and gross generation for yearly data") and
warns that summing its months does not give its year. The gap is largest for bioenergy (+24%) and coal (+10%), whose
plants use most of their own output, smallest for solar and wind (+2 to 4%). Demand differs by the same 6%. Other
renewables are folded into bioenergy in the yearly series (0.0 TWh there, 3.6 in the months).

The program scales to the **monthly** figures: they are net like SMARD's, so a factor only adds own use rather than
also power stations' auxiliary consumption, which never leaves the plant; they exist per month, so the season shows in
the factors; and Ember's monthly carbon intensity, which the intensity is calibrated to, is on the same basis. The yearly
figures serve only the year at a glance, where Ember's own renewable share for the year is quoted as it stands.

**Carbon intensity.** Each series of SMARD's mix carries a lifecycle emission factor (IPCC AR5 WG3 Annex III medians,
listed in [PROVENANCE.md](PROVENANCE.md)); per month, one factor brings the mix's average to Ember's monthly carbon
intensity for Germany, so the quarter hours follow SMARD's mix and the month's average is Ember's. Ember's own factors
differ (its coal factors come from UNECE and GEM, with lignite higher than hard coal), which the calibration absorbs:
in 2025 it lies between 1.08 and 1.12 (1.08 for November), so the IPCC medians on SMARD's mix come out about 10% below
Ember's figures.

## Phases

1. **Plants, static.** MaStR tool, bars sized by capacity, type filters, clustering, build-out over the time window. No
   network at runtime. This alone answers "where is it built, and since when".
2. **Grid.** OSM transmission lines and substations by voltage; expansion projects with status as far as data allows.
3. **Output, national split.** Energy-Charts feed, ghost and fill bars, playback over a day (estimate step 1).
4. **Better estimates and bottleneck.** Weather weighting, ENTSO-E per-unit values, per-TSO totals,
   curtailment/redispatch and north–south arrows.
5. **Other countries.** Global Energy Monitor's tracker for locations (CC BY 4.0), with monthly output estimated from
   Ember (done); ENTSO-E for Europe, OpenElectricity for Australia (5-minute per-farm values), EIA for the US.

## Open questions

- Show small rooftop solar summed per postcode, or only units from some size up (say 1 MW)? Summing shows how much of
  Germany's solar is on roofs, but adds most of the data volume.
- Ship the generated files in `data/`, or generate them locally with the tools? Both MaStR and OSM licences allow
  shipping with attribution (ODbL requires share-alike on the derived database).
- Whether the expansion projects have usable published geometry, or need simplified hand-drawn corridors.
