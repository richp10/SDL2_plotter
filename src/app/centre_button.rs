//! The button that centres the chart on the boat: bottom right, above the
//! status bar. Its state (`Session::centre_button`) is tested without SDL;
//! this module places and draws it. It is drawn after the stream capture,
//! so a viewer of the stream never sees it.

use sdl2::gfx::primitives::DrawRenderer;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{BlendMode, WindowCanvas};

use super::session::CentreButton;

/// The button is a square this many pixels wide, as the photo button.
pub(super) const SIZE: u32 = 72;
/// The status bar's height.
const STATUS_BAR: i32 = 32;
const MARGIN: i32 = 10;
const BACKGROUND: Color = Color::RGBA(30, 30, 30, 190);
const CENTRED: Color = Color::RGBA(0, 90, 200, 220);
const SYMBOL: Color = Color::RGB(255, 255, 255);
const DISABLED: Color = Color::RGB(110, 110, 110);

/// Where the button sits on a screen of the given size.
pub(super) fn area(screen_width: u32, screen_height: u32) -> Rect {
    Rect::new(
        screen_width as i32 - MARGIN - SIZE as i32,
        screen_height as i32 - STATUS_BAR - MARGIN - SIZE as i32,
        SIZE,
        SIZE,
    )
}

pub(super) struct CentreButtonView {
    width: u32,
    height: u32,
}

impl CentreButtonView {
    pub(super) fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub(super) fn area(&self) -> Rect {
        area(self.width, self.height)
    }

    /// The window is now `width` x `height`.
    pub(super) fn resize(&mut self, width: u32, height: u32) {
        (self.width, self.height) = (width, height);
    }

    /// A square with a target symbol (ring, cross and dot): filled blue
    /// while centred, grey before the first fix.
    pub(super) fn show(
        &self,
        canvas: &mut WindowCanvas,
        state: CentreButton,
    ) -> Result<(), String> {
        let rect = self.area();
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(if state.centred { CENTRED } else { BACKGROUND });
        canvas.fill_rect(rect)?;
        let symbol = if state.enabled { SYMBOL } else { DISABLED };
        canvas.set_draw_color(symbol);
        canvas.draw_rect(rect)?;

        let centre = rect.center();
        let (Ok(x), Ok(y)) = (i16::try_from(centre.x()), i16::try_from(centre.y())) else {
            return Ok(());
        };
        let ring = (SIZE / 4) as i16;
        let tick = (SIZE / 8) as i16;
        canvas.circle(x, y, ring, symbol)?;
        canvas.circle(x, y, ring - 1, symbol)?;
        canvas.thick_line(x - ring - tick, y, x - ring + tick / 2, y, 2, symbol)?;
        canvas.thick_line(x + ring - tick / 2, y, x + ring + tick, y, 2, symbol)?;
        canvas.thick_line(x, y - ring - tick, x, y - ring + tick / 2, 2, symbol)?;
        canvas.thick_line(x, y + ring - tick / 2, x, y + ring + tick, 2, symbol)?;
        canvas.filled_circle(x, y, 3, symbol)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_sits_bottom_right_above_the_status_bar() {
        for (w, h) in [(1280, 800), (640, 480)] {
            let a = area(w, h);
            assert_eq!((a.width(), a.height()), (SIZE, SIZE));
            assert_eq!(a.right(), w as i32 - 10);
            assert!(a.bottom() <= h as i32 - 32, "{a:?} over the status bar");
            assert!(a.y() > h as i32 / 2, "{a:?} not at the bottom");
            let photo = super::super::photo_button::area(h);
            assert!(!a.has_intersection(photo), "clear of the photo button");
        }
    }
}
