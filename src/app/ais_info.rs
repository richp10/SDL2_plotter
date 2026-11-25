//! The details window of one AIS target: a panel of label and value rows
//! (`ais::details`), under the menu bar on the left. A tap anywhere or
//! Escape closes it. Drawn after the stream capture, so the stream does not
//! show it.

use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{BlendMode, WindowCanvas};
use sdl2::ttf::{Font, Sdl2TtfContext};

use crate::config::FontConfig;
use crate::screen::{draw_text, TextAlign};

const MARGIN: i32 = 10;
const PADDING: i32 = 12;
const LABEL_WIDTH: u32 = 110;
const VALUE_WIDTH: u32 = 270;
/// The coloured strip at the top, in the target's category colour.
const STRIP: u32 = 6;
const BACKGROUND: Color = Color::RGBA(250, 250, 250, 240);
const BORDER: Color = Color::RGB(60, 60, 60);
const LABEL: Color = Color::RGB(95, 95, 95);
const VALUE: Color = Color::RGB(0, 0, 0);

/// The panel for `rows` rows of `row_height` pixels, on a screen
/// `screen_height` high: under the menu bar, never over the status bar.
pub(super) fn area(rows: usize, row_height: u32, screen_height: u32) -> Rect {
    let top = crate::topbar::HEIGHT as i32 + MARGIN;
    let wanted = STRIP + 2 * PADDING as u32 + rows as u32 * row_height;
    let room = (screen_height as i32 - top - 32 - MARGIN).max(row_height as i32) as u32;
    Rect::new(
        MARGIN,
        top,
        LABEL_WIDTH + VALUE_WIDTH + 2 * PADDING as u32,
        wanted.min(room),
    )
}

pub(super) struct AisInfoView<'ttf> {
    font: Font<'ttf, 'static>,
    height: u32,
}

impl<'ttf> AisInfoView<'ttf> {
    pub(super) fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        height: u32,
        fonts: &FontConfig,
    ) -> Result<Self, String> {
        let font = ttf_context.load_font(&fonts.file, fonts.ui_size)?;
        Ok(Self { font, height })
    }

    /// The window is now `height` high.
    pub(super) fn resize(&mut self, height: u32) {
        self.height = height;
    }

    fn row_height(&self) -> u32 {
        self.font.recommended_line_spacing().max(1) as u32 + 4
    }

    /// Where the panel is for `rows` rows.
    pub(super) fn area(&self, rows: usize) -> Rect {
        area(rows, self.row_height(), self.height)
    }

    pub(super) fn show(
        &self,
        canvas: &mut WindowCanvas,
        rows: &[(&str, String)],
        colour: Color,
    ) -> Result<(), String> {
        let panel = self.area(rows.len());
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(BACKGROUND);
        canvas.fill_rect(panel)?;
        canvas.set_draw_color(colour);
        canvas.fill_rect(Rect::new(panel.x(), panel.y(), panel.width(), STRIP))?;
        canvas.set_draw_color(BORDER);
        canvas.draw_rect(panel)?;

        let row_height = self.row_height();
        let mut y = panel.y() + STRIP as i32 + PADDING;
        for (label, value) in rows {
            if y + row_height as i32 > panel.bottom() - PADDING / 2 {
                break;
            }
            let label_area = Rect::new(panel.x() + PADDING, y, LABEL_WIDTH, row_height);
            let value_area = Rect::new(
                panel.x() + PADDING + LABEL_WIDTH as i32,
                y,
                VALUE_WIDTH,
                row_height,
            );
            draw_text(
                canvas,
                &self.font,
                label,
                label_area,
                TextAlign::Left,
                LABEL,
            )?;
            draw_text(
                canvas,
                &self.font,
                value,
                value_area,
                TextAlign::Left,
                VALUE,
            )?;
            y += row_height as i32;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_sits_under_the_menu_bar_and_above_the_status_bar() {
        for height in [800, 480] {
            let panel = area(15, 22, height);
            assert_eq!(panel.y(), crate::topbar::HEIGHT as i32 + 10);
            assert!(panel.bottom() <= height as i32 - 32, "{panel:?}");
        }
        let short = area(3, 22, 800);
        assert!(
            short.height() < area(15, 22, 800).height(),
            "it fits its rows"
        );
    }
}
