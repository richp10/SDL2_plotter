# Map: architecture deepening (wayfinder:map)

Charted 2026-11-23 from [the architecture review](../architecture-review-2026-11-23.html).
Tracker: this file (local markdown). Tickets are the sections under "Tickets";
a ticket is claimed by setting its status to `in progress` and closed by
recording its resolution. Blocking is written in each ticket's `blocked by`.

## Destination

Every Strong and Worth-exploring candidate in the review is implemented and
committed, one commit per candidate, with tests at the new seam, the tree
green at each step (rustfmt clean, 0 warnings, clippy 0, `cargo test`,
release run), and CLAUDE.md describing the result.

## Notes

- **Execution is carried into this map** (override of "plan, don't do"): the
  effort runs without a human in the loop, so each grilling ticket is
  resolved by the agent stating the decision and its reasons in the
  resolution, then implementing it in the same session.
- Skills: `codebase-design` for the vocabulary (module, interface, depth,
  seam, locality, leverage), `tdd` for every behaviour change (red first at
  the new seam), `diagnosing-bugs` for the defects the review uncovered.
- Standing preferences: public names that other modules use keep their
  names where possible; French log text stays French; identifiers in
  English; no `#[allow]` without a reason beside it.
- Order follows the blocking edges below: persistence and the two small
  tickets first, then `ChartView`, then the two that build on it.

## Decisions so far

<!-- one line per closed ticket, appended as each is resolved -->

