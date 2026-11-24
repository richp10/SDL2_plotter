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
- type: task · status: open · blocked by: none
- Question: a pure `catalogue` module that scans the chart directory, reads
  real zoom ranges and coverage (cached by size and mtime), guesses layers
  and roles from names, merges the user's saved settings, and persists them
  with the set-aside policy. Covers design decisions 1–4.

### C2 — The switching rule
- type: task · status: open · blocked by: C1
- Question: a pure function from (catalogue, active layer, lock, view centre,
  whole zoom) to (active file, gap-fill file, indicators), including the
  fallback to a Chart layer and the world chart. Decisions 5, 6, 8 (rule
  part), 10 (what to indicate).

### C3 — Two-file rendering
- type: task · status: open · blocked by: C2
- Question: the renderer draws a gap-fill file beneath the active file,
  each with its own `TileStore`, without blocking the loop; benchmarks
  before and after. Decision 7.

### C4 — Lock, imagery badge and indicators on screen
- type: task · status: open · blocked by: C2, C3
- Question: lock persisted in `state.json`, the photo badge (in the stream
  too), and the bottom-bar indicators wired to C2's output. Decisions 8–10.

### C5 — Charts menu as layer switcher
- type: task · status: open · blocked by: C1
- Question: the menu lists layers, "Chart manager…", "Lock chart" and
  "Imagery on/off" as `AppAction`s on `Session`. Decision 11.

### C6 — Chart manager window
- type: task · status: open · blocked by: C1, C5
- Question: the full-screen touch panel editing names, layers, roles,
  show/hide and order, with the coverage outline. Decision 12.
