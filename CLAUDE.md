# CLAUDE.md

Guidance for AI coding agents (and humans) working in this repository.

## What this is

**SDL2 Plotter** is a marine chart plotter in Rust: a single binary that renders local MBTiles raster charts with SDL2, follows the boat using live data from a Signal K server over WebSocket, shows AIS targets, and lets the navigator keep routes, waypoints, a track and distance measurements. It also serves the screen as an MJPEG stream so a tablet in the cockpit can watch. Deployment target: a Raspberry Pi running a minimal Linux without a desktop, at the chart table, 24 hours a day for weeks. Windows is a development platform only.

- **Build, run, configure:** [`README.md`](README.md). Windows specifics and verified build numbers: [`WINDOWS_BUILD.md`](WINDOWS_BUILD.md).
- **Known debt:** identifiers and comments mix French and English (see Conventions); settling on one language is deliberately left for later. Deepening work in progress is tracked on the wayfinder map [`docs/wayfinder/architecture-deepening.md`](docs/wayfinder/architecture-deepening.md), charted from [`docs/architecture-review-2026-11-23.html`](docs/architecture-review-2026-11-23.html).

> **It is a safety instrument.** The person at the chart table may be tired, wet and relying on this screen in the dark. Every design choice below follows from one rule: the chart must keep drawing and the boat must keep moving on it, whatever else fails.

## Golden rules

- **Never block the render thread.** The SDL main loop (`app.rs::main_loop`) handles input, draws and sleeps 16 ms, in one thread. Anything that can take more than a millisecond (decoding, encoding, network, disk) runs on a worker thread and talks to the loop through a channel the loop drains once per iteration. Tile loading, JPEG encoding and Signal K already work this way; follow those patterns.
- **Degrade, never exit.** No Signal K server, no chart tiles at the minimum zoom, an unbindable streaming address, a corrupt `state.json`: each one logs a WARN or ERROR and the app carries on with a fallback. A `?` in startup or the main loop that would quit the app needs a reason you can defend to a sailor.
- **Validate every coordinate at the boundary.** Longitude in -180..=180, latitude in -90..=90, both finite, before anything is stored or drawn. Signal K deltas, GPX and JSON files and user clicks are all boundaries. Coordinates are `(lon, lat)` tuples everywhere; see the gotcha in `gps.rs` below.
- **Every file the app writes (`default/`, `export/`, GPX included) goes through `geo_data::atomic_write`** (temp file, sync, rename). Build the contents in memory, then one call. Power cuts mid-write are normal on a boat.
- **Leave the tree green.** `cargo build` with zero warnings and `cargo test` passing before every commit. Ship the test in the same commit as the change, at a public seam (see Testing).
- **Tests write to `std::env::temp_dir()`**, never to `default/`, `export/` or the repo.

## Quality gate (run before finishing)

```bash
cargo fmt -- --check                        # no output: the tree is rustfmt-clean
cargo build 2>&1 | grep -c '^warning'      # must print 0
cargo test                                  # all pass; 5 benchmarks stay ignored
cargo clippy --all-targets 2>&1 | grep -c '^warning'   # must print 0
```

- Touched the renderer or the stream? Also run the benchmarks and put before/after medians in the commit message: `cargo test --release bench_ -- --ignored --nocapture --test-threads=1` (single-threaded, or the timings interfere; needs a chart in `assets/` or `map/`).
- Changed anything the user sees? Run it: `cargo run --release`, pan, zoom, change chart, close the window, then read the tail of `plotter.log`. "Tests pass" is not "it works".
- The tree is rustfmt-clean with default settings. Run `cargo fmt` before committing; never mix a formatter-only change with a behaviour change.
- Clippy is clean. A lint that is wrong for this code gets a targeted `#[allow(clippy::...)]` with a one-line reason above it (see `ConfirmAction`), never a crate-wide allow.
- No `#[allow(dead_code)]` outside the one documented test-only field in `stream.rs`. Dead code is deleted, not silenced.

## Module layout

One binary crate, modules declared in `src/main.rs`. Dependency direction: `main → app → {renderer, screen, data modules, services}`. Data modules depend on `mbtiles_renderer` (for `gps_to_point`) and `screen` (for drawing primitives) and on nothing else; services depend on `config` and `diagnostics` only.

