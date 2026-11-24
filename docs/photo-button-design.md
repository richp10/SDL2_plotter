# Photo button: design

Agreed 2026-11-24 in a grilling session, extending
[the chart catalogue design](chart-catalogue-design.md). Plan:
[docs/superpowers/plans/2026-11-24-photo-button.md](superpowers/plans/2026-11-24-photo-button.md).

## Why

The Charts menu lists every imagery layer wherever the boat is. Over the UK,
where no satellite file reaches, choosing one silently falls back to the
chart and only a line in the bottom bar says why. The navigator cannot tell
from the screen whether a photo of this spot exists.

## Decisions

1. **Available here.** An imagery layer is available when one of its visible
   files covers the screen centre at the zoom being drawn: its levels include
   that zoom, or the zoom is past its deepest level by at most 2 levels
   (`MAX_PHOTO_OVERSCALE = 2`). The same coverage and level rule as the
   switching rule; a photo blurrier than that is not offered.
2. **The button.** 72 px square, bottom left, just above the status bar, thin
   white border. It shows a thumbnail of where a tap goes next: the next
   photo source, or the chart after the last source. The thumbnail is a whole
   tile of that file under the screen centre, scaled down, refreshed only
   when the centre moves onto another tile or zoom level. If that file has no
   tile there, a plain "PHOTO" (or "CHART") icon instead. The button appears
   only while at least one photo source is available here, or while a photo
   is shown.
3. **A tap** (or the I key, or the menu) moves one step: chart, then each
   available source, then chart again. The source used last comes first,
   then the rest in the chart manager's file order. While a photo shows, a
   "k/n" label in the button's corner says which source is on. (The red
   PHOTO badge was removed after trying it, 2026-11-24: the photo is
   obviously not a chart and the button is the way back.)
4. **Leaving coverage.** When the shown photo's layer no longer covers the
   screen centre, the app drops back to the chart layer for good; the photo
   never reappears without a tap.
5. **Lock.** A tap releases a lock, as every layer change does.
6. **Charts menu.** Imagery layers not available here are labelled
   "(not here)", worked out when the menu opens; they can still be chosen.
   The menu's imagery entry becomes "Next photo".
7. **Stream.** The button is drawn after the stream capture, like the menus;
   the bottom bar, with the layer name, is in the stream.
8. **Night mode.** The button is dimmed with the same overlay and opacity as
   the chart. (Night mode currently passes opacity 0 everywhere; the button
   follows the same value.)
9. **Course-up.** The thumbnail stays north-up and unrotated.