- [T1 — One persisted-collection module](#t1--one-persisted-collection-module-geofile) — `GeoFile<R>` in `geo_data.rs`, generic over a `GeoRecord`, with two optional legacy adapters and a `load` that never fails; route, track and waypoints each own one, `State` reuses `set_aside`.
- [T4 — Menu entries carry their action](#t4--menu-entries-carry-their-action) — `MenuBar::event -> Option<AppAction>`; `AppAction` moved to `action.rs`; the 26 string constants and the `input.rs` match are gone.
- [T5 — One gpx module](#t5--one-gpx-module) — `src/gpx.rs`: three writers returning a `String`, one strict reader; creator "SDL2 Plotter"; `route/gpx.rs` deleted.
- [T2 — ChartView as the screen-to-chart seam](#t2--chartview-as-the-screen-to-chart-seam) — copyable snapshot from `renderer.view()`; absolute pixels in, validated wrapped positions out; one `nearest` hit test; six "centre minus click" copies and four hit-test copies gone.
- [T3 — Gesture recogniser separate from per-mode tools](#t3--gesture-recogniser-separate-from-per-mode-tools) — `app/gesture.rs` (pure, time-injected) emits Click/LongPress/Drag; `app/tools.rs` puts `target_at`, `click`, `long_press`, `drag_to`, `drag_end`, `name_waypoint` on `Session`; `input.rs` 705 -> 394 lines; the three defects fixed with tests.
- [T6 — Main-loop policy onto its owners](#t6--main-loop-policy-onto-its-owners) — `Track::record/tick/clear/close` own checkpointing; `Session::on_boat`, `set_ais`, `expire_ais` own intake and expiry; the loop drains, calls, marks dirty; time injected.

## Not yet specified

- Whether `Session`'s fields can all go private once tickets 3 and 6 move
  behaviour onto it, and what the resulting interface looks like.
- Whether `display_buttombar`'s text composition becomes `Session::status_line()`.
- `drag_map` presents without overlays, so the boat, route and AIS vanish
  during a pan; intended or not is a product question for the author.

## Out of scope

- A unified point-list type for route, track, waypoints and measurement
  (review candidate 8): the four shapes differ and the type would be
  forced; share helpers, not the type.
- Settling on one language for identifiers and comments (user decision,
  deferred).

## Tickets

### T1 — One persisted-collection module (`GeoFile`)
- type: grilling + task · status: closed · blocked by: none
- Resolution: `GeoFile<R: GeoRecord>` holds the path and the policy; `load() -> Vec<R>` never fails (missing = empty, unparseable = try `with_legacy_shape` then set aside as `.corrupt` + ERROR, missing + `with_legacy_file` neighbour = migrate once), `save` validates and writes atomically, `remove` for a cleared track. Generic over the record rather than a fixed point so waypoints keep their names in the same versioned envelope; the old bare-array waypoints file is the first legacy shape. Route, Track and Waypoints own their file (no more path parameters or `Session::track_path`), and their loaders return the collection, not a `Result`, so `Plotter::new` has no `?` on saved data. `State` keeps its own shape but uses `set_aside`. Rejected: a shared `PointList` type (out of scope) and keeping `read_or_migrate` free functions (two sources of truth for the path).
- Question: what is the small interface of a persisted-collection module
  that gives route, track and waypoints the same set-aside, version,
  validation, migration and atomic-write policy `state.rs` already has, so
  that no saved file can stop the app from starting?

### T2 — `ChartView` as the screen-to-chart seam
- type: grilling + task · status: closed · blocked by: none
- Resolution: `mbtiles_renderer::view::ChartView` (Copy: centre in world pixels, display zoom, rotation, size), built by `MBTilesRenderer::view()`. Interface: `to_pixel`, `norm_to_pixel`, `to_lonlat -> Option` (absolute pixels; longitude wrapped into range; `None` when the pixel leaves the Mercator square, which is the real "beyond the poles" condition since Mercator latitude never exceeds 90), `panned_center(dx, dy)`, `nearest(points, pixel, radius)`, `contains(pixel, margin)`, `course_to_screen`. `screen_to_gps` and its centre-minus-click convention are deleted; `gps_to_point`/`norm_to_point` stay on the renderer as delegates for the overlays until T7. Mesure, Route and Waypoints take `&ChartView` and absolute `Point`s; their four hit tests are one `nearest`. `render_rect`, `texture_rect`, `display_dimensions`, `rotation_degrees` and `Route::calc_route` are private. A click with no position (off the world) is ignored rather than stored. Deferred to T7: `boat.rs` still reads `Screen` size for its culling.
- Question: what does a copyable view snapshot expose so that input, hit
  testing and overlays take absolute pixels, get validated coordinates, and
  stop reading `Screen`'s public size and repeating "centre minus click"?

### T3 — Gesture recogniser separate from per-mode tools
- type: grilling + task · status: closed · blocked by: T2
- Resolution: `GestureRecognizer::new(long_press, tolerance)` with `press(at, now)`, `motion(at)`, `release(at)`, `tick(now)`, `is_dragging()`, emitting `Gesture::{Click, LongPress, DragStart{origin,at}, DragMove, DragEnd}`; a long press suppresses the click and any later drag, a drag suppresses the long press, and every drag (chart, point, mark) starts only past the tolerance. The mode's tool on `Session` interprets gestures in absolute pixels: `target_at(pixel) -> Target::{RoutePoint, RouteHandle, Waypoint, MeasureMark, Chart}` decides once at `DragStart` what is grabbed and the grab is locked for the drag (fixes dragging A across B grabbing B); `click` returns `Option<ClickRequest::NameWaypoint(pos)>` because typing the name needs SDL text input; `long_press` removes and saves; `drag_to`/`drag_end` move and then persist. Wheel zoom is `AppAction::ZoomIn/ZoomOut` like the keys and no longer resets the pan offset. `Plotter` lost seven gesture fields (`press`, `is_draging`, `xdrag`, `ydrag`, `drag_start`, `selected`, `dragging_measure_start`) for `gestures` and `grab`; the chart drag still presents the texture from `input.rs` because it needs the texture. Tests: five on the recogniser (click within tolerance, drag from the press origin, long press once after 500 ms with no click after, drag suppresses long press, no press ignored) and seven on the tools (target per mode, click adds and saves, handle inserts, long press removes and saves, grab stays locked across another point, two clicks measure 600 nm, waypoint click asks for a name and blank names are refused).
- Question: what events does a pure, time-injected gesture recogniser emit,
  and which per-mode editing logic moves from `input.rs` onto `Session`, so
  that the three defects (drag re-grab, skipped tolerance, wheel zoom
  bypassing actions) have a seam and a test?

### T4 — Menu entries carry their action
- type: task · status: closed · blocked by: none
- Resolution: each `Entry { label, action }` is built in `MenuBar::new`, chart entries as `ChangeChart(i)` in `charts_path` order; `event` returns the entry's action. `AppAction` moved from `app.rs` to its own `action.rs` (re-exported by `app`) so `topbar` does not depend on `app`; it derives `Copy` and `PartialEq` for the tests. The string constants, the chart-name fallback and `Plotter::charts_name` are deleted. Tests: a chart named "Exit" or "On" still switches chart; every entry of every menu yields an action.
- Question: `MenuBar::event` returns `Option<AppAction>` built from the menu
  definition; what happens to the 26 string constants and the chart-name
  fallback?

### T5 — One `gpx` module
- type: task · status: closed · blocked by: T1 (callers save through GeoFile)
- Resolution: `gpx::write_route/write_track/write_waypoints` build the document and the caller saves it with `atomic_write`; `gpx::read_points` (and `load_points` on a file) accepts `trkpt`, `rtept` and `wpt`, both quote styles and either attribute order, and rejects a missing, unparseable or out-of-range coordinate with `InvalidData`, which is the stricter of the two old readers. The surviving creator string is "SDL2 Plotter"; the surviving escaper handles all five reserved characters (the waypoint one missed quotes). `route/gpx.rs`, `track::load_gpx` and `waypoints::escape_xml` are deleted.
- Question: one writer per kind and one validating reader; which of the
  three creator strings and two escapers survives?

### T6 — Main-loop policy onto its owners
- type: grilling + task · status: closed · blocked by: none
- Resolution: `Track::record(pos, now)` adds and checkpoints the first point at once, `tick(now)` checkpoints when pending and the interval has passed, `clear(now)` empties and removes the file, `close()` saves on exit; `checkpoint` is private and `last_save` is set from the injected `now`, so the policy is tested with fabricated instants. `Session::new(...)` replaces the struct literal (the fixture uses it too); `on_boat` logs validity transitions and follows a valid fix; `set_ais` stores the published list and `expire_ais(now)` thins it at most once a second with `ais::expire`, the same `is_fresh` rule the Signal K task applies through `retain_fresh`, kept in both places on purpose: the task evicts when deltas arrive, the session when the connection has gone quiet. `Plotter` lost `ais_targets` and `last_ais_cleanup`. Tests: first point checkpointed immediately; later points only after the interval; close saves pending points; a boat update moves the view only while following and never on an invalid fix; stale AIS targets expire after the timeout and at most once a second.
- Question: which of the loop's inline rules (track checkpointing, boat
  intake while following, AIS expiry in two threads) move onto `Track` and
  `Session`, with what time injection, and what does the loop keep?

### T7 — Draw list and one painter for overlays
- type: grilling + task · status: open · blocked by: T2
- Question: what shapes do the data modules emit, and what does one painter
  need from `ChartView` and `AppearanceConfig` to cull, clip and style them,
  so that `boat.rs` stops logging every frame and every primitive clips?