| Module                                                               | Role                                                                                                                                                                                                                                                          |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main.rs`, `action.rs`                                               | `main.rs` opens the log, loads config, creates the SDL window and texture, starts the streamer, hands over to `app::Plotter`, and declares `Mode` (Measurement / EditRoute / EditWaypoint). `action.rs` holds `AppAction`, the one vocabulary menus, keys and gestures translate into. |
| `app.rs` + `app/{session,actions,tools,gesture,input,render,confirm_dialog}.rs` | `Plotter`: the SDL loop and widgets around a `Session`. `session::Session` holds everything the user edits (renderer, state, route, track, waypoints, measurement, boat, pending confirmation, export dir) with no SDL handle; `actions` (`execute_action`, `apply_confirmation`) and `tools` (`target_at`, `click`, `long_press`, `drag_to`, `drag_end`, `name_waypoint`, per mode) are implemented on it and tested with a fixture chart and temp files. `gesture::GestureRecognizer` is pure and time-injected: press/motion/release/tick in, `Click`, `LongPress`, `DragStart/Move/End` out. `input` only translates SDL events into those and keeps what needs SDL (text input, cursor, dragging the chart texture). `render::display` composes a frame; `confirm_dialog` splits the font-free `Confirmation` from the `ConfirmDialog` that draws it. |
| `mbtiles_renderer.rs` + `mbtiles_renderer/{projection,tile_store}.rs` | The chart view (`MBTilesRenderer`: dimensions, zoom animation, rotation, the explicit `View` set by `look_at`, compositing tiles into the texture) composed from `projection` (pure Web Mercator and rotation maths, no state), `view` (`ChartView`, the copyable pixel<->position snapshot) and `tile_store` (`TileStore`: SQLite connection, metadata, LRU cache, `TileLoader` worker thread). Only `MBTilesRenderer`, `ChartView` and `project_norm` are visible outside. |
| `screen.rs`                                                          | `Screen`: the SDL window, canvas, cursors and every drawing primitive the data modules use (`show_boat`, `show_route`, `show_waypoint`, `show_track`, ...). Owns the `ClipRect` that every SDL2_gfx call goes through. Menus and dialogs draw on `screen.canvas` directly because they are chrome. Not to be confused with `state.map`, which is the chart file. |
| `route.rs`, `track.rs`, `waypoints.rs`, `mesure.rs`                  | Navigator data. Each loads from and saves to `default/` and knows how to draw itself. `mesure` is the distance/bearing tool.                                                                                                                                  |
| `gpx.rs`                                                             | GPX 1.1: `write_route`, `write_track`, `write_waypoints` return the document for the caller to `atomic_write`; `read_points`/`load_points` is the one strict reader (any point tag, both quote styles, coordinates validated). |
| `geo_data.rs`                                                        | `GeoFile<R>`: one collection at one path with the policy every file under `default/` follows (missing = empty, damaged = set aside as `.corrupt` and logged, legacy neighbour or older shape migrated once, atomic writes, coordinates validated both ways). `GeoPoint` is the route/track record; waypoints bring their own named record. Also `set_aside` and `atomic_write`. |
| `signalk.rs`, `ais.rs`, `boat.rs`                                    | Signal K client task (tokio), delta decoding into `BoatState` and `AisTarget`s, AIS expiry.                                                                                                                                                                   |
| `stream.rs`                                                          | MJPEG server: accept thread, one thread per viewer, encoder thread.                                                                                                                                                                                           |
| `state.rs`, `config.rs`, `diagnostics.rs`                            | `state.json` (position, zoom, toggles), `plotter.toml` with validation and defaults, the level-filtered rotating log.                                                                                                                                         |
| `topbar.rs`, `buttombar.rs`                                          | Menu bar (`MenuBar`: each entry carries its `AppAction`, chart entries are `ChangeChart(i)`) and status bar.                                                                                                                                                                 |
| `gps.rs`                                                             | Haversine distance and bearing in nautical miles.                                                                                                                                                                                                             |

## Threads, channels and the frame

Four kinds of thread, all started at launch:

- **SDL main loop** (`app::main_loop`, inside `#[tokio::main]`): per iteration it polls SDL events, drains the track channel, reads the boat `watch`, ticks the gesture recogniser for long presses, refreshes AIS, advances zoom animation, syncs the view, draws if `dirty`, sets the cursor, calls `prefetch_step(8)`, sleeps 16 ms. Every 10 s it logs loop statistics. Set `self.dirty = true` after any change the user should see; nothing redraws otherwise.
- **Signal K task** (tokio): connects with retry, subscribes to the configured paths, decodes deltas through `apply_delta` into a `DeltaSummary`, and publishes `BoatState` (watch), track points (unbounded mpsc) and the AIS list (watch) only when they changed. Evicts AIS targets silent for 5 minutes at most once a second.
- **Tile loader** (`TileLoader`, one per `MBTilesRenderer`): own read-only SQLite connection, request channel in, decoded tiles out, drained by `prefetch_step`. A tile a frame needs before the loader delivers it is decoded on the main thread so frames are never partial.
- **MJPEG**: a non-blocking accept loop, a blocking thread per viewer, and one encoder thread fed by a capacity-one channel so a slow encode drops frames instead of queueing them.

