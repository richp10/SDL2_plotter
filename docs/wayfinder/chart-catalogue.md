# Map: chart catalogue (wayfinder:map)

Charted 2026-11-24 from [the chart catalogue design](../chart-catalogue-design.md).
Tracker: this file (local markdown). Tickets are the sections under "Tickets";
a ticket is claimed by setting its status to `in progress` and closed by
recording its resolution. Blocking is written in each ticket's `blocked by`.

## Destination

The twelve decisions in the design are implemented and committed, one commit
per ticket, with tests at each new seam, the tree green at every step
(rustfmt clean, 0 warnings, clippy 0, `cargo test`, benchmarks where the
renderer changes, release run), and CLAUDE.md describing the result.

## Notes

- The design is settled; tickets are implementation slices, not open
  decisions. Any question a slice raises that the design does not answer
  goes back to the owner as a grilling ticket rather than being decided
  silently.
- Skills: `codebase-design` for seams, `tdd` red-first at each seam,
  `diagnosing-bugs` for regressions.
- Standing preferences: French log text stays French; new identifiers in
  English; the app must work with any combination of files in `map/`.

## Decisions so far

<!-- one line per closed ticket, appended as each is resolved -->

- [C5 — Charts menu as layer switcher](#c5--charts-menu-as-layer-switcher) — one entry per layer with something to show, then Lock chart and Imagery on/off; files leave the menu.
- [C4 — Lock, badge and indicators](#c4--lock-imagery-badge-and-indicators-on-screen) — L locks, I toggles imagery, the bottom bar names layer and file with overscale, fallback, more detail and lock, and a red photo badge (in the stream too) taps back to the chart.
- [C3 — Two-file rendering](#c3--two-file-rendering) — the next best file of the layer is drawn beneath, cut and scaled into the chart's gaps and cached; no cost to warm frames.
- [C2 — The switching rule](#c2--the-switching-rule) — `Catalogue::choose` picks the file per layer, centre and zoom with overscale, more-detail, fallback (never imagery) and zoom limit; the session switches files as you zoom and pan.
- [C1 — Catalogue model and settings](#c1--catalogue-model-and-settingschartsjson) — `Catalogue::load` scans, caches real zooms and coverage, guesses layers and roles from names, keeps user choices in `settings/charts.json`, never fails; the menu shows display names.

## Not yet specified

- Blending imagery over a chart with an opacity slider (deferred by the
  design).
- Whether the catalogue rescan moves to a worker thread or stays at startup,
  depending on measured scan time on a Pi with multi-gigabyte files.

## Out of scope

- Importing, converting or downloading charts (see the chart format
  appraisal); deleting files from the manager.

## Tickets

### C1 — Catalogue model and `settings/charts.json`
- type: task · status: closed · blocked by: none
- Resolution: `src/catalogue.rs`. `Catalogue::load(dir, settings) -> Catalogue` never fails: unreadable or empty `.mbtiles` are skipped with a WARN; a damaged or unknown-version `settings/charts.json` is set aside as `.corrupt` with an ERROR; a missing directory is an empty catalogue. Each `ChartFile` carries file name, path, display name, layer, real zoom levels (from `SELECT DISTINCT zoom_level`, metadata ignored), `Bounds` (metadata `bounds` validated, else the tile extent at the lowest zoom, TMS rows), hidden, order, reviewed. The scan is cached per file keyed by size and mtime in nanoseconds; settings of absent files are kept for when they return. New files: layer from the first known source word (Navionics, OpenSeaMap, NOAA -> Chart; Bing, Google, ArcGIS, Esri -> "<source> satellite", Imagery), else "Imagery" for sat/imagery/aerial/ortho/photo, else the default "Charts" layer; display name from metadata `name` when at least six characters, else the file stem; `reviewed = false`. The MBTiles `type` key is ignored. Startup replaces `state::list_maps` (deleted) with the catalogue: the menu shows visible files by display name in the user's order and the log has one INFO line per file. `files()`/`layers()` are test-only until C2/C6 use them. On the owner's twelve files: three Navionics, one OpenSeaMap, and Bing/Google/ArcGIS as Imagery, as intended; a cached start costs about 6 ms. Nine tests.
- Question: a pure `catalogue` module that scans the chart directory, reads
  real zoom ranges and coverage (cached by size and mtime), guesses layers
  and roles from names, merges the user's saved settings, and persists them
  with the set-aside policy. Covers design decisions 1–4.

### C2 — The switching rule
- type: task · status: closed · blocked by: C1
- Resolution: `Catalogue::choose(Request { layer, lock, centre, zoom }) -> Choice { file, overscale, more_detail, fallback_from, outside_lock, zoom_limit }`, pure. Within the chosen layer, visible files covering the centre (`Bounds::contains`, antimeridian aware) whose levels include the zoom compete on deepest top level, then smaller area, then order; past every top level the deepest is shown overscaled; a file whose first level is deeper is passed over. A lock wins outright and reports `outside_lock`. Nothing in the layer: the best file of the other `Chart` layers, `fallback_from` naming the layer; imagery is never a fallback; nothing at all: `file = None`, the caller shows `charts.default`. `zoom_limit` counts the shown and the chosen layer (a test caught that, counting the shown layer only, the world chart fallback could not be zoomed back out of). Wiring: `Session` owns the catalogue and `charts.default`; `sync_view` pushes the view then shows the chosen file (`change_map` only on change, failures remembered, choice logged at DEBUG); `state.layer` (new, serde default, derived from `state.map` on first start); `ZoomIn` up to `zoom_limit`, `ZoomOut` to 0; `ChangeChart(i)` chooses the file's layer. The renderer lost its zoom policy: `zoom_increase`/`zoom_decrease` and `default_map` are gone, `zoom_to`, `zoom`, `top_zoom`, `path` added, and `change_map` keeps zoom and animation. Lock is in the rule but always `None` until C4; gap fill is C3. Benchmarks unchanged within noise (warm 1600x800 render 3.2-3.4 ms before and after on repeat runs; the drawing path is untouched). 13 new tests (9 rule, 4 session), 180 in all.
- Question: a pure function from (catalogue, active layer, lock, view centre,
  whole zoom) to (active file, gap-fill file, indicators), including the
  fallback to a Chart layer and the world chart. Decisions 5, 6, 8 (rule
  part), 10 (what to indicate).

### C3 — Two-file rendering
- type: task · status: closed · blocked by: C2
- Resolution: `Choice::beneath` is the next best visible file of the shown layer covering the centre whose first level is at or above the zoom (so an overscaled ocean chart sits under a harbour chart, and a locked file outside its coverage has the best local file under it); never across layers, so imagery and charts never mix. `MBTilesRenderer::set_beneath(Option<&str>)` opens it as a second `TileStore`; `render_tiles` draws, for each tile the chart lacks or has transparent, the square of the file beneath cut from its closest level at or above the chart's level and scaled up with nearest-neighbour, cached per tile key in an LRU of the configured size and cleared on `change_map` (which carries the file beneath over until the session resets it). The session sets it after choosing the file; a file beneath that fails to open is remembered with the others. Not tiled through the loader thread: a gap tile's source is decoded on the main thread, as main tiles already are when a frame needs them. Benchmarks: warm 1600x800 render 3.1-3.3 ms (unchanged); new `bench_render_frame_with_a_file_beneath` on the OpenSeaMap test chart's west edge over the world chart, cold 46-48 ms against 34-36 ms without, warm 3.1 ms. Also fixed a test race the new session tests exposed: fixture names from pid plus clock collided between parallel tests on Windows; `test_support::unique_name()` adds a counter. Two new tests (rule, renderer) and the session zoom test extended.
- Question: the renderer draws a gap-fill file beneath the active file,
  each with its own `TileStore`, without blocking the loop; benchmarks
  before and after. Decision 7.

### C4 — Lock, imagery badge and indicators on screen
- type: task · status: closed · blocked by: C2, C3
- Resolution: `state.lock`, `chart_layer`, `imagery_layer` (serde defaults). `AppAction::ToggleLock` (key L) locks the file shown or releases; `AppAction::ToggleImagery` (key I) decides by the chosen layer's role, not what shows (a Bing layer falling back to a chart still counts as imagery chosen), going to the last chart layer (or none: the best chart) or the last imagery layer (or the first one present; none present: logged, nothing changes). Every layer choice goes through `Session::choose_layer`: lock released, layer remembered by role. `Session::chart_status() -> ChartStatus { text, imagery }`: "Layer: file name", then " · OVERSCALE ×2^n", " · No X here, showing Y", " · More detail: name", " · LOCKED" and " · Outside locked chart"; "Overview chart" when the default chart shows. UI text English like the rest of the bar. `Buttombar::show` gained a centre message; `show_photo_badge` draws "PHOTO - not a chart (tap for chart)" top centre in red before the stream capture (confirmed in a frame taken from the stream over São Miguel); a tap inside `photo_badge_area` while imagery shows is `ToggleImagery`. Four session tests. Also: the fixture chart now writes its tiles in one transaction; a zoom-5 fixture took a minute (a commit per tile), and the whole suite drops from about 50 s to 9 s.
- Question: lock persisted in `state.json`, the photo badge (in the stream
  too), and the bottom-bar indicators wired to C2's output. Decisions 8–10.

### C5 — Charts menu as layer switcher
- type: task · status: closed · blocked by: C1
- Resolution: `AppAction::ChangeChart(i)` became `ChooseLayer(i)`, an index into `Catalogue::switchable_layers()` (layers with at least one visible file, catalogue order), which both the menu and the session read, so the two cannot disagree. The Charts menu lists the layers (imagery labelled "(photo)"), then "Lock chart" (`ToggleLock`) and "Imagery on/off" (`ToggleImagery`). With the owner's files the menu is five layer entries plus two, instead of twelve clipped file names. "Chart manager..." is added with the window in C6, so no entry leads nowhere. The menu is built at startup; a layer created or emptied in the manager needs it rebuilt (C6). Tests: the switcher omits a layer whose files are all hidden; the menu's lock and imagery entries; the session tests renamed.
- Question: the menu lists layers, "Chart manager…", "Lock chart" and
  "Imagery on/off" as `AppAction`s on `Session`. Decision 11.

### C6 — Chart manager window
- type: task · status: open · blocked by: C1, C5
- Question: the full-screen touch panel editing names, layers, roles,
  show/hide and order, with the coverage outline. Decision 12.
