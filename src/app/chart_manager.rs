//! The chart manager window: every chart file with its display name,
//! layer, role, real zoom levels, shown or hidden, and order, changed with
//! taps. `Layout` is the geometry and hit testing, font-free and tested;
//! `ChartEdit` and `Session::edit_chart`/`locate_chart` are what a tap does,
//! tested with a fixture; `ChartManager` holds the open/scroll/typing state
//! and draws. The window is chrome: it is drawn after the stream capture.

use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::WindowCanvas;
use sdl2::ttf::{Font, Sdl2TtfContext};

use super::session::Session;
use crate::catalogue::{Catalogue, Role};
use crate::config::FontConfig;
use crate::screen::{draw_text, TextAlign};

const HEADER_HEIGHT: u32 = 48;
/// The row of column names under the title.
const COLUMN_HEADER_HEIGHT: u32 = 28;
/// Where the first row starts.
const ROWS_TOP: u32 = HEADER_HEIGHT + COLUMN_HEADER_HEIGHT;
const FOOTER_HEIGHT: u32 = 36;
const ROW_HEIGHT: u32 = 44;
const BUTTON_WIDTH: u32 = 100;
const MARGIN: i32 = 10;
const MIN_NAME_WIDTH: u32 = 120;
/// Fixed column widths; the name column takes the rest.
const UP: u32 = 50;
const DOWN: u32 = 50;
const LAYER: u32 = 220;
const ROLE: u32 = 110;
const ZOOMS: u32 = 100;
const SHOWN: u32 = 110;
const WHERE: u32 = 90;

const BACKGROUND: Color = Color::RGB(236, 240, 240);
const ROW_SHADE: Color = Color::RGB(222, 228, 228);
const LINE: Color = Color::RGB(150, 160, 160);
const NEW_MARK: Color = Color::RGB(255, 214, 102);
const SATELLITE: Color = Color::RGB(190, 30, 30);
const MAP: Color = Color::RGB(170, 105, 0);

/// Kinds told apart at a glance: the two without soundings stand out.
fn kind_colour(role: Role) -> Color {
    match role {
        Role::Chart => Color::BLACK,
        Role::Satellite => SATELLITE,
        Role::Map => MAP,
    }
}

/// A cell of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cell {
    Up,
    Down,
    Name,
    Layer,
    Role,
    Zooms,
    Shown,
    Where,
}

const CELLS: [(Cell, u32); 8] = [
    (Cell::Up, UP),
    (Cell::Down, DOWN),
    (Cell::Name, 0),
    (Cell::Layer, LAYER),
    (Cell::Role, ROLE),
    (Cell::Zooms, ZOOMS),
    (Cell::Shown, SHOWN),
    (Cell::Where, WHERE),
];

/// A column's name in the header row. Order spans both arrows, so the down
/// arrow's own is empty.
fn header(cell: Cell) -> &'static str {
    match cell {
        Cell::Up => "Order",
        Cell::Down => "",
        Cell::Name => "Name",
        Cell::Layer => "Source",
        Cell::Role => "Kind",
        Cell::Zooms => "Zooms",
        Cell::Shown => "Visible",
        Cell::Where => "Locate",
    }
}

/// What a tap on a Kind cell does.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum KindTap {
    /// Change the layer's kind now.
    Apply,
    /// Say this and wait for a second tap on the same file's Kind.
    Ask(String),
}

/// A tap on `file`'s Kind, with `waiting` the file whose change is already
/// asked about. A change into or out of Chart changes what automatic
/// switching uses, so it is asked about first.
pub(super) fn kind_tap(catalogue: &Catalogue, file: &str, waiting: Option<&str>) -> KindTap {
    let Some(chart) = catalogue.files().iter().find(|f| f.file_name == file) else {
        return KindTap::Apply;
    };
    let from = catalogue.role_of(&chart.layer);
    let to = from.next();
    if !from.needs_confirmation(to) || waiting == Some(file) {
        return KindTap::Apply;
    }
    let effect = if to == Role::Chart {
        "It will be used for automatic switching."
    } else {
        "It will no longer be used for automatic switching."
    };
    KindTap::Ask(format!(
        "Make {} a {}? {effect} Tap Kind again to confirm.",
        chart.layer,
        to.label()
    ))
}

/// What a tap landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    Close,
    PreviousPage,
    NextPage,
    /// Row `index` of the catalogue's files (not of the page).
    Row(usize, Cell),
}

