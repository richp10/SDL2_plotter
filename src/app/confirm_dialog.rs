use sdl2::pixels::Color;
use sdl2::rect::{Point, Rect};
use sdl2::render::{BlendMode, WindowCanvas};
use sdl2::ttf::{Font, Sdl2TtfContext};

use crate::config::FontConfig;
use crate::screen::{draw_text, TextAlign};

const PANEL_WIDTH: u32 = 440;
const PANEL_HEIGHT: u32 = 190;
const BUTTON_WIDTH: u32 = 112;
const BUTTON_HEIGHT: u32 = 38;
const BUTTON_GAP: i32 = 20;

/// The prefix is the verb the dialog confirms; other verbs (Quit, Reset)
/// are expected to join, so the names stay explicit.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy)]
pub(super) enum ConfirmAction {
    ClearRoute,
    ClearTrack,
    ClearWaypoints,
}

impl ConfirmAction {
    fn message(self) -> &'static str {
        match self {
            Self::ClearRoute => "Do you want to clear the current route?",
            Self::ClearTrack => "Do you want to clear the current track?",
            Self::ClearWaypoints => "Do you want to clear all waypoints?",
        }
    }

    fn confirm_label(self) -> &'static str {
        match self {
            Self::ClearRoute | Self::ClearTrack | Self::ClearWaypoints => "Clear",
        }
    }
}

pub(super) struct ConfirmDialog<'ttf> {
    font: Font<'ttf, 'static>,
    screen_width: u32,
    screen_height: u32,
    pending: Option<ConfirmAction>,
}

impl<'ttf> ConfirmDialog<'ttf> {
    pub fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        screen_width: u32,
        screen_height: u32,
        font_config: &FontConfig,
    ) -> Result<Self, String> {
        Ok(Self {
            font: ttf_context.load_font(&font_config.file, font_config.ui_size)?,
            screen_width,
            screen_height,
            pending: None,
        })
    }

    pub fn request(&mut self, action: ConfirmAction) {
        self.pending = Some(action);
    }

    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    pub fn resolve(&mut self, confirm: bool) -> Option<ConfirmAction> {
        let pending = self.pending.take()?;
        confirm.then_some(pending)
    }

    pub fn handle_click(&self, x: i32, y: i32) -> Option<bool> {
        self.pending?;
        let point = Point::new(x, y);
        if self.cancel_button().contains_point(point) {
            Some(false)
        } else if self.confirm_button().contains_point(point) {
            Some(true)
        } else {
            None
        }
    }

    pub fn show(&self, canvas: &mut WindowCanvas) -> Result<(), String> {
        let Some(action) = self.pending else {
            return Ok(());
        };

        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 170));
        canvas.fill_rect(Rect::new(0, 0, self.screen_width, self.screen_height))?;

        let panel = self.panel();
        canvas.set_draw_color(Color::RGB(42, 48, 58));
        canvas.fill_rect(panel)?;
        canvas.set_draw_color(Color::RGB(210, 215, 225));
        canvas.draw_rect(panel)?;

        draw_text(
            canvas,
            &self.font,
            "Confirm action",
            Rect::new(panel.x, panel.y + 20, panel.width(), 28),
            TextAlign::Centered,
            Color::RGB(255, 255, 255),
        )?;
        draw_text(
            canvas,
            &self.font,
            action.message(),
            Rect::new(
                panel.x + 16,
                panel.y + 64,
                panel.width().saturating_sub(32),
                30,
            ),
            TextAlign::Centered,
            Color::RGB(235, 235, 235),
        )?;

        let cancel = self.cancel_button();
        canvas.set_draw_color(Color::RGB(90, 98, 110));
        canvas.fill_rect(cancel)?;
        draw_text(
            canvas,
            &self.font,
            "Cancel",
            cancel,
            TextAlign::Centered,
            Color::WHITE,
        )?;

        let confirm = self.confirm_button();
        canvas.set_draw_color(Color::RGB(155, 55, 55));
        canvas.fill_rect(confirm)?;
        draw_text(
            canvas,
            &self.font,
            action.confirm_label(),
            confirm,
            TextAlign::Centered,
            Color::WHITE,
        )?;

        Ok(())
    }

    fn panel(&self) -> Rect {
        let width = self.screen_width.min(PANEL_WIDTH);
        let height = self.screen_height.min(PANEL_HEIGHT);
        Rect::new(
            (self.screen_width.saturating_sub(width) / 2) as i32,
            (self.screen_height.saturating_sub(height) / 2) as i32,
            width,
            height,
        )
    }

    fn cancel_button(&self) -> Rect {
        self.button_rect(false)
    }

    fn confirm_button(&self) -> Rect {
        self.button_rect(true)
    }

    fn button_rect(&self, confirm: bool) -> Rect {
        let panel = self.panel();
        let center_x = panel.x + panel.width() as i32 / 2;
        let x = if confirm {
            center_x + BUTTON_GAP / 2
        } else {
            center_x - BUTTON_WIDTH as i32 - BUTTON_GAP / 2
        };
        Rect::new(
            x,
            panel.y + panel.height() as i32 - BUTTON_HEIGHT as i32 - 18,
            BUTTON_WIDTH,
            BUTTON_HEIGHT,
        )
    }
}
