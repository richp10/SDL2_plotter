use sdl2::ttf::{Font, Sdl2TtfContext};

use crate::config::FontConfig;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{TextureQuery, WindowCanvas};

const COLOR: Color = gfx_color(200, 220, 220, 220);

const HEIGHT: u32 = 32;

const PHOTO_BADGE_TEXT: &str = "PHOTO - not a chart (tap for chart)";
const PHOTO_BADGE_WIDTH: u32 = 420;
const PHOTO_BADGE_HEIGHT: u32 = 40;
const PHOTO_BADGE_COLOR: Color = Color::RGB(190, 30, 30);

pub struct Buttombar<'ttf> {
    font: Font<'ttf, 'static>,
    width: u32,
    height: u32,
}

impl<'ttf> Buttombar<'ttf> {
    pub fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        width: u32,
        height: u32,
        font_config: &FontConfig,
    ) -> Result<Self, String> {
        let font = ttf_context.load_font(&font_config.file, font_config.ui_size)?;
        Ok(Buttombar {
            font,
            width,
            height,
        })
    }

    /// The bar with a message on the left, one in the centre (the chart
    /// shown) and one on the right.
    pub fn show(
        &mut self,
        canvas: &mut WindowCanvas,
        message1: &str,
        centre: &str,
        message2: &str,
    ) {
        canvas.set_draw_color(COLOR);
        canvas.set_viewport(Rect::new(
            0,
            (self.height - HEIGHT).try_into().unwrap(),
            self.width,
            HEIGHT,
        ));
        let r = sdl2::rect::Rect::new(0, 0, self.width, HEIGHT);
        canvas.fill_rect(r).unwrap();
        let _ = self.show_message_left(canvas, message1);
        if !centre.is_empty() {
            let _ = crate::screen::draw_text(
                canvas,
                &self.font,
                centre,
                Rect::new(0, 0, self.width, HEIGHT),
                crate::screen::TextAlign::Centered,
                Color::BLACK,
            );
        }
        let _ = self.show_message_right(canvas, message2);
        canvas.set_viewport(None);
    }

    /// Where the photo badge sits: top centre, under the menu bar.
    pub fn photo_badge_area(&self) -> Rect {
        let width = PHOTO_BADGE_WIDTH.min(self.width);
        Rect::new(
            (self.width as i32 - width as i32) / 2,
            HEIGHT as i32 + 12,
            width,
            PHOTO_BADGE_HEIGHT,
        )
    }

    /// The badge that says the chart area is a photo. Drawn before the
    /// stream is captured, because a viewer must know it too.
    pub fn show_photo_badge(&mut self, canvas: &mut WindowCanvas) {
        let area = self.photo_badge_area();
        canvas.set_draw_color(PHOTO_BADGE_COLOR);
        let _ = canvas.fill_rect(area);
        canvas.set_draw_color(Color::WHITE);
        let _ = canvas.draw_rect(area);
        let _ = crate::screen::draw_text(
            canvas,
            &self.font,
            PHOTO_BADGE_TEXT,
            area,
            crate::screen::TextAlign::Centered,
            Color::WHITE,
        );
    }

    fn show_message_left(&mut self, canvas: &mut WindowCanvas, texte: &str) -> Result<(), String> {
        let texture_creator = canvas.texture_creator();
        let font = &self.font;
        let color = sdl2::pixels::Color::BLACK;
        let surface = font
            .render(texte)
            .blended(color)
            .map_err(|e| e.to_string())?;
        let texture = texture_creator
            .create_texture_from_surface(&surface)
            .map_err(|e| e.to_string())?;

        let TextureQuery { width, height, .. } = texture.query();
        let cible = Rect::new(20, 7, width, height);

        canvas.copy(&texture, None, Some(cible))?;

        unsafe {
            texture.destroy();
        }
        Ok(())
    }

    fn show_message_right(&mut self, canvas: &mut WindowCanvas, texte: &str) -> Result<(), String> {
        let texture_creator = canvas.texture_creator();
        let font = &self.font;
        let color = sdl2::pixels::Color::BLACK;
        let surface = font
            .render(texte)
            .blended(color)
            .map_err(|e| e.to_string())?;
        let texture = texture_creator
            .create_texture_from_surface(&surface)
            .map_err(|e| e.to_string())?;

        let TextureQuery { width, height, .. } = texture.query();
        let cible = Rect::new(self.width as i32 - width as i32 - 20, 7, width, height);

        canvas.copy(&texture, None, Some(cible))?;

        unsafe {
            texture.destroy();
        }
        Ok(())
    }
}

const fn gfx_color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::RGBA(a, b, g, r)
}
