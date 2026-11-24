//! The photo button: bottom left, above the status bar, shown only while a
//! photo of the screen centre exists (or one is shown). It previews where a
//! tap goes, the next photo source or the chart, with a thumbnail of that
//! file under the centre. Drawn after the stream capture, like the menus.

use std::sync::Arc;

use image::RgbaImage;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{Texture, WindowCanvas};
use sdl2::ttf::{Font, Sdl2TtfContext};

use super::session::PhotoButton;
use crate::config::FontConfig;

/// The button is a square this many pixels wide.
pub(super) const SIZE: u32 = 72;
/// The status bar's height.
const STATUS_BAR: i32 = 32;
const MARGIN: i32 = 10;
const BORDER: u32 = 2;
const EMPTY: Color = Color::RGB(60, 60, 60);
const LABEL_BACKING: Color = Color::RGBA(0, 0, 0, 170);

/// Where the button sits on a screen of the given size.
pub(super) fn area(screen_height: u32) -> Rect {
    Rect::new(
        MARGIN,
        screen_height as i32 - STATUS_BAR - MARGIN - SIZE as i32,
        SIZE,
        SIZE,
    )
}

pub(super) struct PhotoButtonView<'ttf> {
    font: Font<'ttf, 'static>,
    height: u32,
    /// The thumbnail last uploaded, so an unchanged one is not sent again.
    uploaded: Option<(Arc<RgbaImage>, Texture)>,
}

impl<'ttf> PhotoButtonView<'ttf> {
    pub(super) fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        height: u32,
        fonts: &FontConfig,
    ) -> Result<Self, String> {
        Ok(PhotoButtonView {
            font: ttf_context.load_font(&fonts.file, fonts.ui_size)?,
            height,
            uploaded: None,
        })
    }

    pub(super) fn area(&self) -> Rect {
        area(self.height)
    }

    /// The thumbnail (or a plain square naming where a tap goes), its
    /// border, and "k/n" while a photo is shown.
    pub(super) fn show(
        &mut self,
        canvas: &mut WindowCanvas,
        button: &PhotoButton,
        thumbnail: Option<&Arc<RgbaImage>>,
    ) {
        let area = self.area();
        let drawn = thumbnail.is_some_and(|image| {
            self.upload(canvas, image)
                .and_then(|texture| canvas.copy(texture, None, Some(area)))
                .is_ok()
        });
        if !drawn {
            canvas.set_draw_color(EMPTY);
            let _ = canvas.fill_rect(area);
            let label = if button.preview_is_photo {
                "PHOTO"
            } else {
                "CHART"
            };
            let _ = crate::screen::draw_text(
                canvas,
                &self.font,
                label,
                area,
                crate::screen::TextAlign::Centered,
                Color::WHITE,
            );
        }
        canvas.set_draw_color(Color::WHITE);
        for inset in 0..BORDER {
            let _ = canvas.draw_rect(Rect::new(
                area.x() + inset as i32,
                area.y() + inset as i32,
                area.width() - 2 * inset,
                area.height() - 2 * inset,
            ));
        }
        if let Some((k, n)) = button.position {
            let corner = Rect::new(
                area.right() - 34 - BORDER as i32,
                area.bottom() - 22 - BORDER as i32,
                34,
                22,
            );
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(LABEL_BACKING);
            let _ = canvas.fill_rect(corner);
            let _ = crate::screen::draw_text(
                canvas,
                &self.font,
                &format!("{k}/{n}"),
                corner,
                crate::screen::TextAlign::Centered,
                Color::WHITE,
            );
        }
    }

    /// The texture holding `image`, uploaded only when it changed.
    fn upload(
        &mut self,
        canvas: &mut WindowCanvas,
        image: &Arc<RgbaImage>,
    ) -> Result<&Texture, String> {
        let current = matches!(&self.uploaded, Some((shown, _)) if Arc::ptr_eq(shown, image));
        if !current {
            if let Some((_, old)) = self.uploaded.take() {
                // SAFETY: the texture's creator (the canvas) is still alive.
                unsafe { old.destroy() };
            }
            let mut texture = canvas
                .texture_creator()
                .create_texture_static(PixelFormatEnum::RGBA32, image.width(), image.height())
                .map_err(|e| e.to_string())?;
            texture
                .update(None, image.as_raw(), image.width() as usize * 4)
                .map_err(|e| e.to_string())?;
            self.uploaded = Some((image.clone(), texture));
        }
        Ok(&self.uploaded.as_ref().expect("just uploaded").1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_sits_bottom_left_above_the_status_bar() {
        for h in [800, 480] {
            let a = area(h);
            assert_eq!(a.x(), 10);
            assert_eq!((a.width(), a.height()), (72, 72));
            assert!(a.bottom() <= h as i32 - 32, "{a:?} over the status bar");
            assert!(a.y() > h as i32 / 2, "{a:?} not at the bottom");
        }
    }
}
