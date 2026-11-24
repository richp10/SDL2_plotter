# Chart catalogue: how plotters organise multiple charts

*Research note, 2026-11-24. Primary sources are specifications, official manuals and source code read on this date; OpenCPN source is `master` at commit `72ae5b8`. Claims taken only from a search-engine excerpt, a forum or a third-party mirror are marked **[secondary]**. Chart formats and S-57 are covered in the sibling notes and not repeated here.*

## Summary

The MBTiles `type` key (`overlay` / `baselayer`) exists but is optional, and writers disagree on its default: GDAL, QGIS and tippecanoe write `overlay` unless told otherwise, MOBAC writes `baselayer`, and SAS.Planet writes whichever matches what was exported. So `type` cannot by itself tell a satellite mosaic from a chart; the plotter needs a user-set role per file, seeded from `type` at most. OpenCPN is the reference design for automatic scale switching: it orders charts by scale (the "piano"), gives each chart a band of display scales it may be used at, widens the bands until they overlap, and picks the largest-scale chart whose band contains the current display scale, falling back to smaller scales to fill gaps. It treats MBTiles as a separate family that is shown or hidden by hand rather than quilted with real charts, and its users' main complaint is that the chart changes under them as they zoom; the standard answer is a single-chart mode and chart groups. Web plotters (Freeboard-SK, AvNav) instead use a manual layer stack with per-layer opacity, default-ordered by scale with the most detailed on top. Commercial plotters present imagery as an optional photo layer with an opacity slider and a "land only" mode. IMO's ECDIS standard requires an indication when the display is overscale and when a larger-scale chart covers own ship, and requires other information to be "clearly distinguishable" from the chart and removable "by single operator action".

## 1. MBTiles metadata that bears on a chart catalogue

