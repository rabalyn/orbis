# Interface and map clean-up: plan

Feedback of 2026-11-08 on the energy view of Germany (620 km altitude, tilted), sorted into milestones that can be
built one after another. Each milestone ends in a state that's worth committing on its own.

## Where things stand

- **There is no UI library.** `ui.rs` is a home-made widget toolkit (panels, buttons, sliders, check boxes), and
  `ui_draw.rs` paints it pixel by pixel with the 8×8 bitmap font from the `font8x8` crate, scaled by whole numbers. That
  is where the blocky, "clunky" look comes from. Map labels already use real fonts (cosmic-text with Noto Sans).
- **The text in the top right** is one string built in `main.rs` (`hemisphere_label`, clock, altitude, tiles, frame
  pacing, `energy_status`, network): position, debug counters and energy facts in one block, in the bitmap font.
- **Bar level** (`energy::Level::for_altitude`): states above 900 km, districts from 900 to 170 km, municipalities from
  170 to 30 km, single plants below. At 620 km, Germany fills the screen but already shows about 400 district bars.
- **Bar height** (`energy::Scale::for_view`): the largest bar of the level is 40 % of the window height, whatever the
  camera distance. Heights are in screen terms, not in metres, so from space a bar is hundreds of kilometres tall and,
  tilted, leans across Denmark into Sweden.
- **Place names** come from OpenFreeMap vector tiles (OpenMapTiles schema). `vector.rs` reads `class` and `rank` but
  not `capital`, which the schema has (2 = national capital, 3–4 = state capital). Labels are placed by priority until
  220 are placed, so small towns with a good rank in their tile can beat Berlin.
- **Credits** are plain text at the bottom of the screen; the URLs in them can't be clicked.

## Milestone 1: a readable map (no new UI needed)

**1.1 Place names by zoom, with capitals, for every country**
- Read `capital` in `read_places`; give `Label` a capital kind (none, state, national).
- National capitals: prefix `★` (U+2605; in the bundled Noto Sans Symbols 2 subset, and on the desktop through
  font fallback), highest priority after country names, shown from the zoom where country names appear.
- State capitals: prefix `☆` or a small `◆`, shown with state names.
- Below that, by zoom band rather than by "whatever fits": at country view (roughly above 400 km) only countries,
  states, national and state capitals; other cities join at closer range by rank; towns and villages only close up.
- Lower the label budget from 220 to what each band needs, and find why Berlin was dropped in the screenshot (priority
  order or collision with neighbouring labels), with a test that a capital is never displaced by a town.
- Done when: the screenshot view shows Berlin ★, the 16 state capitals and neighbouring capitals (Prague, Vienna, Warsaw,
  Copenhagen, Amsterdam …), and little else.

**1.2 The bar level follows what is in view**
- New altitude bands, tuned so that one country filling the screen shows states: states above about 300 km, districts
  300–80 km, municipalities 80–15 km, plants below (final numbers from trying them out; hysteresis kept).
- Done when: the screenshot view shows 16 state bars plus the offshore areas.

**1.3 Bar heights in proportion to the ground**
- Height in metres per MW, fixed for each level instead of a share of the window, so a bar doesn't grow as you fly
  away. Scale chosen so the largest bar of a level is a fraction (about a quarter) of the typical width of its areas:
  a state bar tens of kilometres tall, not hundreds.
- The existing height slider stays as a factor on top of that.
- A scale mark in the legend says what height stands for ("this tall = 1 GW").
- Done when: from the screenshot view no bar leans out of Germany, and zooming in within a level keeps bars the same
  size in the landscape.

## Milestone 2: an interface that looks like 2026

Decision needed first: **which way to go.**

- **A. egui (recommended).** `egui` with `egui-wgpu` and `egui-winit` draws into the existing wgpu frame on Linux and
  in the browser (WebGL), and brings proper text rendering, scrolling, text input, keyboard focus, hi-DPI and
  accessibility (AccessKit, so screen readers work). Themed to look like KDE Breeze: Noto Sans (Plasma's default font),
  Breeze dark/light colours, blue accent, rounded corners, follows the system's light or dark setting. All dialogs are
  ported, which is the bulk of the work.
- **B. Restyle the own toolkit.** Keep `ui.rs`, but paint with tiny-skia (anti-aliased, rounded, translucent) and
  cosmic-text instead of the bitmap font. Less churn and no new dependency, but scrolling, text input and accessibility
  stay home-made.

A real Qt/Kirigami look isn't possible from a winit/wgpu program; A gets closest.

Work packages (for A; B has the same steps minus the porting):

- **2.1 Foundation**: egui in the render loop on desktop and web, input routed (egui first, then the globe), credit band
  rule kept.
- **2.2 Theme**: Breeze-like colours, Noto Sans at the system's size (10 pt on Plasma) times the scale factor, dark and
  light, icons from a symbol font instead of drawn pixels.
- **2.3 Port the panels**, one per commit: icons row, compass, help, view options, time window and slider, energy
  display window, power chart, info card, services, search, saved places, imports.
- **2.4 Remove the old painter** and the bitmap font from the interface (it may stay for the screenshot test overlay).

## Milestone 3: information where it belongs

- **3.1 Split the top-right block**:
  - *Where and when*: coordinates, altitude and clock as one quiet line or chip, top left.
  - *Energy*: level, month, scale and the generated/renewable/use figures move into an energy legend card next to the
    power chart.
  - *Technical*: tiles, fps, present mode, downloads, cache: hidden by default, behind a key (F12) and an option.
- **3.2 Level of detail control** in the energy legend: a slider with fixed stops
  `Auto · States · Districts · Municipalities · Plants`. *Auto* follows the altitude bands from 1.2, and the stop that
  is in effect is highlighted, so it's always visible that more detail exists and how to get it. A fixed stop holds that
  level while zooming. Each stop names how many bars it means (16, about 400, about 11,000, all plants).
- **3.3 Clickable credits**: links underlined and with a hand cursor; a click opens the browser (`xdg-open` on the
  desktop, a new tab on the web). Source names without a URL open the matching section of `PROVENANCE.md` online, or
  stay plain.
- **3.4 Docs**: FEATURES.md, README, ENERGY.md and the help window brought up to date; new screenshots.

## Milestone 4: browser polish

- Page title, favicon, loading screen in the theme instead of a blank canvas.
- Fonts sized by `devicePixelRatio`, crisp on hi-DPI screens; touch gestures checked.
- Settings saved in the browser's storage, as on the desktop.

## Not in this plan

- Energy data outside Germany (ENERGY.md phase 5). Capitals in 1.1 are for every country, bars stay German.
- Per-capita or per-area bars. Easy to add as a choice after 1.3 if wanted.
