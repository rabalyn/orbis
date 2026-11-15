# Testing requirements

These requirements apply to every code and behavior change in Orbis.

1. Every feature and externally observable behavior must have automated tests. Cover the normal path, boundaries, invalid input, persistence, and rendering or interaction effects where applicable.
2. Run the relevant tests after each change. Before calling a task complete, run the full `cargo test` suite and report the commands and results. A passing compile check is not a substitute for tests.
3. Add or update tests in the same change as the behavior. Do not leave a new feature with only a manual test plan.
4. Record any behavior that cannot currently be tested automatically as a blind spot, explain why, and either add the needed test seam or development helper in the same change, or state the remaining limitation clearly. A tooling gap that blocks a required test is work to fix, not a reason to omit the test.
5. Keep the feature specification and its coverage description in sync. Tests should check behavior, not just implementation details.

## Test commands

- `cargo test` runs the full native unit and integration suite.
- `cargo check --target wasm32-unknown-unknown` checks the browser build when that target is installed.
- The `--screenshot` and `--record` modes are available for repeatable rendered-output inspection. Prefer a regression assertion over a screenshot-only/manual check when the behavior can be expressed in code.

## Measuring performance

- `--benchmark SECONDS` turns the view in the window and prints the time between frames, which the frame limit sets, and the work time per frame with its parts (update, panels, labels, drawing), which is the figure to compare. Use `--offline` so that no server is loaded.
- `cargo build --profile profiling --features profile` builds an optimised program with symbols and a sampling profiler. With `ORBIS_FLAMEGRAPH=FILE.svg` set, a run writes a flamegraph of all threads and the folded stacks beside it (`FILE.folded`).
- A change made for speed must leave pictures as they were: compare `--screenshot` images taken with a fixed `--time` before and after.

## Measurement trails coverage

The measurement module tests altitude-aware leg distances, uphill/downhill totals, flat legs, antimeridian distance, track style propagation, visibility filtering, clone/delete behavior, JSON persistence, and GPX export/import round trips. The overlay raster test checks custom color and dotted line rendering. The application sends the same imported overlay documents to globe and atlas rendering; both view paths consume the tile overlay, independent of atlas projection.

Known blind spots: automated tests do not yet drive the live egui window to perform the complete ruler workflow, inspect segment label placement in a rendered globe/map screenshot, or exercise the OS file-drop and export path in an interactive session. Terrain sampling is tested at the mesh interpolation layer elsewhere, but an interactive click over each loaded terrain source is not driven end to end. These remain manual acceptance checks until a window-level test harness is added; the deterministic measurement math, persistence, GPX serialization, visibility, and raster styling have unit coverage.