/// The window's geometry for a screen size. Pure, so it is tested.
#[derive(Debug, Clone, Copy)]
pub(super) struct Layout {
    width: u32,
    height: u32,
}

impl Layout {
    pub fn new(width: u32, height: u32) -> Self {
        Layout { width, height }
    }

    pub fn rows_per_page(&self) -> usize {
        (self.height.saturating_sub(ROWS_TOP + FOOTER_HEIGHT) / ROW_HEIGHT).max(1) as usize
    }

    fn name_width(&self) -> u32 {
        let fixed: u32 = CELLS.iter().map(|(_, w)| w).sum::<u32>() + 2 * MARGIN as u32;
        self.width.saturating_sub(fixed).max(MIN_NAME_WIDTH)
    }

    /// Where `cell` of the row shown at `slot` (0 = top of the page) is.
    pub fn cell(&self, slot: usize, cell: Cell) -> Rect {
        let (x, w) = self.column(cell);
        let y = (ROWS_TOP + slot as u32 * ROW_HEIGHT) as i32;
        Rect::new(x, y, w, ROW_HEIGHT)
    }

    /// Where the name of `cell`'s column is written.
    pub fn column_header(&self, cell: Cell) -> Rect {
        let (x, w) = self.column(cell);
        Rect::new(x, HEADER_HEIGHT as i32, w, COLUMN_HEADER_HEIGHT)
    }

    /// The left edge and width of `cell`'s column.
    fn column(&self, cell: Cell) -> (i32, u32) {
        let mut x = MARGIN;
        for (c, w) in CELLS {
            let w = if c == Cell::Name {
                self.name_width()
            } else {
                w
            };
            if c == cell {
                return (x, w);
            }
            x += w as i32;
        }
        unreachable!("every cell is in CELLS")
    }

    fn button(&self, from_right: i32) -> Rect {
        let x = self.width as i32 - MARGIN - from_right * (BUTTON_WIDTH as i32 + MARGIN);
        Rect::new(x - BUTTON_WIDTH as i32, 6, BUTTON_WIDTH, HEADER_HEIGHT - 12)
    }

    pub fn close_button(&self) -> Rect {
        self.button(0)
    }

    pub fn next_button(&self) -> Rect {
        self.button(1)
    }

    pub fn previous_button(&self) -> Rect {
        self.button(2)
    }

    /// What is at `(x, y)` with the page starting at row `first` of
    /// `rows` rows.
    pub fn hit(&self, x: i32, y: i32, first: usize, rows: usize) -> Option<Hit> {
        let point = (x, y);
        if self.close_button().contains_point(point) {
            return Some(Hit::Close);
        }
        if self.next_button().contains_point(point) {
            return Some(Hit::NextPage);
        }
        if self.previous_button().contains_point(point) {
            return Some(Hit::PreviousPage);
        }
        if y < ROWS_TOP as i32 {
            return None;
        }
        let slot = ((y - ROWS_TOP as i32) / ROW_HEIGHT as i32) as usize;
        let index = first + slot;
        if slot >= self.rows_per_page() || index >= rows {
            return None;
        }
        CELLS
            .iter()
            .find(|(cell, _)| self.cell(slot, *cell).contains_point(point))
            .map(|(cell, _)| Hit::Row(index, *cell))
    }
}

/// What a tap on a row does to the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ChartEdit {
    Up,
    Down,
    NextLayer,
    CycleRole,
    ToggleHidden,
    Review,
    Rename(String),
}

impl Session {
    /// Applies `edit` to the chart file, saves the catalogue and shows the
    /// file that now fits the view.
    pub(super) fn edit_chart(&mut self, file: &str, edit: ChartEdit) {
        let catalogue = &mut self.catalogue;
        let changed = match edit {
            ChartEdit::Up => catalogue.move_file(file, true),
            ChartEdit::Down => catalogue.move_file(file, false),
            ChartEdit::NextLayer => catalogue.cycle_layer(file),
            ChartEdit::CycleRole => catalogue.cycle_role(file),
            ChartEdit::ToggleHidden => catalogue.toggle_hidden(file),
            ChartEdit::Review => catalogue.mark_reviewed(file),
            ChartEdit::Rename(name) => catalogue.rename(file, &name),
        };
        if !changed {
            return;
        }
        if let Err(error) = self.catalogue.save() {
            crate::diagnostics::record("ERROR", format!("Erreur sauvegarde du catalogue: {error}"));
        }
        // A layer may have gone (cycled away from, or emptied by hiding).
        if let Some(layer) = &self.state.layer {
            if !self
                .catalogue
                .switchable_layers()
                .iter()
                .any(|l| &l.name == layer)
            {
                self.state.layer = None;
            }
        }
        self.sync_view();
    }

