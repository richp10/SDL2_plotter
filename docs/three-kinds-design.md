# Three kinds of layer: Chart, Satellite, Map

Agreed 2026-11-24, after the photo button (see `photo-button-design.md`).

## Why

A layer had one of two roles, Chart or Imagery. Two things were wrong with
that. OSM-style land maps counted as charts, though they have no soundings.
And the manager's layer column could move a satellite file into a chart layer
with a tap. A layer stays one source (Navionics, Bing, Google...), because
satellite sources differ by date and cloud and chart sources differ in their
depths. What it is gets a clearer word and a third value.

## Decisions

1. **Kind.** `Role` becomes `Chart`, `Satellite` or `Map`. Saved
   `"imagery"` reads as Satellite, so existing settings keep working.
   "Satellite" is the word the user sees everywhere. The button placeholder
   says SAT, MAP or CHART.
2. **Getting to a Map.** The photo button steps through every non-chart
   source here: chart, then the last non-chart used, then the remaining
   satellites, then the remaining maps, then back to chart. A map is offered
   on the same terms as a photo (at most `MAX_PHOTO_OVERSCALE` levels past
   its deepest).
3. **Fallback.** A Map is never a fallback, and neither is a Satellite.
   Automatic switching only lands on a Chart, or on the overview.
4. **Changing kind** (manager, Kind column) cycles Chart → Satellite → Map
   → Chart for the whole layer. A change into or out of Chart asks first.
   The title line says what will happen ("Make Bing satellite a Chart? It
   will be used for automatic switching. Tap Kind again to confirm."), and
   any other tap cancels. Satellite ↔ Map applies at once.
5. **Source column** cycles only through layers of the same kind, then a
   layer of its own. A file changes kind only through the Kind column.
6. **OpenSeaMap-test**, cycled into Navionics by accident, goes back to its
   own OpenSeaMap chart layer (a one-off edit of the owner's
   `settings/charts.json`).
7. **Headers.** The manager table gets a header row: Order · Name · Source ·
   Kind · Zooms · Visible · Locate. Visibility reads Visible / Hidden.
8. **Colours.** Chart black, Satellite red, Map dark amber.
9. **On screen.** The bottom bar (streamed) adds " · NO DEPTHS" while a
   Satellite or Map file is shown. The overview background is labelled
   "Overview map".
10. **Name guesses.** "openstreetmap" and "osm" join an OpenStreetMap layer
    and "topo" a Topo layer, all of kind Map. "openseamap" is checked first
    and stays a Chart. Plain "map" is not a clue.

## Not changed

A lock works on any kind. `state.imagery_layer` keeps its name in
`state.json` and now means the last non-chart layer used.
