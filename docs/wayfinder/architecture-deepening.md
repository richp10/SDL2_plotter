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
- type: grilling + task · status: open · blocked by: none
- Question: what does a copyable view snapshot expose so that input, hit
  testing and overlays take absolute pixels, get validated coordinates, and
  stop reading `Screen`'s public size and repeating "centre minus click"?

### T3 — Gesture recogniser separate from per-mode tools
- type: grilling + task · status: open · blocked by: T2
- Question: what events does a pure, time-injected gesture recogniser emit,
  and which per-mode editing logic moves from `input.rs` onto `Session`, so
  that the three defects (drag re-grab, skipped tolerance, wheel zoom
  bypassing actions) have a seam and a test?

### T4 — Menu entries carry their action
- type: task · status: open · blocked by: none
- Question: `MenuBar::event` returns `Option<AppAction>` built from the menu
  definition; what happens to the 26 string constants and the chart-name
  fallback?

### T5 — One `gpx` module
- type: task · status: open · blocked by: T1 (callers save through GeoFile)
- Question: one writer per kind and one validating reader; which of the
  three creator strings and two escapers survives?

### T6 — Main-loop policy onto its owners
- type: grilling + task · status: open · blocked by: none
- Question: which of the loop's inline rules (track checkpointing, boat
  intake while following, AIS expiry in two threads) move onto `Track` and
  `Session`, with what time injection, and what does the loop keep?

### T7 — Draw list and one painter for overlays
- type: grilling + task · status: open · blocked by: T2
- Question: what shapes do the data modules emit, and what does one painter
  need from `ChartView` and `AppearanceConfig` to cull, clip and style them,
  so that `boat.rs` stops logging every frame and every primitive clips?
