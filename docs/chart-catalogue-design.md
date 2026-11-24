# Chart catalogue: design

Agreed 2026-11-24 in a grilling session, from the recommendation in
[the chart format appraisal](chart-format-appraisal-2026-11-24.md) ("a chart
catalogue instead of a file list"). Evidence: [chart catalogue
research](chart-catalogue-research-2026-11-24.md). Work is tracked on the
wayfinder map [chart catalogue](wayfinder/chart-catalogue.md).

## Why

Today every `*.mbtiles` in `map/` is one entry in the Charts menu, switching
is manual and whole-screen, and nothing distinguishes a navigational chart
from a satellite photo. The owner's files show the problem: twelve files that
are really four sources (Navionics, Bing, Google, ArcGIS) at several scales,
every one tagged `type = baselayer` in its metadata, and recorded zoom ranges
that disagree with the tiles. The app must work with any combination of
files, not just these.

## Vocabulary

- **Chart file**: one MBTiles file in the chart directory.
- **Layer**: an ordered group of chart files from the same source at
  different scales ("Navionics", "Bing satellite"). The unit the layer
  switcher chooses and the scope of automatic switching.
- **Role**: per layer, `Chart` or `Imagery`. Imagery is a photo, never a
  navigational chart, and is only ever shown because the user chose it.
- **Real zoom range**: the zoom levels a file actually contains, read from its
  tiles, not its metadata.
- **Active file**: the chart file currently drawn for the active layer;
  **gap-fill files**: what is drawn where it has no tile: its own coarser
  levels, then up to three files on screen (its own layer first, then other
  chart layers), hatched. See `Catalogue::fills`.
- **Lock**: freezes the active file against automatic switching.

## Decisions

1. **Layers.** Files are grouped into layers, each with a role. Layers and
   roles are guessed from file names as a convenience only; the user can
   regroup in the manager, and nothing depends on the guess being right.
2. **New files.** A file not yet in the catalogue joins a default layer
   "Charts" with role Chart, works at once, is marked "new – check role" in
   the manager, and is logged at INFO. Only names that clearly say so (Sat,
   Imagery, Aerial, Ortho, Photo, Bing, Google, Esri, ArcGIS) are guessed as
   Imagery. The MBTiles `type` key is ignored: writers disagree on its
   default (GDAL, QGIS, tippecanoe write `overlay`; MOBAC writes `baselayer`).
3. **Storage.** `settings/charts.json` (a new `settings/` folder holding only
   this file), keyed by file name, loaded through the `GeoFile`-style policy:
   missing = defaults, damaged = set aside as `.corrupt` with an ERROR, never
   stops the app starting. Chart files are never written. `state.json` and
   `plotter.toml` stay where they are.
4. **Real zoom range** is read from the tiles table and cached in
   `settings/charts.json` with the file's size and modification time, so
   multi-gigabyte files are not rescanned at each start. Coverage comes from
   metadata `bounds`, falling back to the tile extent when absent.
5. **Automatic switching.** Within the active layer, a file is eligible when
   its coverage contains the screen centre and its real zoom range includes
   the current zoom. The most detailed eligible file wins: highest top zoom,
   then smaller coverage area, then the user's order. Beyond the top zoom of
   the most detailed file, that file stays on, overscaled. Choices are made at
   whole zoom steps only, so the 240 ms zoom animation never flickers between
   files.
6. **Fallbacks.** When no file in the active layer covers the screen centre,
   the app shows the best Chart layer that does, and the bottom bar says so
   ("No Bing here – showing Navionics"). The fallback is never Imagery.
   `charts.default` (the world chart) is the last resort.
7. **Gap fill.** At most two files draw at once: the less detailed covering
   file first, the active file over it, so a chart edge never leaves blank
   background. Steady-state frames are cached; a cold view reads about twice
   the tiles.
8. **Lock.** Freezes the active file while panning and zooming; persists in
   `state.json`; released by switching layer or pressing lock again. Outside
   the locked file's coverage the gap-fill file keeps drawing and the bar
   shows "Outside locked chart". It is never released automatically.
9. **Imagery.** Chosen only by hand. "Imagery on/off" returns to the last
   imagery layer used. While imagery shows, a large "PHOTO – not a chart"
   badge sits top centre, appears in the MJPEG stream too (it is information
   about the chart, not chrome), and returns to the chart when tapped.
   Blending imagery over a chart with opacity is deferred.
10. **Indicators** (bottom bar): active layer and file; "OVERSCALE ×N" once
    zoom is at least one level past the file's top zoom; "More detail: …" when
    the lock or the zoom hides a more detailed file; a lock symbol while
    locked. These meet the IMO MSC.232(82) §6.1 overscale and
    larger-scale-available indications and §7.1/7.2 (distinguishable,
    removable by one action) for imagery.
11. **Charts menu.** "Chart manager…", one entry per layer (the layer
    switcher), "Lock chart", "Imagery on/off". Individual files leave the
    menu.
12. **Chart manager window.** Full-screen touch panel, one row per file:
    editable display name, layer, role, real zoom range, coverage outline on
    the chart when selected, show/hide, order. No delete, import or download.

## Constraints carried from the codebase

- Nothing new blocks the 16 ms loop: catalogue scanning runs at startup (or
  on a worker when the cache is stale), tile reads stay on the loader thread.
- The switching rule and the catalogue are pure modules with no SDL, tested
  at their interface with fixture charts and temp files, as `TileStore` and
  `Session` are today.
- The pan-offset rule holds: a file switch is a chart change and goes through
  `anchor_on_visible_center`.
