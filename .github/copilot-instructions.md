# Copilot instructions for SDL2 Plotter

## Project shape

SDL2 Plotter is a single Rust binary for a marine chart table: it renders local MBTiles raster charts, follows a vessel from Signal K, displays AIS/routes/tracks/waypoints, and optionally serves the rendered chart as MJPEG. The deployment target is a long-running Raspberry Pi Linux system; Windows is primarily a development platform. Read `README.md` for setup and `CLAUDE.md` for the fuller design map and safety constraints.

Run commands from the repository root. Runtime paths such as `default/`, `settings/`, `export/`, `assets/`, `map/`, and `plotter.log` are relative to the current working directory. Chart files and most runtime state are installation data, not source fixtures.

## Build, run, and checks

Linux requires the SDL2 development libraries listed in `README.md`. Windows setup and the vcpkg/static-link arrangement are documented in `WINDOWS_BUILD.md`.

```bash
cargo run
cargo fmt
cargo build --release
./target/release/plotter

cargo check
cargo fmt -- --check
cargo build 2>&1 | grep -c '^warning'
cargo test
cargo clippy --all-targets 2>&1 | grep -c '^warning'
```

Run one test by name, or use its fully qualified module path:

```bash
cargo test navigation_distance_includes_the_remaining_legs
cargo test catalogue::tests:: -- --nocapture
cargo test catalogue::tests::some_test_name -- --exact --nocapture
```

Use `cargo test -- --list` to discover the exact names when a test filter is
unclear. `cargo test <filter>` runs matching tests; `--exact` requires the
complete test name.

Ignored renderer/stream benchmarks are run serially:

```bash
cargo test --release bench_ -- --ignored --nocapture --test-threads=1
```

When changing rendering or streaming, preserve benchmark coverage and compare medians. When changing visible behavior, run the release binary manually and inspect the end of `plotter.log`; automated tests do not exercise SDL widgets and the full event loop.

## Architecture

- `main.rs` initializes diagnostics, validated `AppConfig`, SDL, the streaming server, and `app::Plotter`.
- `app.rs` owns the SDL loop. `app/session.rs` is the SDL-free application model; `app/actions.rs` applies `AppAction`, `app/tools.rs` handles hit testing/editing, `app/gesture.rs` is a time-injected gesture state machine, and `app/render.rs` assembles a frame. SDL-specific input and widgets stay in `input.rs`, `chart_manager.rs`, `photo_button.rs`, and the bar/dialog modules.
- `MBTilesRenderer` combines Web Mercator projection, an explicit `ChartView`, zoom/rotation, and `TileStore`. Tile SQLite access and image decoding use a worker thread; the loop drains results and prefetches incrementally. `Catalogue` scans MBTiles files, caches their actual zooms/coverage in `settings/charts.json`, groups them into user-editable layers, and chooses the chart/photo source for the current centre and zoom.
- Data modules (`route`, `track`, `waypoints`, `mesure`, `boat`, `ais`) produce chart-coordinate `overlay::Shape` values. `overlay::plan` projects/culls/rotates them against one `ChartView`; `screen.rs` is the SDL drawing executor.
- `signalk.rs` runs the Signal K WebSocket task and publishes boat, track, and AIS updates through channels. `stream.rs` captures the already-rendered canvas on the SDL thread, then encodes JPEG on a worker; a capacity-one queue drops stale frames rather than blocking the chart.
- `geo_data.rs`, `gpx.rs`, and `state.rs` own persistence. `config.rs` validates `plotter.toml`; `diagnostics.rs` provides the rotating operator log.

The normal frame flow is: synchronize the view, poll SDL events, drain service channels, tick gestures/zoom, render only when dirty, capture the stream frame, prefetch tiles, and sleep. Keep policies in `Session` or the owning service rather than duplicating them in the loop.

## Repository-specific conventions

- **Never block the render thread.** Disk, network, tile decoding, JPEG encoding, and other work that can take more than a millisecond belongs in a worker with channels and a main-loop drain. A slow or broken optional service must not hold back chart drawing.
- **Degrade instead of exiting.** Missing Signal K, unavailable tiles, corrupt persisted state, unreadable chart settings, or a failed stream bind should be logged with `diagnostics::record` and fall back to a usable state. Do not turn recoverable runtime failures into startup `?` returns.
- **Coordinates are `(longitude, latitude)` everywhere.** At every input boundary require finite longitude in `-180..=180` and latitude in `-90..=90`. Reuse `geo_data::valid_position`; keep haversine distance/bearing logic centralized in `gps.rs`.
- **All application writes are atomic.** Files under `default/` and `export/`, including GPX, must be built in memory and written through `geo_data::atomic_write`. `GeoFile` treats missing data as empty, sets corrupt data aside, supports migrations, and validates records on both load and save.
- **The view is the source of truth.** Change state, then call `Session::sync_view`; do not perform ad hoc click-to-coordinate or centre arithmetic and do not set renderer view state directly. `ChartView` supplies projection, hit testing, bounds, and course-up rotation. Before zoom/chart changes, fold a following-mode pan offset into the visible position with the existing anchoring path.
- **Clip before SDL integer conversion.** Overlay planning and `Screen` clipping must keep off-screen coordinates from wrapping when converted to SDL2_gfx `i16`; do not cast raw screen coordinates yourself.
- **Chart roles matter.** `Catalogue` distinguishes `Chart`, `Satellite`, and `Map`. Automatic switching and depth-bearing behavior are chart-only; user-selected photo/map layers are explicit. Use the catalogue rules rather than choosing an MBTiles file directly.
- **Actions have one vocabulary.** Menus, keys, and gestures translate to `AppAction` in `action.rs`; implement behavior on `Session`, and route destructive operations through the existing confirmation flow.
- **Keep persistence and data modules SDL-free.** They may depend on `ChartView` and `overlay::Shape`, but not on `screen`. Services depend on configuration/diagnostics rather than UI state.
- **Tests use public seams and temporary files.** Prefer pure projection/overlay/catalogue tests, SDL-free `Session` tools/actions, injected `Instant`s for time policies, and shared fixture charts. Never write tests into `default/`, `export/`, or the repository; use `std::env::temp_dir()` and clean up resources that hold MBTiles files open.
- **Logging is the user-facing diagnostic path.** Use `diagnostics::record` for operational messages. Avoid new `println!`/`eprintln!`; the only existing exceptions are startup logging failure fallbacks and benchmark tests. Existing identifiers/comments mix French and English; leave established names alone and use English for new names.
- **Keep the tree warning-free.** Use targeted clippy allowances only when a lint is demonstrably inappropriate, with a brief reason; do not add crate-wide or casual `dead_code` allowances.

## Changes that touch common seams

- A new menu action needs an `AppAction` variant/name, a menu entry, `Session::execute_action`, and a focused action test. Destructive actions also need a confirmation variant and `apply_confirmation`.
- A new configuration field needs serde defaults, a `Default` value, validation, the checked-in `default/plotter.toml` key, and README documentation.
- A new persisted collection should use `GeoFile`/`GeoRecord`, versioned JSON, migration hooks where required, coordinate validation, and atomic saves.
- A new background task should own its resources, expose input/output channels, be drained by the loop, and shut down/join cleanly like `TileLoader`.
- A new overlay requires a `Shape`, projection/culling in `overlay::plan`, a `Draw`, and a clipped private primitive in `screen.rs`.