`display` draws in this order: chart texture, measurement, route, boat, AIS, track, waypoints, night overlay, bottom bar, then captures for the stream, then top bar and dialog, then presents. The stream therefore never shows the menus.

## Conventions & gotchas

- **Coordinate order.** Tuples are `(lon, lat)` throughout, including `AisTarget::position`, route points, `screen_to_gps` and every function in `gps.rs`. `gps.rs` is the only haversine: `distance_nm`, `distance_meters`, `bearing_deg`. Do not add a local copy; `mesure.rs` reports nautical miles and `track.rs` works in metres from the same functions.
- **The view is explicit.** `Session::sync_view()` derives the centre (boat while following, else `state.position`), the pan offset and the course-up rotation from state and pushes them into the renderer with `look_at`/`set_rotation`. It runs at construction, at the top of every loop iteration, after every `execute_action` and in `display`. `renderer.view()` returns a copyable `ChartView` snapshot of it: `to_pixel(lonlat)`, `to_lonlat(pixel) -> Option` (absolute screen pixels, longitude wrapped, `None` off the Mercator square), `nearest(points, pixel, radius)` for every hit test, `panned_center`, `contains`, `course_to_screen`. Input, hit tests, overlays, prefetching and `render_frame` all derive from that one view, so there is no render-order dependency and no "centre minus click" arithmetic anywhere. Change state, then call `sync_view`; never set the view directly.
- **i16 drawing.** SDL2_gfx takes `i16`. Never cast a screen coordinate to `i16` yourself; pass `i32` `Point`s into `screen.rs` and let `ClipRect` clip segments and cull points. Off-screen points wrap otherwise and draw phantom lines across the chart.
- **Zoom.** `available_zooms` is what the chart actually contains and can have gaps; `source_zoom_for` picks the level to draw from and `display_zoom` is fractional during the 240 ms animation. Zooming in at the top level is a no-op; zooming out past the bottom switches to `charts.default`, the overview chart, which is intended.
- **MBTiles rows are TMS.** `fetch_tile_bytes` flips `y` (`n - 1 - y`). Metadata `minzoom`, `maxzoom`, `center` and `bounds` are optional and the code copes without them.
- **Signal K self context.** The hello message's `self` field identifies our own vessel; deltas for that context and for `vessels.self` are the boat, every other `vessels.*` is AIS. Without the hello, only `vessels.self` is recognised.
- **Streaming address.** A failed bind logs an ERROR and the app runs without the stream (`Streamer::start_or_disable`). The checked-in `plotter.toml` binds 127.0.0.1 so a dev machine starts; the boat's address goes in deployment config. On Windows an accepted socket inherits the listener's non-blocking flag; `serve` resets it, keep that.
- **Logging.** `diagnostics::record(level, message)` for everything an operator might need; levels are DEBUG, INFO, WARN, ERROR, PANIC. INFO is the default floor, `PLOTTER_LOG=debug` lowers it, 5 MB cap with one `.1` backup. The only `println!`/`eprintln!` allowed are the fallbacks in `diagnostics.rs` and `main.rs` for when the log file itself cannot be opened, and the `bench_*` tests; a release build on Windows has no console, so a print is a message to nobody.
- **Paths are relative to the working directory**: `default/`, `export/`, `assets/`, `plotter.log` and the chart `directory`. Run from the repo root. Charts (`*.mbtiles`, `map/`) are gitignored; `WINDOWS_BUILD.md` says where to get test charts.
- **French.** Log messages and many comments are in French; identifiers mix French and English (`Mesure`, `Buttombar`, `restante`, `RAYON_TERRE`). Write new identifiers and comments in English and leave existing names alone unless the commit is a rename.
- **Windows build.** `Cargo.toml` adds `use-vcpkg` and `static-link` to `sdl2` only under `cfg(windows)`; Linux resolves the same three features as before. Keep platform differences in `Cargo.toml` and `.gitattributes`, never in `#[cfg]` blocks in the drawing or data code.
- **Dependencies.** Access WebSocket types through `tokio_tungstenite::tungstenite`; there is no direct `tungstenite` dependency. `tokio` is used only for the Signal K task.

