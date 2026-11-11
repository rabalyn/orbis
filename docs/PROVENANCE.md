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
  and restated in our own words in [FEATURES.md](FEATURES.md)
- public standards and format documentation
- general geodesy and real-time rendering practice

## Published formulae in use

Eclipses are computed, with no data source: the VSOP87 series for the Earth (Bretagnon and Francou) and the ELP-2000/82 series for the Moon (Chapront-Touzé and Chapront), as printed in truncated form in Meeus, "Astronomical Algorithms", 2nd ed. (appendix III and chapters 22, 25 and 47); ΔT from the polynomials of Espenak and Meeus (NASA/TP-2006-214141); the shadow geometry from the Besselian elements of the Explanatory Supplement to the Astronomical Almanac. The test values (times, magnitudes, gamma and the central line of 2024-04-08) are facts from the NASA/GSFC eclipse bulletins of Fred Espenak (eclipse.gsfc.nasa.gov); their terms were not checked and no table from them is bundled.

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

Finer Moon layers from the same service and terms, each found as a Trek WMTS layer and laid over
the WAC mosaic where it has a picture (credit: `Moon: LROC WAC and NAC mosaics, NASA/GSFC/Arizona
State University; Kaguya TC mosaic, JAXA/SELENE; via NASA Moon Trek https://trek.nasa.gov/moon/`):

- `Kaguya_TCortho_Mosaic_Global_4096ppd`, SELENE (Kaguya) Terrain Camera ortho mosaic, PNG with
  transparency, used at zoom 9 and 10. JAXA data served by NASA; JAXA's own terms for SELENE data
  (free use with credit "JAXA/SELENE") were not read for this, so recheck before publishing.
- LRO Narrow Angle Camera mosaics of the Apollo sites: `LRO_NAC_Apollo11_Mosaic_p`,
  `LRO_NAC_Apollo12_Mosaic_p`, `LRO_NAC_Apollo14_Mosaic_p`, `LRO_NAC_Mosaic_26N004E_50cmp`
  (Apollo 15), `LRO_NAC_Mosaic_09S015E_50cmp` (Apollo 16) and `NAC_DTM_APOLLO17_MOSAIC_120CM`,
  used from zoom 11 inside each mosaic's area.

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

| Mars, finer global imagery | The CTX global mosaic (Murray Lab, Caltech; Dickson et al.) as Esri's tile service `https://astro.arcgis.com/arcgis/rest/services/OnMars/CTX/MapServer/tile/{z}/{y}/{x}`: 512-pixel tiles, plate carrée, levels 0 to 12, used from 8. Its service description names the source; no usage terms beyond the Murray Lab's request for credit were found, so recheck before publishing | `CTX mosaic, NASA/JPL/MSSS/The Murray Lab, via Esri` |
| Mars, rover sites | Mars Trek layers `curiosity_hirise_mosaic`, `opportunity_hirise_mosaic`, `spirit_hirise_mosaic` and `sojourner_hirise_mosaic`, PNG with transparency, used from zoom 13 inside each mosaic's area | `HiRISE, NASA/JPL/University of Arizona` |
| Ceres and Vesta, finer | Ceres Trek `CE_LAMO_G_00N_180E_EQU_CLR_scale.eq` (zoom 6 and 7) and Vesta Trek `VestaHAMOLAMOBlend.eq` (zoom 7) | added to each body's credit as "LAMO" |

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

