# Plan: energy bars for every country

Germany has bars at every level (states, districts, towns, plants), built from MaStR, BKG and SMARD. Every other
country has one bar at the Countries level. This plan brings the States and Plants levels to the world, with monthly
estimates from the Ember figures already built in. Districts and towns stay German. This file is deleted when the
plan is done.

## M1: world plant list

- Source: Global Energy Monitor, Global Integrated Power Tracker (GIPT), September 2026 release, CC BY 4.0. Its
  download is a form, so the user downloads the file; `tools/gen_world_plants.py PATH` reads it (columns by header
  name, defensively) and writes `data/plants_world.bin`.
- Keep units with status operating, retired (with years), or mothballed; drop announced/pre-construction/cancelled.
  Keep start and retirement year, so the build-out over time works as in Germany.
- Smallest unit kept: 1 MW; solar and wind outside Europe and the US: 10 MW, if size demands it. Target: the web
  build grows by at most a few MB. Germany keeps MaStR and ignores GEM's German units.
- Map GEM types onto `energy::Source`. Each unit belongs to a country (ISO code, as in `countries.rs`).
- The Plants level shows world plants beyond Germany, height from capacity; clicking one shows name, type, capacity,
  years, owner where given. Credit GEM on screen when its plants are in view, and in PROVENANCE.md.
- Tests on a small fixture under `tests/fixtures/gem/`.

## M2: states and provinces

- Source: Natural Earth admin-1 states and provinces, 1:10m, public domain. `tools/gen_world_states.py` writes
  outlines (simplified) with ISO 3166-2 code, name, country, and population, to `data/states_world.bin`.
- Population per state: Natural Earth carries none, so use the sum of Natural Earth populated places, or a
  CC BY 4.0 grid (GHS-POP, Kontur), whichever is small and good enough; say which in PROVENANCE.md.
- The States level shows a bar per state worldwide (Germany's from BKG as now). Until M3, a state's bar is its
  country's consumption shared by population, and its capacity is empty.
- Tests: outlines parse, every state has a country, population sums are near the country's.

## M3: monthly estimates and the gap

- Each world plant's output for a month is its capacity times its country's capacity factor for that source and
  month, from Ember monthly (yearly where Ember has no months), over the capacity GEM lists for that country and
  source, capped at 1.
- A state's bar is the sum of its plants (point in outline), plus consumption by population.
- Generation that no plant accounts for (rooftop solar, small units: Ember's figure above what plants can make) is a
  per-country gap, shown in the country's bar and info card, never spread onto plants.
- The monthly chart and the info cards use these numbers; FEATURES.md, ENERGY.md, help and PROVENANCE.md updated.

## Rules for every milestone

`cargo test` passes, clippy has no new warnings (8 known), `cargo check --target wasm32-unknown-unknown` is clean,
the web build stays working. One commit per milestone. `config/settings.json` is never staged or committed.
