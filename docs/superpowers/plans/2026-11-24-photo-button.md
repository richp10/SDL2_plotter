# Photo Button Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A thumbnail button that appears only where a satellite photo of the screen centre exists, and steps chart → each photo source → chart on a tap.

**Architecture:** The catalogue answers "which photo sources cover this spot" as a pure function; the session turns that into a button description and a `NextPhoto` action (plus dropping back to the chart when the photo's coverage is left); a thumbnail cache in the renderer module cuts a tile into a 72 px image; the shell draws the button after the stream capture and routes taps to it; the Charts menu labels imagery layers "(not here)" when it opens.

**Tech Stack:** Rust 2021, SDL2 0.38 (`gfx`, `ttf`), `rusqlite` 0.29 bundled, `image` 0.24, `lru` 0.10.

**Spec:** `docs/photo-button-design.md` (read it first; decision numbers below refer to it). Background: `docs/chart-catalogue-design.md`, `CLAUDE.md`.

## Global Constraints

- Quality gate before every commit, from the repo root, in Git Bash with `export PATH="$HOME/.cargo/bin:$PATH"`: `cargo fmt -- --check` prints nothing; `cargo build 2>&1 | grep -c '^warning'` prints 0; `cargo test` all pass; `cargo clippy --all-targets 2>&1 | grep -c '^warning'` prints 0.
- Never commit with a red test. One commit per task. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Log messages (`crate::diagnostics::record`) in French; identifiers, comments and on-screen text in English.
- Tests write only under `std::env::temp_dir()`; temp names from `crate::mbtiles_renderer::test_support::unique_name()`.
- No `#[allow(dead_code)]`. A pub item only tests use gets `#[cfg(test)]` until a later task uses it.
- Nothing in the 16 ms loop may take more than a millisecond or two per frame in steady state; a thumbnail is decoded at most once per (file, tile) key.
- `MAX_PHOTO_OVERSCALE = 2` (decision 1). Button 72 px square (decision 2). Imagery is never chosen without a tap, key press or menu choice (decisions 3, 4).
- Kill a running `plotter.exe` only if it was started by you; the owner's own running copy locks the release exe, so ask before a release build if one is running.

## Review Focus

- **The overscale boundary:** a photo 2 levels past its deepest level is offered, 3 levels past is not. Pinned in Task 1.
- **Leaving and re-entering coverage while a photo shows:** after drifting out the chart layer is chosen, and drifting back does not bring the photo back. Pinned in Task 2.
- **A hidden imagery file, or a layer whose files are all hidden:** never offered by the button or counted in "k/n". Pinned in Task 1.
- **A tap with the chart locked:** the lock is released and the photo shows. Pinned in Task 2.
- **A source that covers the spot but has no tile under the centre:** the button still shows, with the icon. Pinned in Tasks 3 (`None` thumbnail) and 4 (icon drawn when the thumbnail is `None`).

---

### Task 1: Photo sources available here (catalogue)

**Files:**
- Modify: `src/catalogue.rs` (new method and constant beside `choose`; tests in its `tests` module)

**Interfaces:**
- Consumes: existing private `Catalogue::best_in(&self, layer: &str, centre: (f64, f64), zoom: i32) -> Option<&ChartFile>`, `visible()`, `role_of()`.
- Produces:
  - `pub const MAX_PHOTO_OVERSCALE: i32 = 2;`
  - `pub fn photo_sources(&self, centre: (f64, f64), zoom: i32, preferred: Option<&str>) -> Vec<&ChartFile>` — one file per available `Role::Imagery` layer (that layer's `best_in` file, kept only if `zoom - top_level <= MAX_PHOTO_OVERSCALE`); the layer named `preferred` first, then layers in the order their first visible file appears in `files()`.

- [ ] **Step 1: Write the failing tests** in `catalogue::tests`, reusing the `charts()` fixture, `AZORES` and `MID_ATLANTIC` and adding a Google file:

```rust
/// `charts()` plus Google imagery of the Azores (z7-17).
fn photo_charts() -> Charts {
    let mut c = charts();
    let zooms: Vec<(i32, i64, i64)> = (7..=17).map(|z| (z, 0, 0)).collect();
    chart(&c.dir, "Azores_GoogleSat.mbtiles", &zooms, &[("bounds", "-34,36,-22,41")]);
    c.catalogue = Catalogue::load(&c.dir, &c.dir.join("charts.json"));
    c
}

#[test]
fn photo_sources_are_the_imagery_layers_covering_the_centre() {
    let c = photo_charts();
    let layers = |v: Vec<&ChartFile>| v.iter().map(|f| f.layer.clone()).collect::<Vec<_>>();
    assert_eq!(layers(c.catalogue.photo_sources(AZORES, 9, None)), vec!["Bing satellite", "Google satellite"]);
    assert_eq!(layers(c.catalogue.photo_sources(AZORES, 9, Some("Google satellite"))), vec!["Google satellite", "Bing satellite"]);
    assert!(c.catalogue.photo_sources(MID_ATLANTIC, 9, None).is_empty(), "no photo there");
}

#[test]
fn a_photo_is_offered_up_to_two_levels_past_its_deepest_and_not_three() {
    let c = photo_charts();
    assert_eq!(c.catalogue.photo_sources(AZORES, 19, None).len(), 2);
    assert!(c.catalogue.photo_sources(AZORES, 20, None).is_empty());
}

#[test]
fn hidden_photo_files_are_never_offered() {
    let mut c = photo_charts();
    c.catalogue.files.iter_mut().filter(|f| f.layer == "Bing satellite").for_each(|f| f.hidden = true);
    let offered: Vec<String> = c.catalogue.photo_sources(AZORES, 9, None).iter().map(|f| f.layer.clone()).collect();
    assert_eq!(offered, vec!["Google satellite"]);
}
```

- [ ] **Step 2: Run** `cargo test catalogue::tests::photo` and `cargo test catalogue::tests::a_photo` and `cargo test catalogue::tests::hidden_photo`. Expected: compile error, `photo_sources` not found.

- [ ] **Step 3: Implement** `MAX_PHOTO_OVERSCALE` and `photo_sources` in `src/catalogue.rs` with the interface above. Layer order: iterate `self.visible()`, collecting each `Role::Imagery` layer name the first time it is seen; move `preferred` to the front if present.

- [ ] **Step 4: Run** `cargo test catalogue`. Expected: all pass. Run the full quality gate; `photo_sources` is unused outside tests until Task 2, so mark it and the constant `#[cfg(test)]` for this commit if the build warns, and remove that attribute in Task 2.

- [ ] **Step 5: Commit** `src/catalogue.rs` with message `Photo sources available at the screen centre`.

---

### Task 2: The session steps through photos and drops back on leaving coverage

**Files:**
- Modify: `src/action.rs` (rename `ToggleImagery` to `NextPhoto`, add `BackToChart`; `name()` strings `"NextPhoto"`, `"BackToChart"`)
- Modify: `src/app/actions.rs` (replace the `ToggleImagery` arm; add `BackToChart`; tests)
- Modify: `src/app/session.rs` (`photo_button()`, `PhotoButton`, drop-back in `show_chosen_chart`; test support)
- Modify: `src/mbtiles_renderer/tile_store.rs` test support (`fixture_chart_bounded`)
- Modify: `src/app/input.rs` (key I → `NextPhoto`; PHOTO badge tap → `BackToChart`), `src/topbar.rs` (menu entry label "Next photo" → `NextPhoto`; update its test)

**Interfaces:**
- Consumes: `Catalogue::photo_sources`, `MAX_PHOTO_OVERSCALE` (Task 1); existing `Session::choose_layer(&mut self, Option<String>)`, `chart_choice()`, `state.chart_layer`, `state.imagery_layer`, `renderer.display_zoom()`.
- Produces:
  - `AppAction::NextPhoto`, `AppAction::BackToChart`.
  - In `session.rs`: `pub struct PhotoButton { pub preview: Option<PathBuf>, pub preview_is_photo: bool, pub position: Option<(usize, usize)> }` — `preview` is the file whose tile previews where a tap goes (next source's file, or the chart layer's file that `choose` would show; `None` if neither exists), `position` is `Some((k, n))` (1-based) while a photo is shown.
  - `pub fn photo_button(&self) -> Option<PhotoButton>` — `None` when no source is available here and no photo is shown.
  - Test support: `fixture_chart_bounded(zooms: &[i32], bounds: &str) -> PathBuf` (as `fixture_chart` plus a `bounds` metadata row) and `session_with_bounded_charts(charts: &[(&str, &[i32], &str)]) -> Fixture`.

Behaviour of `NextPhoto` (decision 3): sources = `photo_sources(centre, drawn zoom, state.imagery_layer)`. Showing a chart → choose `sources[0]`'s layer (if none, log INFO `"Aucune photo ici"` and change nothing). Showing source k of n → source k+1, or after the last → `choose_layer(state.chart_layer.clone())`. `BackToChart` → `choose_layer(state.chart_layer.clone())`. Drop-back (decision 4): in `show_chosen_chart`, when `state.layer` is an imagery layer and the choice has `fallback_from == Some(that layer)`, call `choose_layer(state.chart_layer.clone())` before switching files and log INFO `"Photo hors couverture, retour à la carte"`.

- [ ] **Step 1: Write the failing tests** in `app::actions::tests`, using `session_with_bounded_charts` with an ocean chart `("Ocean_Navionics.mbtiles", &[1, 2, 3], "-180,-85,180,85")`, `("Isle_BingSat.mbtiles", &[1, 2, 3], "0,0,40,40")` and `("Isle_GoogleSat.mbtiles", &[1, 2, 3], "0,0,40,40")`, view at `(20.0, 20.0)` zoom 3, `follow = false`, layer `"Navionics"`; use the existing `settle` helper after zoom changes:
  - `next_photo_steps_through_the_sources_here_and_back_to_the_chart`: `photo_button()` is `Some` with `position == None` and `preview` ending `Isle_BingSat.mbtiles`; `NextPhoto` → renderer path ends `Isle_BingSat.mbtiles`, `photo_button().position == Some((1, 2))`, preview ends `Isle_GoogleSat.mbtiles`; `NextPhoto` → Google, `Some((2, 2))`, preview is the ocean chart with `preview_is_photo == false`; `NextPhoto` → layer `"Navionics"`.
  - `the_last_photo_used_comes_first`: after choosing Google then going back to the chart, `photo_button().preview` ends `Isle_GoogleSat.mbtiles`.
  - `no_button_and_no_change_where_no_photo_exists`: position `(-100.0, 20.0)` → `photo_button()` is `None`; `NextPhoto` leaves `state.layer == Some("Navionics")`.
  - `leaving_the_photos_coverage_drops_back_to_the_chart_for_good`: on Bing at `(20, 20)`; set position `(-100, 20)`, `sync_view()` → layer `"Navionics"`; set position back to `(20, 20)`, `sync_view()` → layer still `"Navionics"`, renderer path ends `Ocean_Navionics.mbtiles`.
  - `a_tap_releases_the_lock`: `ToggleLock`, then `NextPhoto` → `state.lock == None`, path ends `Isle_BingSat.mbtiles`.
  - `back_to_chart_returns_from_any_source`: on Google, `BackToChart` → layer `"Navionics"`.
  - Rename/replace the two existing tests that use `ToggleImagery` (`imagery_is_switched_on_and_off_by_hand_and_badged_while_shown`, `without_imagery_files_the_imagery_toggle_changes_nothing`) so they use `NextPhoto` with the same expectations.

- [ ] **Step 2: Run** `cargo test app::actions`. Expected: compile errors for `NextPhoto`, `BackToChart`, `photo_button`, `session_with_bounded_charts`.

- [ ] **Step 3: Implement** the action variants, `fixture_chart_bounded`/`session_with_bounded_charts`, `PhotoButton`/`photo_button`, the `NextPhoto`/`BackToChart` arms, and the drop-back, per the Interfaces and Behaviour above. Update `input.rs` (I key, badge tap) and `topbar.rs` (entry `"Next photo"` → `NextPhoto`, its test expects `NextPhoto`). Remove any `#[cfg(test)]` added in Task 1.

- [ ] **Step 4: Run** `cargo test`, then the quality gate. Expected: all pass, 0 warnings.

- [ ] **Step 5: Commit** with message `Step through the photos available here; drop back to the chart outside them`.

---

### Task 3: Thumbnails

**Files:**
- Create: `src/mbtiles_renderer/thumbnail.rs`
- Modify: `src/mbtiles_renderer.rs` (`mod thumbnail; pub use thumbnail::Thumbnails;`), `src/mbtiles_renderer/tile_store.rs` (make `fetch_tile_bytes` and `decode_tile` `pub(super)` if they are private)

**Interfaces:**
- Consumes: `tile_store::fetch_tile_bytes(&Connection, z, x, y)`, `tile_store::decode_tile(Option<Vec<u8>>) -> CachedTile`, `projection::lonlat_to_world` / `TILE_SIZE`.
- Produces: `pub struct Thumbnails` with
  - `pub fn new() -> Self`
  - `pub fn get(&mut self, path: &Path, centre: (f64, f64), zoom: i32, size: u32) -> Option<Arc<RgbaImage>>` — the tile of `path` containing `centre` at `zoom` clamped to that file's levels (its deepest level at or below `zoom`, else its first level), scaled to `size`×`size` with `imageops::FilterType::Triangle`; `None` if the file cannot be opened or has no tile there. Cached by `(path, level, x, y)` in an `LruCache` of 32 entries; the last 4 opened connections kept, opened read-only.

- [ ] **Step 1: Write the failing tests** in `thumbnail.rs`:
  - `a_thumbnail_is_the_tile_under_the_centre_scaled_down`: `fixture_chart_with(&[2], Rgba([10, 200, 30, 255]))`, `get(path, (5.0, 5.0), 2, 72)` → `Some`, 72×72, pixel (36, 36) equals `[10, 200, 30, 255]`.
  - `the_same_tile_is_not_cut_twice`: two calls with centres `(5.0, 5.0)` and `(6.0, 6.0)` (same zoom-2 tile) return `Arc::ptr_eq` images.
  - `no_tile_under_the_centre_gives_none`: the fixture with `DELETE FROM tiles WHERE tile_column != 0` (via `rusqlite::Connection`), centre `(100.0, 5.0)` → `None`; a missing path → `None`.
  - `a_zoom_past_the_files_levels_uses_its_deepest_level`: fixture `&[1, 2]`, zoom 9 → `Some`.

- [ ] **Step 2: Run** `cargo test thumbnail`. Expected: compile error, `Thumbnails` not found.

- [ ] **Step 3: Implement** `Thumbnails` per the interface. Rows are TMS: `fetch_tile_bytes` already flips; pass the XYZ row.

- [ ] **Step 4: Run** `cargo test thumbnail`, then the quality gate (mark `Thumbnails` items `#[cfg(test)]` only if the build warns; Task 4 uses them). No benchmark needed: the render path is untouched.

- [ ] **Step 5: Commit** with message `Thumbnail of the tile under the screen centre, cached per tile`.

---

### Task 4: Draw the button and route taps to it

**Files:**
- Create: `src/app/photo_button.rs`
- Modify: `src/app.rs` (`mod photo_button;`, a `thumbnails: Thumbnails` field and a `photo_button: photo_button::PhotoButtonView<'ttf>` field), `src/app/render.rs` (draw after the stream capture; shared `NIGHT_OPACITY`), `src/app/input.rs` (tap → `NextPhoto`), `src/screen.rs` (`pub fn night_rect(&mut self, area: Rect, opacity: u8) -> Result<(), String>`)

**Interfaces:**
- Consumes: `Session::photo_button() -> Option<PhotoButton>` (Task 2), `Thumbnails::get` (Task 3), `renderer.display_zoom()`, the view centre (`renderer.view().panned_center(0, 0)`), `Buttombar` height 32 px.
- Produces:
  - `pub(super) fn area(screen_width: u32, screen_height: u32) -> Rect` in `photo_button.rs`: `Rect::new(10, screen_height as i32 - 32 - 10 - 72, 72, 72)`.
  - `pub(super) struct PhotoButtonView<'ttf>` with `new(ttf_context, width, height, fonts) -> Result<Self, String>` and `show(&mut self, canvas: &mut WindowCanvas, button: &PhotoButton, thumbnail: Option<&RgbaImage>)`: thumbnail (or a dark grey square with "PHOTO" / "CHART" centred when `None`), 2 px white border, "k/n" in the bottom-right corner on a dark backing when `position` is `Some`.
  - `pub(super) const NIGHT_OPACITY: u8 = 0;` in `render.rs`, used by both `screen.night_layer(NIGHT_OPACITY)` and `screen.night_rect(area, NIGHT_OPACITY)`.

- [ ] **Step 1: Write the failing tests** in `photo_button.rs`:
  - `the_button_sits_bottom_left_above_the_status_bar`: for 1600×800, `area` is `x == 10`, `width == 72`, `bottom() <= 800 - 32`, `y > 800 / 2`; for 800×480 it still fits above the bar.

- [ ] **Step 2: Run** `cargo test photo_button`. Expected: compile error.

- [ ] **Step 3: Implement** `area`, `PhotoButtonView`, `night_rect` and the wiring: in `display`, after the stream capture and before the top bar, if `session.photo_button()` is `Some(b)`, get the thumbnail of `b.preview` at the view centre and drawn zoom (rounded) with size 72, `show` it, then `night_rect(area, NIGHT_OPACITY)`. Upload the thumbnail through a streaming texture cached by `Arc` pointer so an unchanged thumbnail is not re-uploaded. In `handle_mouse_down`, before the PHOTO badge check: a press inside `area` while `photo_button()` is `Some` executes `NextPhoto` and starts no gesture.

- [ ] **Step 4: Run** `cargo test`, the quality gate, then a release run if no owner copy is running: `cargo build --release`, start `./target/release/plotter.exe`, close it after a few seconds, and check the new lines of `plotter.log` contain no `WARN`/`ERROR`/`PANIC`. Restore `default/state.json` from a copy taken before the run.

- [ ] **Step 5: Commit** with message `Photo button: a thumbnail of the next source, bottom left, not streamed`.

---

### Task 5: "(not here)" in the Charts menu, and the docs

**Files:**
- Modify: `src/app/session.rs` (`pub fn layer_labels(&self) -> Vec<String>`), `src/app.rs` (remove the free `layer_labels`; refresh the menu when it is about to open), `src/app/input.rs`, `CLAUDE.md`, `docs/wayfinder/chart-catalogue.md`

**Interfaces:**
- Consumes: `Catalogue::switchable_layers()`, `Catalogue::photo_sources` (Task 1), `Topbar::set_layers(Vec<String>)`, `Topbar::is_open()`.
- Produces: `Session::layer_labels() -> Vec<String>`: per switchable layer, `name` for a chart layer, `"{name} (photo)"` for an imagery layer available here, `"{name} (photo, not here)"` for one that is not.

- [ ] **Step 1: Write the failing test** in `app::actions::tests` (or a new `session` test) with the Task 2 bounded fixture: at `(20, 20)` zoom 3, `layer_labels() == ["Navionics", "Bing satellite (photo)", "Google satellite (photo)"]`; at `(-100, 20)`, the two photo labels end with `"(photo, not here)"`.

- [ ] **Step 2: Run** the test. Expected: compile error, `layer_labels` not found on `Session`.

- [ ] **Step 3: Implement** `Session::layer_labels`; in `handle_mouse_down`, when the top bar is closed and the press is on the bar (`y < 32`), call `self.topbar.set_layers(self.session.layer_labels())` before passing the press on; `close_chart_manager` uses `self.session.layer_labels()` too.

- [ ] **Step 4: Update the docs.** `CLAUDE.md`: in the catalogue row, add `photo_sources` and `MAX_PHOTO_OVERSCALE`; in the zoom/layer gotcha, replace the `ToggleImagery` sentence with `NextPhoto` (chart → each source here → chart; last used first), `BackToChart` (badge), the drop-back on leaving coverage, and the photo button (bottom left, thumbnail of where a tap goes, drawn after the stream capture). `docs/wayfinder/chart-catalogue.md`: add ticket `C7 — Photo button` (status closed, a resolution of what was built and the test counts) and its line under Decisions so far.

- [ ] **Step 5: Run** `cargo test` and the quality gate. Expected: all pass, 0 warnings.

- [ ] **Step 6: Commit** with message `Mark photo layers not available here in the Charts menu; document the photo button`.