### Names of features on other bodies (`data/names.tsv`)

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Names, positions, sizes and kinds of features on the Moon, Mars, Mercury, Ceres and Vesta | USGS Gazetteer of Planetary Nomenclature, the center-point shapefiles of its [GIS downloads](https://planetarynames.wr.usgs.gov/GIS_Downloads) (only the `.dbf` tables are read), names with the status "Adopted by IAU", downloaded 2026-11-09 | A work of the US Geological Survey, so in the US public domain; the names themselves are the IAU's. Credit is given anyway | `Names: IAU / USGS Gazetteer of Planetary Nomenclature`, in the search panel and in the box listing features |

`tools/gen_names.py` turns the tables into `data/names.tsv`: body, name, planetocentric latitude, east longitude from
-180 to 180, diameter in km, and R for wide regions or F for other features. Albedo features without a size are given
800 km, other features without one 1 km.

### Mission tracks (`data/missions.bin`)

| Track | Source | Terms |
|---|---|---|
| Perseverance and Ingenuity | NASA/JPL's MMGIS map of the Mars 2020 mission, `https://mars.nasa.gov/mmgis-maps/M20/Layers/json/M20_traverse.json` and `.../m20_heli_flight_path.json` (GeoJSON with spacecraft clock times), downloaded 2026-11-09 | NASA content, NASA media usage guidelines as above |
| Curiosity | The same for MSL: `https://mars.nasa.gov/mmgis-maps/MSL/Layers/json/MSL_traverse.json` and `MSL_waypoints.json` (drives by sol) | As above |
| Spirit and Opportunity | PDS Geosciences Node, MER Analyst's Notebook traverse tables, `https://an.rsl.wustl.edu/mera/AN/meri/MER/traverse/` (`MERA_traverse.csv`, `MERB_traverse_5130.csv`) | PDS archive data, NASA, public |
| Apollo 12 and 14 moonwalks | LROC Reduced Data Record extras, `https://pds.lroc.im-ldi.com/data/LRO-L-LROC-5-RDR-V1.0/LROLRC_2001/EXTRAS/SHAPEFILE/APOLLO_12/` and `APOLLO_14/` (paths and points with ground elapsed time) | PDS archive data, NASA/GSFC/Arizona State University |

No in-app credit line is shown for the tracks; they are named in the box when a mission is clicked, whose article is
credited there. Add one before publishing if wanted. `tools/gen_missions.py` documents the conversion: times from the
spacecraft clock (epoch 2000-01-01T11:58:55.816 UTC) for Mars 2020, from sols with Mars24's Mars Sol Date and an
assumed timetable for Curiosity, Spirit and Opportunity, and from the launch times of Apollo 12 (1969-11-14 16:22:00
UTC) and Apollo 14 (1971-01-31 21:03:02 UTC) plus ground elapsed time. Positions are planetocentric.

The 30 landing sites without a track (`SITES` in `src/missions.rs`) are published coordinates and landing times as
known to the author, not read from one source for this; recheck before relying on them. They are facts, not a data
file. Each mission's English Wikipedia title is kept with it for the click.

### Place search

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Place names to positions | OpenStreetMap's [Nominatim](https://nominatim.org) at `https://nominatim.openstreetmap.org/search` (`format=jsonv2`, `accept-language` from `--lang`) | Read from the [Nominatim Usage Policy](https://operations.osmfoundation.org/policies/nominatim/): at most one request per second; a valid User-Agent identifying the application (the program sends `orbis-globe/0.1`, which names the program but carries no contact address); attribution; no autocomplete, so a request goes out only when the user presses Enter; repeated queries are cached; the service must be switchable without a new release (`--geocoder-url`); data under the ODbL. Requests are one per 1.1 s at most, one in flight, and each answer is kept on disk | `Search: Nominatim, © OpenStreetMap contributors`, shown in the search panel and in the credits while results are in use, also in saved images |

The policy also says the public API may be used only where the application developer is responsible for it
and that no-code generators may not offer it; this program is a conventional application with user-triggered
searches. The policy may change without notice and the service may be withdrawn: recheck before publishing,
and consider a contact address in the user agent. Typed coordinates are read locally and never sent.
The JSON answers are read with `serde_json` (MIT or Apache-2.0).

### Information on click (Wikipedia and Wikidata)

| Data | Source | Terms | Credit shown in the app |
|---|---|---|---|
| Article text, titles, descriptions, coordinates, other languages, infobox | Wikipedia's MediaWiki Action API, `https://{lang}.wikipedia.org/w/api.php` (generator `search` and `geosearch`, `titles` for missions and other languages, props `coordinates`, `pageprops`, `description`, `info`, `extracts`, `langlinks`; and `action=parse` with `prop=text&section=0` for the first section's HTML, from which only the infobox table is read; JSON) | Text is CC BY-SA 4.0 (and GFDL), per the [Wikimedia Terms of Use](https://foundation.wikimedia.org/wiki/Policy:Terms_of_Use): reuse needs a link or URL to the article, or a list of authors, and the licence named; changes must be indicated. The text is shown as the API gives it, only with pronunciation guides dropped and characters the built-in font lacks simplified or replaced | `Text: Wikipedia, "TITLE" (xx.wikipedia.org), CC BY-SA 4.0.` in the box while it shows the article or its infobox, with the title linking to the page (and so its history) and the licence to its text; the "Open article in browser" button leads there too |
| Facts (type, country, population, elevation, area, website, time zone) | Wikidata's query service, `https://query.wikidata.org/sparql`, one SPARQL query per article by item number | Structured data in the main namespaces is CC0 ([Wikidata:Licensing](https://www.wikidata.org/wiki/Wikidata:Licensing)); the page does not say whether credit is expected, so it is given anyway. The [query service manual](https://www.mediawiki.org/wiki/Wikidata_Query_Service/User_Manual) limits each client to 60 s of query time per minute and 5 parallel queries, answers HTTP 429 with Retry-After when exceeded and may ban clients that go on | `Facts: Wikidata Qnnn, CC0.` in the box while facts are shown, the item linking to its Wikidata page |

Etiquette read from [API:Etiquette](https://www.mediawiki.org/wiki/API:Etiquette): requests are serial, at least 0.3 s apart and at most six per click on Earth (up to four searches and two more for a feature on another body), only
after a click (nothing at start or on hover), batched through generators, with `maxlag=5`, in JSON, and answers are cached on disk for a week (CACHING.md). A
`maxlag`, `ratelimited` or HTTP 429 answer pauses the host through the usual back-off. Only text is used, no images, whose licences vary per file.

The [Wikimedia User-Agent policy](https://foundation.wikimedia.org/wiki/Policy:Wikimedia_Foundation_User-Agent_Policy) asks for a descriptive agent in the form
`name/version (contact information)`, and says generic agents may be blocked without notice. **Unresolved gap:** the program's agent is `orbis-globe/0.1` and has no
contact address, because no personal address or name may be put in it and the project has no generic contact yet. It is distinctive but does not meet the policy; a
project contact (a page or a role address) must be added before publishing. Requests worked from this agent on 2026-11-07, but the query service answered HTTP 429
after about ten queries within a few minutes of testing. Wikipedia's licence page and the policies above could be read; nothing about requests per second for the
Action API beyond "serial, be considerate" was found.

Saved real answers used as test data are in `tests/fixtures/wiki/`: short extracts of Wikipedia text (CC BY-SA 4.0, from the articles Vienna, Tokyo Metropolis and
Cairo) and facts from Wikidata (CC0), recorded on 2026-11-07, kept with the source's licence. `parse_tycho_en.json` and
`parse_perseverance_en.json` are the first sections of the articles "Tycho (lunar crater)" and "Perseverance (rover)" as the
parse API gave them on 2026-11-09 (CC BY-SA 4.0), kept for the infobox tests.

### Map service directory (`data/services.json`)

The directory was built independently from each publisher's own documents, and not from any other program's list.
Every entry has its terms and the page they were read from in its `terms` field. One tile of each was fetched once, on
2026-11-07, with the program's user agent, to prove the address works (all answered 200 with a 256 pixel picture).
The addresses were found by adding each service with `--add-service` from its capabilities document, except OpenTopoMap, whose tile address
is its published one (`tile.opentopomap.org`) and was confirmed by the fetch. Users can add any other service themselves.

| Entry | Source and terms read | Credit shown |
|---|---|---|
| OpenTopoMap (XYZ, zoom 0-17) | opentopomap.org/about: free use including commercial with attribution, CC-BY-SA; embedding in applications welcome as long as the server is not overloaded (no mass downloads); no limits stated. They ask for a short notice of use, which has not been sent. | `Kartendaten: © OpenStreetMap-Mitwirkende, SRTM | Kartendarstellung: © OpenTopoMap (CC-BY-SA)` (the wording the page gives) |
| TopPlusOpen (WMTS, Germany and surroundings) | BKG's TopPlusOpen page and its capabilities (`Fees`, `AccessConstraints`): free of charge for anyone under Datenlizenz Deutschland Namensnennung 2.0 (dl-de/by-2-0), no access restrictions, source credit required | `Kartendarstellung: © BKG dl-de/by-2-0, Datenquellen: ...` (the form the capabilities give, without the year) |
| BRT Achtergrondkaart (WMTS, Netherlands) | PDOK's capabilities: Fees none, AccessConstraints none; PDOK page: "gratis te gebruiken", openly available through WMTS. The licence is not named in what was read, and no attribution is stated | `Kaartgegevens: © Kadaster, BRT Achtergrondkaart via PDOK` (our wording, kept as a courtesy). Recheck the licence before publishing |
| basemap.at map and labels overlay (WMTS, Austria) | CC BY 4.0, as for the aerial layer (CACHING.md); capabilities: Fees none | `Grundkarte: basemap.at` |
| NASA GIBS Blue Marble (WMTS and WMS) and MODIS Terra true colour (WMTS) | GIBS documentation (nasa-gibs.github.io): NASA promotes full and open sharing of data; no limits are stated; users of GIBS in clients are asked to include an acknowledgement beginning "We acknowledge the use of imagery provided by services from NASA's Global Imagery Browse Services (GIBS)". NASA's data policy page returned 403, so the policy itself was not read | `Imagery: NASA GIBS, part of NASA ESDIS (https://earthdata.nasa.gov/gibs)`, a short form of that acknowledgement |

Considered and left out:

| Source | Reason |
|---|---|
| OpenStreetMap standard tiles (`tile.openstreetmap.org`) | The usage policy allows apps with a distinct user agent, attribution and view-only requests, but says the address should not be hard-coded so it can change without a software update, and forbids prefetching and offline use. A built-in directory entry would hard-code it. Users can add it themselves |
| swisstopo (wmts.geo.admin.ch) as a map | The terms pages could not be loaded (404 on the addresses tried), so the permission could not be confirmed for this purpose |
| USGS National Map basemaps | USGS says its own products are public domain, but the maps mix in material from others and the service's own terms page was not found |
| OpenSeaMap seamark tiles | No usage policy could be found (the wiki page returned 404) |
| Esri, Mapbox, Google, Bing, Stadia and similar commercial tiles | Not evaluated: commercial services that generally need keys or accounts, which Orbis does not handle |

The credit of an added service is shown while its tiles are on screen, also in saved images.

### Event feeds (earthquakes, eruptions, tsunamis, fireballs)
Evaluated from each publisher's own pages on 2026-11-07, with one real request to each address to see the format. Nothing is
bundled; answers are cached on disk (10 minutes for the live earthquake feed, a day for the others) and fetched only when the
user opens the events panel (key E) or gives `--event-feed`.

| Feed | Address and format | Terms read | Credit shown |
|---|---|---|---|
| USGS earthquakes | `earthquake.usgs.gov/earthquakes/feed/v1.0/summary/4.5_month.geojson` for dates in the last 20 days, and the FDSN query `fdsnws/event/1/query?format=geojson&minmagnitude=5...` for other dates (GeoJSON, updated every minute) | The feed page states no rate limit or attribution rule (it points to a lifecycle policy page that returned 404). usgs.gov's copyright page: USGS-produced data are in the U.S. public domain, and credit "U.S. Geological Survey" is asked for | `Earthquakes: U.S. Geological Survey` |
| NOAA NCEI eruptions | Hazard Service API `www.ngdc.noaa.gov/hazel/hazard-service/api/v1/volcanoes?minYear=..&maxYear=..` (JSON, `items`, the Significant Volcanic Eruptions database) | No API page with limits or terms could be loaded. The NCEI record of a sibling dataset says access is unrestricted, free, with a no-liability disclaimer and a request to cite the DOI; no licence is named. The volcano database's own DOI was not confirmed, so none is cited | `Eruptions: NOAA NCEI, Global Significant Volcanic Eruptions Database` |
| NOAA NCEI tsunamis | `.../api/v1/tsunamis/events?minYear=..&maxYear=..` (JSON, `items`) | As above; the tsunami database page (`ngdc.noaa.gov/hazard/tsu_db.shtml`) asks to cite doi:10.7289/V5PN93H7 | `Tsunamis: NOAA NCEI/WDS Global Historical Tsunami Database (doi:10.7289/V5PN93H7)` |
| NASA/JPL CNEOS fireballs | `ssd-api.jpl.nasa.gov/fireball.api?req-loc=true&date-min=..&date-max=..&limit=300` (JSON, `fields` and `data`) | The API page (ssd-api.jpl.nasa.gov/doc/fireball.html) states no usage limit or terms; its signature names the source as "NASA/JPL Fireball Data API" | `Fireballs: NASA/JPL Center for Near Earth Object Studies (CNEOS)` |

Requests are one per feed, at least 1.2 s apart, with the program's user agent and the usual pauses. Because no limits are
published for the NOAA and NASA services, the program stays well below any plausible one. The eruptions database is of significant
eruptions only (a dozen or so a year), and tsunami events come year by year, so the panel keeps those within 90 days of the clock.

### Renewable energy in Germany (`data/energy_de.bin`)
Built offline by `tools/gen_energy.py` and bundled; the program requests nothing for it.

| Data | Source | Terms | Credit shown |
|---|---|---|---|
| Generating units: source, net rated power, commissioning and final shutdown dates, municipality key, position | Marktstammdatenregister of the Bundesnetzagentur, full export `download.marktstammdatenregister.de/Gesamtdatenexport_20261107_26.2.zip` (zipped UTF-16 XML), files `EinheitenSolar_*`, `EinheitenWind`, `EinheitenWasser`, `EinheitenBiomasse`, `EinheitenVerbrennung`, `EinheitenKernkraft` | Datenlizenz Deutschland – Namensnennung – Version 2.0, as the register's download page states (read 2026-11-08) | `Plants: Marktstammdatenregister, Bundesnetzagentur, dl-de/by-2-0 (summed)` |
| States, districts and municipalities (as of 1 January 2026) | BKG VG250, `daten.gdz.bkg.bund.de/produkte/vg/vg250_ebenen_0101/aktuell/vg250_01-01.utm32s.shape.ebenen.zip` | Datenlizenz Deutschland – Namensnennung 2.0 (`dokumentation/nutzungsbedingungen_vg250.pdf` in the archive, read 2026-11-08). It requires a clearly visible source note, `© BKG (year of retrieval) dl-de/by-2-0, Datenquellen: https://sgx.geodatenzentrum.de/web_public/gdz/datenquellen/datenquellen_vg_nuts.pdf`, and a note of changes for edited data | `Areas and population: © BKG (2026) dl-de/by-2-0 (simplified, summed), Datenquellen: https://sgx.geodatenzentrum.de/web_public/gdz/datenquellen/datenquellen_vg_nuts.pdf` |
| Population of municipalities at 31 December 2025 (`EWZ` of `VG250_GEM`), for `data/population_de.bin` | BKG VG250-EW, `daten.gdz.bkg.bund.de/produkte/vg/vg250-ew_ebenen_1231/aktuell/vg250-ew_12-31.utm32s.shape.ebenen.zip` (retrieved 2026-11-08) | Same terms as VG250: the archive's `dokumentation/nutzungsbedingungen_vg250.pdf` covers "VG250 und VG250-EW" (read 2026-11-08) | as above |

The tool reads the shapefiles, projects them from ETRS89 / UTM 32N with the Krüger series and simplifies them with
Douglas–Peucker, using only Python's standard library. `tools/gen_population.py` sums the population over the same areas. Offshore park outlines are convex hulls round each park's turbines, made
by the tool. Outlines are simplified, units are summed, and positions are rounded to 1e-4 degrees; the data is otherwise
unchanged.

### Generation and consumption in Germany (SMARD)
Evaluated on 2026-11-08 from the publisher's page `smard.de/home/datennutzung`, with real requests to see the format. Nothing is
bundled; answers are cached on disk and fetched only while the energy bars show output.

| Data | Address and format | Terms read | Credit shown |
|---|---|---|---|
| Public net generation by source and total consumption (Netzlast), Germany, 15 minutes | `www.smard.de/app/chart_data/{filter}/DE/index_quarterhour.json` lists the weekly files; `.../{filter}/DE/{filter}_DE_quarterhour_{start}.json` holds `series` of [milliseconds, MWh per quarter hour or null]. Filters 4068, 4067, 1225, 1226, 4066, 1223, 4069, 4071, 1227, 1224, 1228, 4070, 410 | Data under § 111d EnWG for free use; market data under CC BY 4.0, attribution "Bundesnetzagentur \| SMARD.de". SMARD takes the data from ENTSO-E | `Generation and consumption: Bundesnetzagentur \| SMARD.de (CC BY 4.0)` |

For the factors to Ember's months the program also reads SMARD's monthly files, one per series and year:
`.../{filter}/DE/{filter}_DE_month_{start}.json`, `series` of [milliseconds of the month's start, MWh in the month], same
terms and credit. A year that ended two months ago is kept for good, a later one for a day.

Energy-Charts is not used by the program today. Its older site publishing notes restrict reuse of the site's publications,
but the current API documentation states that API data is CC BY 4.0 and requires attribution to Energy-Charts.info.
The API also documents public rate limits and asks clients to cache responses and honour HTTP 429 `Retry-After` values.
Before adding an endpoint, check that endpoint's current API documentation and cache it; do not treat the site's graphic
and publication terms as the API data license.

Potential additions include cross-border trading and physical flows, day-ahead prices, forecasts, installed capacity,
and frequency. Germany's public generation overlaps with SMARD and should not be added as a second live feed without a
specific need.

### Electricity of the world's countries (`data/owid_energy.bin`, `data/countries.bin`)
Built offline by `tools/gen_owid.py` (retrieved 2026-11-09) and bundled; the program requests nothing for it. Germany's
monthly figures scale the output to include own use and calibrate the carbon intensity, and its yearly renewable share is
the energy card's year at a glance; the other countries' figures make the countries level of the energy layer (one bar
per country, its box) and the flat map's shade by renewable share (`world_energy.rs`). `owid::CREDIT` shows with all of
them. The Data tab of the information box shows a country's population and yearly electricity figures from them
(`country_facts.rs`), with links to Our World in Data and CC BY 4.0.

| Data | Source | Terms | Credit shown |
|---|---|---|---|
| Per country and year: population, electricity generation, demand, net imports, carbon intensity, renewable share, generation by source (coal, gas, oil, nuclear, hydro, wind, solar, bioenergy, other renewables) | Our World in Data's energy dataset, `catalog.ourworldindata.org/energy/owid_energy/owid_energy.csv`; the sources of each column are named in its codebook, `github.com/owid/energy-data/blob/master/owid-energy-codebook.csv` (read 2026-11-09) | OWID's own work is CC BY 4.0; third-party data keeps its terms. The electricity columns kept are Ember's (Yearly Electricity Data and Yearly Electricity Data Europe, 2026, CC BY 4.0 as OWID's indicator metadata states) and, for years before Ember's, the Energy Institute's Statistical Review of World Energy (2025), which OWID's metadata gives as "© Energy Institute 2026" with its terms at `energyinst.org/terms` (not read). Population is OWID's "Population based on various sources (2024)", mainly the UN's World Population Prospects. The EIA figures OWID names are in its primary-energy columns, which are not kept | `Electricity of countries: Our World in Data (CC BY 4.0), from Ember (CC BY 4.0) and the Energy Institute Statistical Review of World Energy` (`owid::CREDIT`) |
| Per country and month: generation in total and by the same sources, demand, net imports, carbon intensity | OWID's chart API, `ourworldindata.org/grapher/electricity-mix.csv?frequency=monthly&metric=<generation, demand, net_imports or carbon_intensity>&source=<total or a source>&useColumnShortNames=true`, from Ember's Monthly Electricity Data | Ember, CC BY 4.0 (from `electricity-mix.metadata.json` and the indicator metadata it links) | as above |
| Country outlines, label points, English names and Wikidata items | Natural Earth admin-0 countries 1:50m, version 5.2, `github.com/nvkelso/natural-earth-vector`, `geojson/ne_50m_admin_0_countries.geojson` | Public domain (naturalearthdata.com, from general knowledge) | none required |

Countries are keyed by ISO 3166 alpha-3. OWID's regions and aggregates are left out apart from the World (`OWID_WRL`);
Kosovo, which has no ISO code, is `OWID_KOS` in both files. Natural Earth gives France and Norway the code -99 in
`ISO_A3`, so `ISO_A3_EH` is used; territories that carry their country's code there (Australia's Indian Ocean
territories) join it, and areas without any code (Somaliland, Northern Cyprus, Siachen) are left out. Years before 1900
are dropped, values are rounded to 1 GWh (0.01 g/kWh, 0.01 %), and outlines are simplified by 0.02° and rounded to
1e-4°, with islands under 0.002 square degrees dropped (never a country's largest ring); the data is otherwise unchanged. The first year with Ember's
demand marks where OWID's electricity figures switch from the Energy Institute to Ember, which is how the reader names
each figure's source.

### States and provinces of the world (`data/states_world.bin`)
Built offline by `tools/gen_world_states.py` (retrieved 2026-11-09) and bundled; the program requests nothing for it.
It gives the States level of the energy layer a bar per state beyond Germany, whose states stay BKG's
(`world_states.rs`). `world_states::CREDIT`, `Country and state outlines: Natural Earth (public domain); population of
states: Wikidata (CC0)`, shows at the Countries and States levels and with the map's shade.

| Data | Source | Terms | Credit shown |
|---|---|---|---|
| Outlines, label points, English names, kinds and ISO 3166-2 codes of 3,299 states, provinces and regions, merged from 4,561 admin-1 units (Germany's 16 left out) | Natural Earth admin-1 states and provinces 1:10m, `github.com/nvkelso/natural-earth-vector`, `geojson/ne_10m_admin_1_states_provinces.geojson` (master, 2026-11-09); the country of each from `geojson/ne_50m_admin_0_countries.geojson` as for `countries.bin` | Public domain (naturalearthdata.com, from general knowledge) | `world_states::CREDIT` |
| Population per state | Wikidata, property P1082 of the item Natural Earth gives each state (`wikidataid`), through `query.wikidata.org/sparql`: the preferred figure, else the latest by point in time | CC0 (Wikidata's structured data) | `world_states::CREDIT` |
| Population of merged regions | Wikidata, P1082 of the item (P131) a third or more of the region's units lie in, if it is at least their sum and under 60% of the country; else their sum | CC0 | `world_states::CREDIT` |
| Population of units Wikidata has no usable figure for (731 of 4,561; 619 of the 3,299 states end up marked estimated) | estimated by area from Natural Earth's `POP_EST` of the country, less the states with a figure | Public domain | as above |

Choice of population source: Natural Earth's populated places (public domain, 0.6 MB) count cities only, so sums per
state miss the countryside (Bihar or rural Africa against their capitals); GHS-POP and Kontur (CC BY 4.0) are grids of
hundreds of MB to GB that need GIS libraries to sum. Wikidata's census figures per state are a few hundred kB and CC0.
Where several Natural Earth states share one item, its figure is split by area; a figure above the country's whole
population is dropped as a wrong item. In 14 countries with more than 40 units (France, Italy, Spain, the United
Kingdom, the Philippines and others; not the US, Russia, Japan or Vietnam) units are merged by Natural Earth's `region`
field (England's regions and the other three nations for the UK), outlines unioned with shapely before simplifying, so
that the level compares with Germany's states. Outlines are simplified by 0.025° and rounded to 1/250°, with islands under 0.005
square degrees dropped (never a state's largest ring); the data is otherwise unchanged.

### Power plants of the world (`data/plants_world.bin`)
Built offline by `tools/gen_world_plants.py` from Global Energy Monitor's spreadsheet, which the user downloaded through
GEM's form (`Global Integrated Power September 2026 V2.xlsx`, kept in the gitignored `data-raw/gem/`), and bundled; the
program requests nothing for it. It gives the Plants level of the energy layer the plants beyond Germany and the world's
states their installed capacity (`world_plants.rs`).

| Data | Source | Terms | Credit shown |
|---|---|---|---|
| Power units outside Germany: type, technology, fuel, capacity, status, start and retirement year, owner, position, GEM location ID and gem.wiki page; 60,028 sites of 80,471 units in 198 countries | Global Energy Monitor, Global Integrated Power Tracker (GIPT), September 2026 release, sheets "Power facilities" and "Regions, area, and countries" (for ISO 3166 alpha-3 codes) | Creative Commons Attribution 4.0 International, as the file's About sheet states ("Copyright © Global Energy Monitor. Global Integrated Power Tracker, September 2026 release. Distributed under a Creative Commons Attribution 4.0 International License"); licence page accepted at download: <https://globalenergymonitor.org/creative-commons-license> | `world_plants::CREDIT`: `Plants outside Germany: Global Energy Monitor, Global Integrated Power Tracker, Sept. 2026 (CC BY 4.0), modified`, while world plants stand as bars and at the States level (their estimated output makes the states' bars); each plant's box names the release and links its gem.wiki page |

Recommended citation (from the About sheet): "Global Energy Monitor, Global Integrated Power Tracker, September 2026
release." GEM does not endorse Orbis; the credit names the source only.

Changes made (as CC BY 4.0 asks them to be stated): units are **filtered** by status (operating, mothballed and retired
kept; announced, pre-construction, construction, shelved and cancelled dropped), by size (at least 1 MW; solar and wind
outside Europe and the United States at least 10 MW), by date (retired units without a retirement year, and retired ones
without a start year, dropped) and by country (Germany's units dropped, as the Marktstammdatenregister covers them);
units of one source at one GEM location are **merged into sites** (name, position and gem.wiki page of the first unit,
the owner and technology of most of the capacity, owners' shares dropped); sources are **mapped** onto the program's
(geothermal drawn as biomass, oil/gas by its first fuel); start and retirement years become months; positions are rounded
to 1e-4°; GEM's overseas French departments and Bonaire count to France and the Netherlands; and the data is **converted**
to the program's binary format. Nationally aggregated distributed (<1 MW) solar, which GEM keeps per country, would be
stored per country rather than as plants; this release's spreadsheet has none (the About sheet says it is in the solar
tracker's summary tables), so the table is empty.

Left out for its licence: units whose "Other IDs (unit/phase)" carry a `TZ:` ID (18,730 rows), which the About sheet
says were partly or wholly sourced from TransitionZero's Solar Asset Mapper (August 2025) under CC BY-NC 4.0. Without
them, everything bundled is under CC BY 4.0.

The test fixture `tests/fixtures/gem/` (`plants.csv`, `regions.csv` and the `plants_world.bin` made from them) is
hand-made with invented plants in GEM's column layout; it holds no GEM data.

### Emission factors (`power_feed::EMISSIONS`)
Lifecycle emissions in g CO₂-eq per kWh, the medians of IPCC AR5 Working Group III, Annex III "Technology-specific cost
and performance parameters", table A.III.2 (Schlömer et al., 2014), as Ember also quotes them for its renewable factors
in its methodology (`files.ember-energy.org/public-downloads/ember_electricity_data_methodology.pdf`, read 2026-11-09).
Figures from a published report, used as numbers; no text or tables are copied.

| SMARD series | Factor | AR5 technology |
|---|---|---|
| Photovoltaik | 48 | Solar PV, utility (rooftop 41) |
| Wind an Land | 11 | Wind onshore |
| Wind auf See | 12 | Wind offshore |
| Wasserkraft | 24 | Hydropower |
| Biomasse | 230 | Biomass, dedicated |
| Braunkohle, Steinkohle | 820 | Coal, pulverised (AR5 gives no figure for lignite of its own) |
| Erdgas | 490 | Gas, combined cycle |
| Sonstige Konventionelle | 820 | none in AR5 for oil, waste or works gases: coal's figure stands in |
| Kernenergie | 12 | Nuclear |
| Sonstige Erneuerbare | 38 | Geothermal |
| Pumpspeicher | 0 | stored energy, counted when it was generated |

The intensity of the mix is calibrated each month to Ember's monthly carbon intensity for Germany (above), so these
factors set only the shape over the day.
