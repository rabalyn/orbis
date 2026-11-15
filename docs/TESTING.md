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

## Energy bars built away from the frame

At the Plants level there are hundreds of thousands of bars. Their list is made only when the level, month, sources or the view's place change; the scale of the bars in view and the mesh (with its buffers on the graphics card) are worked out on threads of their own, and a frame takes the result when it is there. `--screenshot` waits for both.

The tests check the grouping of overlapping sites (same result every time, nothing lost, plain and faint kept apart), the scale from the kept heights against the one from the bars, the scale counting only bars in view, the copy of the ground another thread asks, that frames go on while something is still being worked out, the mesh job for plain and grouped bars on the machine's graphics adapter, and that the day chart's legend keeps to its width. `--benchmark` names the slowest frames and their parts; `--fullscreen` and `--energy-detail LEVEL` set up the heavy cases.

Known blind spots: the hand-over between the frame and the two threads inside a running window (a mesh overtaken by a later request, the start view's area being looked for once per set of bars) is not driven by a test; nor is memory use. Check a change here with `--benchmark` at `--energy-detail plants` from far away, watching the process's memory, and compare `--screenshot` pictures before and after: they should not differ, nor from one run to the next.

## Overlays painted by the graphics card

Roads and boundaries are drawn onto each tile's overlay image either by the processor (`tiny-skia`, `overlay::render`) or, in a window on a real graphics card, by the card from triangles (`overlay_gpu`). `--cpu-overlays` and `--gpu-overlays` choose one; `--screenshot` uses the processor unless told otherwise, so that pictures are the same on every machine.

The tests check the triangles without a card (width, round ends and joins, dashes, hairlines, thinning, what lies off the image, groups), that texel centres inside them match what the processor paints, the hand-over from the loader (no card: the processor's picture unchanged; card: groups in drawing order with imports laid over), and the choice between the two ways. One test paints on the machine's graphics adapter and compares the image with the processor's texel by texel.

Known blind spots: that last test passes without comparing anything on a machine with no graphics adapter (it says so on standard error). The mip levels made on the card, the pass split above 255 groups and the look on the globe are not asserted; compare `--screenshot` pictures taken with `--cpu-overlays` and `--gpu-overlays`. The browser build keeps to the processor: painting there has not been tried.

## Measurement trails coverage

The measurement module tests altitude-aware leg distances, uphill/downhill totals, flat legs, antimeridian distance, track style propagation, visibility filtering, clone/delete behavior, JSON persistence, and GPX export/import round trips. The overlay raster test checks custom color and dotted line rendering. The application sends the same imported overlay documents to globe and atlas rendering; both view paths consume the tile overlay, independent of atlas projection.

Known blind spots: automated tests do not yet drive the live egui window to perform the complete ruler workflow, inspect segment label placement in a rendered globe/map screenshot, or exercise the OS file-drop and export path in an interactive session. Terrain sampling is tested at the mesh interpolation layer elsewhere, but an interactive click over each loaded terrain source is not driven end to end. These remain manual acceptance checks until a window-level test harness is added; the deterministic measurement math, persistence, GPX serialization, visibility, and raster styling have unit coverage.