**The spec.** MBTiles 1.3 requires `name` and `format`; it says the metadata table SHOULD contain `bounds` (WGS 84, `left,bottom,right,top`, "an area covered by all zoom levels"), `center`, `minzoom` and `maxzoom`, and MAY contain `attribution` (HTML), `description`, `type` ("`overlay` or `baselayer`") and `version` (the tileset's own revision) [S1]. In 1.1, `type`, `version` and `description` were part of the basic metadata alongside `name` and `format` [S2]. The spec gives no meaning to `overlay` beyond the word, and defines no `scale` key.

**What writers actually put in `type` and `bounds`:**

| Writer | `type` | `bounds` | Source |
|---|---|---|---|
| GDAL raster driver | `TYPE` creation option, default **`overlay`** | written unless `WRITE_BOUNDS=NO` | [S3] |
| QGIS "Generate XYZ tiles (MBTiles)" | hard-coded **`overlay`** | written; also `minzoom`, `maxzoom`, `version` = `1.1` | [S4] |
| tippecanoe (vector) | hard-coded **`overlay`** | written, plus a non-standard `antimeridian_adjusted_bounds` | [S5] |
| SAS.Planet export | **`overlay`** when exporting an overlay layer, `baselayer` otherwise | written; also non-standard `scheme` | [S6] |
| MOBAC | **`baselayer`** | written | [S7] **[secondary]**: sample output in a SourceForge bug report and a third-party source mirror |
| MapTiler | not verified | not verified | no primary source found |

Consequences worth noting:

- A satellite mosaic made with SAS.Planet from a base map is labelled `baselayer`, exactly like a chart made with MOBAC; a chart rendered through GDAL or QGIS with defaults is labelled `overlay`. Neither label reliably separates imagery from charts.
- SAS.Planet can write XYZ rows and records it in a `scheme` metadata key, which its developer describes as a deliberate deviation from the TMS-only spec [S8] **[secondary: bug tracker]**. OpenCPN honours `scheme=xyz` [S9]; AvNav makes the user choose xyz or tms per file and says xyz is its default [S20]. A bug report says SAS.Planet export sometimes writes wrong `minzoom`/`maxzoom` and omits `bounds` [S8] **[secondary]**.
- OpenCPN reads `bounds`, `minzoom`, `maxzoom`, `name`, `description`, `type` and `scheme`, then "fix[es] the missing/wrong metadata values" from the tiles table, and sets the chart's nominal scale from `maxzoom` [S9, `mbtiles.cpp` lines 327-332, 549-584].
- @signalk/charts-plugin skips an MBTiles file whose metadata lacks `bounds`. It reports every MBTiles chart as `type: 'tilelayer'`, so the file's `overlay`/`baselayer` never reaches the client, and it reads a non-standard `scale` key, defaulting to 250000 [S17, `src/charts.ts` lines 205-227].

## 2. OpenCPN

**Chart bar and piano.** The chart bar is "the primary interface for selecting nautical charts during navigation". Each key is a chart at the current position, coloured by type: blue for raster, green for ENC, brown/yellow for CM93, pink/purple for MBTiles. In quilt mode keys are ordered "largest scale to the left, smallest to the right". In single-chart mode hovering shows a thumbnail and a click switches to that chart; in quilt mode hovering highlights the chart's coverage in "transparent reddish" and a click sets the reference scale. Ctrl+B hides or shows the bar [S13].

**Quilting, as documented.** The quilt reference chart is the left-most, largest-scale chart highlighted on the bar; smaller-scale charts fill the viewport where it does not reach; clicking a key one to the right lowers the reference scale without moving the view; zooming moves the reference left or right. "Disable Fullscreen Quilting" restricts the quilt to charts overlapping the screen centre. Raster and vector charts are quilted separately, not together. Since 4.2 Mercator, transverse Mercator, polyconic and skewed raster charts can share a quilt, but skewed quilting "requires OpenGL". A chart can be dropped from the quilt with "Hide This Chart" and restored with "Add this chart to quilt" [S12]. Q toggles quilting [S12, S15].

**Quilting, as implemented** (`quilt.cpp` [S10]):

- Each candidate chart gets a band of on-screen scales. For raster and vector charts the most-zoomed-in limit is nominal scale / 4, so up to 4x overzoom (`GetNomScaleMax`); the zoomed-out limit is nominal x a user "zoom modifier" for raster, nominal x 4 x modifier for vector, and nominal x 4 for MBTiles (`GetNomScaleMin`). The largest-scale chart gets unlimited overzoom, and the smallest-scale raster or MBTiles chart gets 24x more underzoom so the user can always zoom out.
- The bands are then widened so each overlaps the next smaller-scale chart's band ("so as to make no 'holes' in the coverage"), capped at twice the original. Remaining holes are closed by extending smaller-scale charts' bands (`AdjustRefOnZoom`).
- The reference chart is "the largest scale chart whose scale limits contain the requested scale", with "5 percent leeway to allow for roundoff errors".
- When zooming out without OpenGL, only charts containing the viewport centre may become the reference; zooming in, vector charts, or OpenGL allow any chart on screen.
- A chart must share the quilt's projection and have skew under 1 degree unless the relevant options are on (`IsChartQuiltableRef`, `GetNewRefChart`).
- If the only candidates are MBTiles, the quilt is built from MBTiles ("mbtiles charts only set"). An MBTiles file whose path contains `basemap` is shown only when the basemap option is enabled.

**MBTiles.** The manual says MBTiles are drawn as raster overlays that "do not ever participate in the RNC/ENC chart quilt composition", that several can be selected at once, that a left-click on their key toggles them, and that sets over 5 GB are deselected at startup [S13]. In the renderer, a file whose `type` is `overlay` draws only the tiles at the current view zoom; any other file draws every zoom from its minimum up to the view zoom, so lower zooms fill gaps in higher ones [S9, lines 919-921]. The source's MBTiles-only quilt path suggests the manual's "never" is older than the code **[unverified which version changed this]**.

**Chart groups.** A group is a named subset of chart directories or individual charts, defined under Options > Charts > Chart Groups; "All Charts" always exists and cannot be edited. Groups are views, not copies. Keys 1-9 select the first groups and 0 selects "All Charts" from 4.8.0. The manual suggests an empty group to show only the background map [S14]. The manual both says individual charts can be added to a group and, in its FAQ, that they must be organised into folders [S14].

**Over- and underzoom.** The canvas embosses an "OverZoom" label when the reference chart is shown more than 3.9x beyond its scale; for MBTiles it uses the file's own zoom factor instead [S11, `EmbossOverzoomIndicator`]. Changelogs record a selectable "overzoom emphasis" and a blur that fades charts out between 10x and 30x overzoom [S16] **[secondary: changelog mirror]**. No underzoom indicator was found.

**User complaints.** The OpenCPN FAQ answers a user whose KAP chart "reverts" on zoom out: "the chart will change per zoom..the which is fine unless you want to stay on a particular chart", and the fix is "Turn quilting off (keyboard "q")" or chart groups [S15]. Forum threads repeat the same advice **[secondary]**.

## 3. Freeboard-SK and Signal K

- **Chart list as layer stack.** Freeboard's help: "Check the checkbox to display the chart", so any number show at once. "Charts you have not re-ordered are stacked in descending order of scale, so that more detailed charts are on top"; drag handles re-order them and "your chosen order is used instead" [S18].
- **Opacity** is a per-chart slider; an "In view" switch lists only charts covering the current view; a bounds switch draws every listed chart's rectangle on the map; a "constrain map zoom" button limits zoom to the selected charts' range [S18].
- **Overlays** have been folded into the chart list: "Overlays have moved to the Chart list" [S18]. Recent pull requests add a "near vessel" ordering, caption the list "Top" and "Base Layer", and put plugin image overlays above every chart; an open issue notes chart z-indices collide with weather layers from the 41st chart [S19] **[secondary: search excerpts of GitHub, not opened]**.
- **No automatic scale switching** is described; the help mentions only per-chart minimum zoom [S18].
- **MBTiles `type`** is not used, because the charts plugin replaces it with `tilelayer` before Freeboard sees it (section 1) [S17].

## 4. Other plotters

- **AvNav** (Raspberry Pi). Its chart guide describes one chart built as 3-5 zoom "layers" like a paper portfolio (overview 7-10, navigation 10-15, detail 16-18); where a detailed layer covers an area it is shown, otherwise the lower layer is shown "possibly enlarged" [S20]. Overlays (other charts, GPX, KML, GeoJSON) are an ordered list with opacity 0-1; placing one before the chart row draws it beneath; each chart has its own overlay list plus "default overlays" for all charts; a layers button on the navigation page toggles them for the current session and device [S21]. The user still selects one chart at a time [S21].
- **Garmin.** Satellite photos are a chart setting: "Land Only" keeps "standard chart information on the water, with photos overlaying the land"; "Photo Map Blend" shows photos over land and water at a slider-set opacity. Imagery needs premium charts [S22].
- **Raymarine LightHouse 3.** "Aerial overlay" is on or off per cartography vendor, with "Visibility" 0-100% and a "Coverage" setting [S23] **[secondary: search excerpt of the official manual PDF]**. Older E-Series offered "On Land" or "On Land and Sea" [S23] **[secondary]**.
- **qtVlm.** Loading a folder quilts its charts automatically and picks "the chart that has a native scale closest to the one you have"; "Show only this chart" gives single-chart mode; toolbar buttons toggle raster, vector and MBTiles quilting separately; an option puts rasters over or under MBTiles; a lock button fixes the zoom level to stop automatic chart switching [S26] **[secondary: search excerpts of the qtVlm documentation and third-party guides]**.
- **SEAiq.** Shows installed charts as "light magenta outlines" and opens a chart once zoomed in far enough; raster charts are off by default [S24]. Its product page says it quilts "selecting the best available chart data for the scale you are viewing at" [S24] **[secondary: search excerpt]**.
- **iNavX.** Its +/- buttons offer a menu of the next larger or smaller scale charts, with an option to auto-select the best-detail chart; raster quilting came later via a tiled NOAA product [S25] **[secondary: reviews; the official help page returned 404]**.
- **Navionics, Aqua Map, OpenPlotter.** Not researched to primary sources. OpenPlotter ships OpenCPN and Freeboard-SK, so sections 2 and 3 apply **[unverified]**.

## 5. Safety guidance

- **Overscale and larger-scale indications.** IMO's ECDIS standard MSC.232(82), section 6.1, requires an indication if "the information is displayed at a larger scale than that contained in the ENC" or if "own ship's position is covered by an ENC at a larger scale than that provided by the display". Appendix 5 names these "Information overscale" and "Larger scale ENC available"; the raster (RCDS) table has "Larger scale information available, or overscale" and "Larger scale RNC available for the area of the vessel" [S27].
- **Largest scale for safety checks.** Clause 11.2 says "the largest scale data available ... should always be used" for safety-contour and prohibited-area alarms, whatever scale is on screen [S27]. The superseded A.817(19), clause 3.4, said a chart should first be displayed "at the largest scale available" [S28] **[secondary: search excerpt]**.
- **What must always show.** The standard display includes "chart scale boundaries"; RCDS must show the chart's own scale and "the scale at which it is displayed" [S27].
- **Other information.** Clause 7.1: other navigational information "should not degrade the displayed SENC information and it should be clearly distinguishable from the SENC information". Clause 7.2: it must be removable "by single operator action" [S27]. This is the nearest official text to the imagery-versus-chart question.
- **Raster mode limits.** RCDS mode requires "an appropriate portfolio of up-to-date paper charts" on board, and a continuous indication if the displayed raster chart is not on WGS 84 [S27].
- **S-52 overscale pattern.** The display portion compiled from a smaller-scale chart must carry the AP(OVERSC01) pattern when it is "overscale by X2 or more"; the larger-scale part gets only the overscale indication. When the mariner zooms on charts of one scale, only the indication is shown [S29] **[secondary: search excerpts of S-52 6.1.1 and PresLib 4.0]**. An IHO working paper reports manufacturers implement the pattern inconsistently [S30] **[secondary]**.
- **Satellite imagery.** No IMO, IHO or national-office text was found forbidding imagery for pilotage. An IHO review article urges using more satellite-derived data on charts [S31] **[secondary]**. A widely used imagery-chart project warns that reefs may be "'missing' or painted out" and that a "fuzzy expanse" may not be empty [S32] **[secondary]**. No casualty report was found that blamed imagery; a 2018 MAIB yacht grounding report uses imagery only as a figure backdrop [S33] **[secondary]**.
- **Mixing raster sources.** No IHO wording on blending raster sources was found beyond the RCDS clauses above.

## 6. Mechanisms worth borrowing

- **Map each chart to a nominal scale.** OpenCPN derives an MBTiles scale from `maxzoom` [S9]; Signal K uses an optional `scale` key [S17]. For our files `maxzoom` is the only dependable input.
- **Give each chart a usable band, not a point.** OpenCPN allows up to 4x overzoom and a few times underzoom per chart, widens bands to overlap, and picks the largest scale whose band contains the display scale [S10]. The overlap already damps flicker; no explicit time-based hysteresis was found in the code read.
- **Choose by view centre, not whole view, at least when zooming out.** OpenCPN requires the reference to contain the viewport centre unless it can afford full-screen selection [S10]; "Disable Fullscreen Quilting" makes centre-only the rule [S12].
- **Fill gaps from smaller scales.** OpenCPN's quilt and its base MBTiles renderer both draw lower-detail data where the preferred chart has none [S9, S10]; AvNav shows the lower layer "possibly enlarged" [S20].
- **Always say what is on screen.** Chart bar with a highlighted reference chart and coverage highlight on hover [S13]; "OverZoom" emboss above about 4x [S11]; IMO requires overscale and larger-scale-available indications [S27].
- **Always offer an escape from automation.** Single-chart mode (Q), hide-from-quilt, groups, qtVlm's level lock [S12, S14, S15, S26].
- **Keep imagery a separate, removable layer.** Garmin "Land Only" or "Blend" with opacity [S22]; Raymarine visibility slider [S23]; Freeboard and AvNav per-layer opacity [S18, S21]; IMO 7.1-7.2 [S27].

## Patterns that recur

1. Charts ordered by scale, most detailed first, wherever a list is shown (OpenCPN piano, Freeboard default stack).
2. Automatic choice of the largest usable scale, with a manual override always one action away.
3. Imagery treated as an optional layer with opacity, not as a chart in the same stack.
4. Groups or folders as the way to restrict automation to a chosen set of charts.
5. Coverage outlines and an overzoom warning so the user knows what they are looking at.
6. MBTiles metadata treated as a hint and repaired from the tiles table when wrong.

## Sources

- [S1] MBTiles 1.3 specification. https://github.com/mapbox/mbtiles-spec/blob/master/1.3/spec.md
- [S2] MBTiles 1.1 specification. https://github.com/mapbox/mbtiles-spec/blob/master/1.1/spec.md
- [S3] GDAL MBTiles raster driver, creation options. https://gdal.org/en/stable/drivers/raster/mbtiles.html
- [S4] QGIS `src/analysis/processing/qgsalgorithmxyztiles.cpp` lines 1035-1043. https://github.com/qgis/QGIS
- [S5] tippecanoe `mbtiles.cpp`, `make_metadata` and metadata writes. https://github.com/felt/tippecanoe
- [S6] SAS.Planet `Src/RegionProcess/Export/MBTiles/u_StorageExportToMBTiles.pas` lines 205-210, 459-479. https://github.com/sasgis/sas.planet.src
- [S7] MOBAC bug #348, sample metadata. https://sourceforge.net/p/mobac/bugs/348/ ; MBTiles.java mirror. https://www.javatips.net/api/mobac-ext-master/src/main/java/mobac/program/atlascreators/MBTiles.java
- [S8] SAS.Planet Mantis #3682, #3683, #3847. https://www.sasgis.org/mantis/view.php?id=3683
- [S9] OpenCPN `gui/src/mbtiles/mbtiles.cpp` at 72ae5b8. https://github.com/OpenCPN/OpenCPN
- [S10] OpenCPN `gui/src/quilt.cpp` at 72ae5b8: `IsChartQuiltableRef`, `GetNewRefChart`, `GetNomScaleMax/Min`, `AdjustRefOnZoom`.
- [S11] OpenCPN `gui/src/chcanv.cpp` at 72ae5b8, `EmbossOverzoomIndicator`.
- [S12] OpenCPN manual, Chart Quilting. https://opencpn.org/wiki/dokuwiki/doku.php?id=opencpn:manual_advanced:charts:quilting
- [S13] OpenCPN manual, Chart Bar. https://opencpn.org/wiki/dokuwiki/doku.php?id=opencpn:manual_basic:ui_user_interface:chart_bar
- [S14] OpenCPN manual, Chart Groups. https://opencpn.org/wiki/dokuwiki/doku.php?id=opencpn:manual_basic:set_options:charts:chart_groups
- [S15] OpenCPN FAQ, Chart Questions. https://opencpn.org/wiki/dokuwiki/doku.php?id=opencpn:manual_basic:faq:charts-trouble
- [S16] OpenCPN changelog mirror (Shoreline). http://opencpn.shoreline.fr/
- [S17] @signalk/charts-plugin `src/charts.ts`. https://github.com/SignalK/charts-plugin
- [S18] Freeboard-SK help. https://demo.signalk.org/@signalk/freeboard-sk/assets/help/index.html
- [S19] Freeboard-SK PRs #805, #906, #1011 and issue #1015. https://github.com/SignalK/freeboard-sk
- [S20] AvNav, Karten und Overlays. https://www.wellenvogel.net/software/avnav/docs/charts.html
- [S21] AvNav, Overlays. https://www.wellenvogel.net/software/avnav/docs/hints/overlays.html
- [S22] Garmin echoMAP CHIRP manual, Showing Satellite Imagery on the Navigation Chart. https://www8.garmin.com/manuals/webhelp/echomapchirp/EN-US/GUID-7F33FA14-39F6-4E99-8BA5-35B02668AFE0.html
- [S23] Raymarine LightHouse 3 operation instructions 81406. https://productimageserver.com/literature/ownersManual/85068OM.pdf
- [S24] SEAiq Charts help. http://doc.seaiq.com/ChartHelp.html ; https://seaiq.com/recreational.html
- [S25] iNavX review. https://gpstracklog.com/2012/07/inavx-marine-navigation-for-ios-review.html
- [S26] qtVlm documentation 5.7.3. http://visitmyharbour.download.s3.amazonaws.com/qtVlm_documentation_en.pdf ; Starpath cheat sheet https://www.starpath.com/qtVlm/
- [S27] IMO Resolution MSC.232(82), Revised performance standards for ECDIS, 2006: clauses 5.7, 6.1, 7.1-7.2, 11.2, Appendices 2, 5, 7. https://wwwcdn.imo.org/localresources/en/KnowledgeCentre/IndexofIMOResolutions/MSCResolutions/MSC.232(82).pdf
- [S28] IMO Resolution A.817(19). https://puc.overheid.nl/nsi/doc/PUC_2433_14/3/
- [S29] IHO S-52 Edition 6.1.1. https://iho.int/uploads/user/pubs/standards/s-52/S-52%20Edition%206.1.1%20-%20June%202015.pdf
- [S30] IHO ENCWG4 2019, Overscale area pattern performance in ECDIS. https://iho.int/uploads/user/Services%20and%20Standards/ENCWG/ENCWG4/ENCWG4_2019_5.26_EN_v1_Overscale%20area%20pattern%20performance%20in%20ECDIS_FINAL.pdf
- [S31] IHO International Hydrographic Review, satellite imagery in the Canadian Arctic. https://ihr.iho.int/articles/satellite-imagery-for-hazard-detection-better-use-of-satellite-imagery-could-help-prevent-groundings-in-the-canadian-arctic/
- [S32] Noonsite, Using OpenCPN with satellite imagery. https://www.noonsite.com/report/navigation-using-opencpn-with-satellite-imagery/
- [S33] MAIB report 12/2018, CV24. https://assets.publishing.service.gov.uk/media/5b30ee7d40f0b67f8511a514/2018_-_12_-_CV24.pdf