    /// Shows where the file is: centres the chart on its coverage at its
    /// first level, in its layer, outlined until the manager opens again.
    pub(super) fn locate_chart(&mut self, file: &str) {
        let Some(chart) = self
            .catalogue
            .files()
            .iter()
            .find(|f| f.file_name == file)
            .cloned()
        else {
            return;
        };
        let b = chart.bounds;
        self.state.follow = false;
        self.state.map_offset = (0, 0);
        self.state.position = Some(b.centre());
        if !chart.hidden {
            self.choose_layer(Some(chart.layer.clone()));
        }
        if let Some(&first) = chart.zooms.first() {
            self.renderer.zoom_to(first);
            self.state.zoom = first;
        }
        self.highlight = Some(b);
        self.sync_view();
    }
}

/// The window's open/scroll/typing state, and its drawing.
pub(super) struct ChartManager<'ttf> {
    font: Font<'ttf, 'static>,
    layout: Layout,
    open: bool,
    /// First row shown.
    first: usize,
    /// The file whose name is being typed, and the text so far.
    pub typing: Option<(String, String)>,
    /// The file whose kind change waits for a second tap, and what the
    /// title line asks.
    pub confirm_kind: Option<(String, String)>,
}

impl<'ttf> ChartManager<'ttf> {
    pub fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        width: u32,
        height: u32,
        fonts: &FontConfig,
    ) -> Result<Self, String> {
        Ok(ChartManager {
            font: ttf_context.load_font(&fonts.file, fonts.ui_size)?,
            layout: Layout::new(width, height),
            open: false,
            first: 0,
            typing: None,
            confirm_kind: None,
        })
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.typing = None;
        self.confirm_kind = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.typing = None;
        self.confirm_kind = None;
    }

    pub fn hit(&self, x: i32, y: i32, rows: usize) -> Option<Hit> {
        self.layout.hit(x, y, self.first, rows)
    }

    /// Moves the page by `pages` (negative: back), within `rows` rows.
    pub fn page(&mut self, pages: i32, rows: usize) {
        let per_page = self.layout.rows_per_page();
        let last_first = rows.saturating_sub(1) / per_page * per_page;
        let next = self.first as i64 + i64::from(pages) * per_page as i64;
        self.first = next.clamp(0, last_first as i64) as usize;
    }

    pub fn show(&self, canvas: &mut WindowCanvas, catalogue: &Catalogue) {
        let (width, height) = (self.layout.width, self.layout.height);
        canvas.set_draw_color(BACKGROUND);
        let _ = canvas.fill_rect(Rect::new(0, 0, width, height));
        let files = catalogue.files();
        let text = |canvas: &mut WindowCanvas, s: &str, area: Rect, align, colour| {
            let _ = draw_text(canvas, &self.font, s, area, align, colour);
        };

        let (title, colour) = match &self.confirm_kind {
            Some((_, prompt)) => (prompt.clone(), SATELLITE),
            None => (
                format!(
                    "Chart manager: {} files. Tap a cell to change it.",
                    files.len()
                ),
                Color::BLACK,
            ),
        };
        let title_width = self.layout.previous_button().left() - MARGIN * 3;
        text(
            canvas,
            &title,
            Rect::new(MARGIN * 2, 0, title_width.max(0) as u32, HEADER_HEIGHT),
            TextAlign::Left,
            colour,
        );
        for (cell, _) in CELLS {
            let area = match cell {
                Cell::Up => self
                    .layout
                    .column_header(Cell::Up)
                    .union(self.layout.column_header(Cell::Down)),
                Cell::Down => continue,
                _ => self.layout.column_header(cell),
            };
            let align = if cell == Cell::Name {
                TextAlign::Left
            } else {
                TextAlign::Centered
            };
            let inset = Rect::new(
                area.x() + 6,
                area.y(),
                area.width().saturating_sub(12),
                area.height(),
            );
            text(canvas, header(cell), inset, align, LINE);
        }
        for (rect, label) in [
            (self.layout.previous_button(), "Previous"),
            (self.layout.next_button(), "Next"),
            (self.layout.close_button(), "Close"),
        ] {
            canvas.set_draw_color(LINE);
            let _ = canvas.draw_rect(rect);
            text(canvas, label, rect, TextAlign::Centered, Color::BLACK);
        }

        for slot in 0..self.layout.rows_per_page() {
            let Some(file) = files.get(self.first + slot) else {
                break;
            };
            let row = self
                .layout
                .cell(slot, Cell::Up)
                .union(self.layout.cell(slot, Cell::Where));
            if slot % 2 == 1 {
                canvas.set_draw_color(ROW_SHADE);
                let _ = canvas.fill_rect(row);
            }
            let name = match &self.typing {
                Some((typed, buffer)) if typed == &file.file_name => format!("{buffer}_"),
                _ if !file.reviewed => format!("NEW  {}", file.name),
                _ => file.name.clone(),
            };
            if !file.reviewed {
                canvas.set_draw_color(NEW_MARK);
                let _ = canvas.fill_rect(self.layout.cell(slot, Cell::Name));
            }
            let role = catalogue.role_of(&file.layer);
            let zooms = match (file.zooms.first(), file.zooms.last()) {
                (Some(a), Some(b)) => format!("z{a}-{b}"),
                _ => "-".to_string(),
            };
            let cells = [
                (Cell::Up, "▲".to_string(), Color::BLACK),
                (Cell::Down, "▼".to_string(), Color::BLACK),
                (Cell::Name, name, Color::BLACK),
                (Cell::Layer, file.layer.clone(), Color::BLACK),
                (Cell::Role, role.label().to_string(), kind_colour(role)),
                (Cell::Zooms, zooms, Color::BLACK),
                (
                    Cell::Shown,
                    if file.hidden { "Hidden" } else { "Visible" }.to_string(),
                    Color::BLACK,
                ),
                (Cell::Where, "Where".to_string(), Color::BLACK),
            ];
            for (cell, label, colour) in cells {
                let area = self.layout.cell(slot, cell);
                let align = if cell == Cell::Name {
                    TextAlign::Left
                } else {
                    TextAlign::Centered
                };
                let inset = Rect::new(
                    area.x() + 6,
                    area.y(),
                    area.width().saturating_sub(12),
                    area.height(),
                );
                text(canvas, &label, inset, align, colour);
            }
            canvas.set_draw_color(LINE);
            let _ = canvas.draw_line(row.bottom_left(), row.bottom_right());
        }

        let page_end = (self.first + self.layout.rows_per_page()).min(files.len());
        let footer = format!(
            "Rows {}-{} of {}. Layer cycles through the layers, then a layer of its own. Role applies to the whole layer. Where shows the coverage on the chart. Tap NEW to confirm a guess; tap a name again to type it.",
            (self.first + 1).min(files.len()),
            page_end,
            files.len()
        );
        text(
            canvas,
            &footer,
            Rect::new(
                MARGIN * 2,
                (height - FOOTER_HEIGHT) as i32,
                width - 4 * MARGIN as u32,
                FOOTER_HEIGHT,
            ),
            TextAlign::Left,
            Color::BLACK,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::test_support::session_with_charts;
    use super::*;

    fn layout() -> Layout {
        // 1600x800: 15 rows a page under the column headers.
        Layout::new(1600, 800)
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.center().x(), r.center().y())
    }

    #[test]
    fn header_buttons_and_row_cells_are_told_apart() {
        let l = layout();
        assert_eq!(l.rows_per_page(), 15);
        let (x, y) = centre(l.close_button());
        assert_eq!(l.hit(x, y, 0, 3), Some(Hit::Close));
        let (x, y) = centre(l.next_button());
        assert_eq!(l.hit(x, y, 0, 3), Some(Hit::NextPage));
        let (x, y) = centre(l.previous_button());
        assert_eq!(l.hit(x, y, 0, 3), Some(Hit::PreviousPage));

        let (x, y) = centre(l.cell(1, Cell::Layer));
        assert_eq!(l.hit(x, y, 0, 3), Some(Hit::Row(1, Cell::Layer)));
        let (x, y) = centre(l.cell(1, Cell::Where));
        assert_eq!(
            l.hit(x, y, 20, 30),
            Some(Hit::Row(21, Cell::Where)),
            "paged"
        );
        let (x, y) = centre(l.cell(2, Cell::Name));
        assert_eq!(l.hit(x, y, 0, 2), None, "below the last row");
        assert_eq!(l.hit(5, 20, 0, 3), None, "the title");
        let (x, y) = centre(l.column_header(Cell::Role));
        assert_eq!(l.hit(x, y, 0, 3), None, "a column header");
        assert!(l.column_header(Cell::Role).bottom() <= l.cell(0, Cell::Role).top());
    }

    #[test]
    fn every_column_has_a_header_and_order_spans_both_arrows() {
        let headers: Vec<&str> = CELLS.iter().map(|(c, _)| header(*c)).collect();
        assert_eq!(
            headers,
            vec!["Order", "", "Name", "Source", "Kind", "Zooms", "Visible", "Locate"]
        );
    }

    #[test]
    fn a_kind_change_into_or_out_of_chart_asks_first_and_a_second_tap_applies() {
        let fx = session_with_charts(&[
            ("Ocean_Navionics.mbtiles", &[2]),
            ("Isle_BingSat.mbtiles", &[2]),
        ]);
        let catalogue = &fx.session.catalogue;
        match kind_tap(catalogue, "Ocean_Navionics.mbtiles", None) {
            KindTap::Ask(prompt) => assert_eq!(
                prompt,
                "Make Navionics a Satellite? It will no longer be used for automatic \
                 switching. Tap Kind again to confirm."
            ),
            KindTap::Apply => panic!("a chart stops being one: ask"),
        }
        assert_eq!(
            kind_tap(
                catalogue,
                "Ocean_Navionics.mbtiles",
                Some("Ocean_Navionics.mbtiles")
            ),
            KindTap::Apply
        );
        assert!(
            matches!(
                kind_tap(
                    catalogue,
                    "Ocean_Navionics.mbtiles",
                    Some("Isle_BingSat.mbtiles")
                ),
                KindTap::Ask(_)
            ),
            "waiting on another file"
        );
        assert_eq!(
            kind_tap(catalogue, "Isle_BingSat.mbtiles", None),
            KindTap::Apply,
            "satellite to map needs no confirmation"
        );
        fx.finish();
    }

    #[test]
    fn the_cells_of_a_row_fill_the_width_without_overlap() {
        let l = layout();
        let mut x = MARGIN;
        for (cell, _) in CELLS {
            let r = l.cell(0, cell);
            assert_eq!(r.x(), x, "{cell:?}");
            x += r.width() as i32;
        }
        assert!(x <= 1600 - MARGIN, "{x}");
        assert!(Layout::new(640, 480).cell(0, Cell::Name).width() >= MIN_NAME_WIDTH);
    }

    #[test]
    fn edits_from_the_window_are_saved_and_shown() {
        let mut fx = session_with_charts(&[
            ("Ocean_Navionics.mbtiles", &[1, 2]),
            ("Isle_BingSat.mbtiles", &[1, 2]),
        ]);
        fx.session.state.layer = Some("Navionics".into());
        fx.session
            .edit_chart("Ocean_Navionics.mbtiles", ChartEdit::Rename("Ocean".into()));
        fx.session
            .edit_chart("Ocean_Navionics.mbtiles", ChartEdit::ToggleHidden);
        let saved = std::fs::read_to_string(fx.dir.join("charts.json")).unwrap();
        assert!(saved.contains("\"Ocean\""), "{saved}");
        assert_eq!(
            fx.session.state.layer, None,
            "its only file hidden, the layer left the switcher"
        );
        assert!(!fx.session.showing_photo(), "never a photo by fallback");
        fx.finish();
    }

    #[test]
    fn where_centres_the_chart_on_the_files_coverage_and_outlines_it() {
        let mut fx = session_with_charts(&[
            ("Ocean_Navionics.mbtiles", &[1, 2]),
            ("Coast_Navionics.mbtiles", &[3, 4]),
        ]);
        fx.session.state.follow = true;
        fx.session.locate_chart("Coast_Navionics.mbtiles");
        // The zoom animates to the file's first level; the file follows.
        std::thread::sleep(std::time::Duration::from_millis(260));
        fx.session.renderer.advance_zoom_animation();
        fx.session.sync_view();
        assert!(!fx.session.state.follow);
        assert_eq!(fx.session.state.zoom, 3, "its first level");
        assert!(fx.session.highlight.is_some());
        let (lon, lat) = fx.session.state.position.unwrap();
        assert!(
            lon.abs() < 1e-6 && lat.abs() < 1e-6,
            "whole-world fixture: {lon},{lat}"
        );
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Coast_Navionics.mbtiles"));
        fx.finish();
    }
}