## Non-negotiables

- A saved file must never be able to stop the app from starting. `GeoFile::load` and `State::new` enforce it: bad values are rejected, the file is set aside with a `.corrupt` suffix, an ERROR is logged, and the app continues with an empty collection or defaults. Loaders therefore return the collection, not a `Result`.
- An index from saved state (`nav_id`, chart index, selection) is clamped or checked before use. `Route::active_waypoint` is the model.
- Track points are never dropped silently: the track is checkpointed every `checkpoint_interval_seconds` and on exit, and `ClearTrack` asks for confirmation.
- Course-up rotation applies to the chart and every overlay through `MBTilesRenderer::rotation`; nothing draws in unrotated chart space on its own.
- The MJPEG stream shows the chart, never the menus or dialogs, so a viewer cannot be confused by a half-open menu on the tablet.

## Adding things

- **A menu action:** add an `AppAction` variant with its `name()` in `action.rs`; add `entry("Label", AppAction::Variant)` to a `Menu` in `topbar.rs::MenuBar::new` (the entry carries its action, there is no string matching); implement it in `actions.rs::execute_action` on `Session`, with a test beside the others there. Destructive actions request a `Confirmation` and do their work in `apply_confirmation` (see `ClearRoute`).
- **A config field:** struct field with `#[serde(default)]` semantics, a `Default` value, a check in `AppConfig::validate`, the key in `default/plotter.toml`, and the README section. The round-trip and defaults tests in `config.rs` will catch a missing `Default`.
- **A persisted data file:** a record type implementing `geo_data::GeoRecord` and a `GeoFile<R>` owned by the collection; `load()` never fails and `save()` is atomic. Older files are migrated with `with_legacy_file` (a neighbour such as `.gpx`) or `with_legacy_shape` (an older shape at the same path); bump `FORMAT_VERSION` only with such a migration.
- **A Signal K path:** field in `SignalKPaths` with a default, subscribe it in `SignalKClient::subscribe`, decode it in `apply_delta`, extend the `DeltaSummary` if the loop must react, and add a case to the `apply_delta` test.
- **A drawing primitive:** in `screen.rs`, taking `i32` `Point`s, clipping through `ClipRect` before any `i16` conversion, colours from `AppearanceConfig`.
- **Background work:** a thread owning its resources, a channel in, a channel out, a `drain` called from the main loop, and a `Drop` that closes the channel and joins the thread. `TileLoader` is the template.

## Testing

Tests live in `#[cfg(test)] mod tests` beside the code. Test at public seams, named for the behaviour (`navigation_distance_includes_the_remaining_legs`), with expected values from an independent source (60 nautical miles per degree of latitude, not the function under test).

- **Seams that exist:** `GestureRecognizer` with injected `Instant`s; `Session` tools via the shared fixture in `session::test_support` (`target_at`, `click`, `long_press`, `drag_to`); `ChartView::centred_on` for every pixel/position conversion and hit test, no chart file needed; `Session` (build it with a struct literal over `fixture_chart` and temp paths, see `actions.rs` tests) for every `AppAction` and confirmation; `MenuBar` for the menu geometry; `gps` free functions; `GeoFile` and `Route::load`, `Track::with_checkpoint_interval`, `Waypoints::load`, `State::new` on temp files (including corrupt ones); `apply_delta`, `update_ais_target`, `retain_fresh` for Signal K; `MBTilesRenderer` via `fixture_chart`, which writes a real MBTiles file with solid PNG tiles; `clip_segment` and `ClipRect` for drawing; `Logger` for the log; `listen`, `serve` and `Streamer::submit` for the stream, driven by a raw `TcpStream` client.
- **No seam:** `Plotter`, `input.rs`, `render.rs`, `Screen` and the widgets need an SDL window. Logic that wants a test goes on `Session` (if it is an action) or in the module that owns the data; `input.rs` only translates events into gestures and `AppAction`s; a new editing behaviour goes on `Session` in `tools.rs` with a test.
- Benchmarks are `#[ignore]` tests named `bench_*`, printing a median; see Quality gate.
- A fixture file is deleted at the end of its test, so anything holding it open (a loader thread, a connection) must be dropped first. On Windows the delete fails otherwise.
