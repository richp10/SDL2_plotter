use crate::config::{AppearanceConfig, FontConfig, WindowConfig};
use sdl2::gfx::primitives::DrawRenderer;
use sdl2::mouse::{Cursor, SystemCursor};
use sdl2::pixels::Color;
use sdl2::rect::Point;
use sdl2::rect::Rect;
use sdl2::render::BlendMode;
use sdl2::render::{Texture, TextureQuery, WindowCanvas};
use sdl2::EventPump;
use sdl2::{
    ttf::{Font, Sdl2TtfContext},
    Sdl,
};

pub struct MoteurSdl {
    pub context: Sdl,
    pub event_pump: EventPump,
    pub ttf_context: Sdl2TtfContext,
}

impl MoteurSdl {
    pub fn init() -> Result<Self, String> {
        let context = sdl2::init()?;
        let event_pump = context.event_pump()?;
        let ttf_context = sdl2::ttf::init().map_err(|e| e.to_string())?;

        Ok(MoteurSdl {
            context,
            event_pump,
            ttf_context: ttf_context,
        })
    }
}

pub struct Map<'ttf> {
    font: Font<'ttf, 'static>,
    pub canvas: WindowCanvas,
    pub screen_width: u32,
    pub screen_height: u32,
    pub cursor_hand: Cursor,
    pub cursor_cross: Cursor,
    pub cursor_arrow: Cursor,
    pub cursor_size: Cursor,
    appearance: AppearanceConfig,
}

impl<'ttf> Map<'ttf> {
    pub fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        context: &Sdl,
        window_config: &WindowConfig,
        font_config: &FontConfig,
        appearance: &AppearanceConfig,
    ) -> Result<Self, String> {
        let font = ttf_context.load_font(&font_config.file, font_config.window_size)?;

        let video_subsystem = context
            .video()
            .map_err(|e| format!("Sous-système vidéo SDL indisponible : {e}"))?;
        let window = match window_config.fullscreen {
            false => video_subsystem
                .window(
                    &window_config.title,
                    window_config.width,
                    window_config.height,
                )
                .position_centered()
                .build(),
            true => video_subsystem
                .window(
                    &window_config.title,
                    window_config.width,
                    window_config.height,
                )
                .position_centered()
                .fullscreen_desktop()
                .build(),
        }
        .map_err(|e| {
            format!(
                "Impossible de créer la fenêtre {}x{} (plein écran : {}) : {e}",
                window_config.width, window_config.height, window_config.fullscreen
            )
        })?;

        let display_mode = video_subsystem
            .current_display_mode(0)
            .map_err(|e| format!("Mode d'affichage de l'écran 0 indisponible : {e}"))?;
        let (screen_width, screen_height) = match window_config.fullscreen {
            false => (window_config.width, window_config.height),
            true => (display_mode.w as u32, display_mode.h as u32),
        };

        let canvas = window
            .into_canvas()
            .accelerated()
            .build()
            .map_err(|e| format!("Impossible de créer le canvas accéléré : {e}"))?;
        let cursor_hand = Cursor::from_system(SystemCursor::Hand)?;
        let cursor_cross = Cursor::from_system(SystemCursor::Crosshair)?;
        let cursor_arrow = Cursor::from_system(SystemCursor::Arrow)?;
        let cursor_size = Cursor::from_system(SystemCursor::SizeAll)?;
        cursor_hand.set();
        Ok(Map {
            font,
            canvas,
            screen_width,
            screen_height,
            cursor_hand,
            cursor_cross,
            cursor_arrow,
            cursor_size,
            appearance: appearance.clone(),
        })
    }

    pub fn draw_mid_handles(&mut self, pts: &[Point]) -> Result<(), String> {
        self.canvas
            .set_draw_color(configured_color(self.appearance.route_color));
        for p in pts {
            let r = sdl2::rect::Rect::new(p.x - 4, p.y - 4, 12, 12);
            self.canvas.draw_rect(r)?; // contour seulement, pour les distinguer des vrais points
        }
        Ok(())
    }

    fn clip(&self) -> ClipRect {
        ClipRect::for_screen(self.screen_width, self.screen_height)
    }

    /// The point as `i16` coordinates when it is near enough to the screen
    /// to be drawn with the SDL2_gfx primitives, otherwise `None`.
    pub fn point_i16(&self, point: Point) -> Option<(i16, i16)> {
        self.clip().point_i16(point)
    }

    pub fn show_cog(&mut self, mut radians: f64, pos: Point) -> Result<(), String> {
        radians -= std::f64::consts::PI / 2.0;
        let x1 = pos.x as f64 + (radians.cos() * self.appearance.cog_length);
        let y1 = pos.y as f64 + (radians.sin() * self.appearance.cog_length);
        let end = Point::new(x1.round() as i32, y1.round() as i32);
        let Some(((ax, ay), (bx, by))) = self.clip().segment_i16(pos, end) else {
            return Ok(());
        };

        self.canvas.thick_line(
            ax,
            ay,
            bx,
            by,
            self.appearance.cog_width,
            configured_color(self.appearance.cog_color),
        )?;
        Ok(())
    }

    pub fn show_route(&mut self, pts: &[Point]) {
        let clip = self.clip();
        for seg in pts.windows(2) {
            let Some(((ax, ay), (bx, by))) = clip.segment_i16(seg[0], seg[1]) else {
                continue;
            };
            let _ = self.canvas.thick_line(
                ax,
                ay,
                bx,
                by,
                self.appearance.route_width,
                configured_color(self.appearance.route_color),
            );
        }
        self.canvas
            .set_draw_color(configured_color(self.appearance.route_color));
        for pt in pts {
            let Some((x, y)) = clip.point_i16(*pt) else {
                continue;
            };
            let _ = self.canvas.filled_circle(
                x,
                y,
                self.appearance.route_point_size,
                configured_color(self.appearance.route_color),
            );
        }
    }

    pub fn show_map(
        &mut self,
        texture: &Texture,
        source_rect: Rect,
        display_width: u32,
        display_height: u32,
        rotation_degrees: f64,
    ) -> Result<(), String> {
        let dst = Rect::new(
            (self.screen_width as i32 - display_width as i32) / 2,
            (self.screen_height as i32 - display_height as i32) / 2,
            display_width,
            display_height,
        );
        self.canvas.copy_ex(
            texture,
            Some(source_rect),
            Some(dst),
            rotation_degrees,
            None,
            false,
            false,
        )?;
        Ok(())
    }

    pub fn drag_map(
        &mut self,
        texture: &Texture,
        x: i32,
        y: i32,
        source_rect: Rect,
        display_width: u32,
        display_height: u32,
        rotation_degrees: f64,
    ) -> Result<(), String> {
        let dst_rect = Rect::new(
            (self.screen_width as i32 - display_width as i32) / 2 + x,
            (self.screen_height as i32 - display_height as i32) / 2 + y,
            display_width,
            display_height,
        );
        self.canvas
            .set_draw_color(configured_color(self.appearance.map_background_color));
        self.canvas.clear();
        self.canvas.copy_ex(
            texture,
            Some(source_rect),
            Some(dst_rect),
            rotation_degrees,
            None,
            false,
            false,
        )?;
        self.canvas.present();
        Ok(())
    }

    pub fn show_boat(&mut self, x: i32, y: i32) -> Result<(), String> {
        if !self.clip().contains(f64::from(x), f64::from(y)) {
            return Ok(());
        }
        let boat_color = configured_color(self.appearance.boat_color);
        self.canvas.set_blend_mode(BlendMode::Blend);
        self.canvas.set_draw_color(boat_color);
        draw_filled_circle(
            &mut self.canvas,
            x,
            y,
            i32::from(self.appearance.boat_size),
            boat_color,
        )?;
        draw_circle(
            &mut self.canvas,
            x,
            y,
            i32::from(self.appearance.boat_circle_radius),
            boat_color,
        )?;
        Ok(())
    }

    pub fn show_ais_target(
        &mut self,
        x: i32,
        y: i32,
        course_over_ground: Option<f64>,
    ) -> Result<(), String> {
        let clip = self.clip();
        let Some((x16, y16)) = clip.point_i16(Point::new(x, y)) else {
            return Ok(());
        };
        if let Some(course) = course_over_ground.filter(|course| course.is_finite()) {
            let end_x =
                x + (course.sin() * f64::from(self.appearance.ais_course_length)).round() as i32;
            let end_y =
                y - (course.cos() * f64::from(self.appearance.ais_course_length)).round() as i32;
            if let Some(((ax, ay), (bx, by))) =
                clip.segment_i16(Point::new(x, y), Point::new(end_x, end_y))
            {
                let _ = self.canvas.thick_line(
                    ax,
                    ay,
                    bx,
                    by,
                    self.appearance.ais_course_width,
                    configured_color(self.appearance.ais_color),
                );
            }
        }
        self.canvas.filled_circle(
            x16,
            y16,
            self.appearance.ais_size,
            configured_color(self.appearance.ais_color),
        )?;
        self.canvas.circle(
            x16,
            y16,
            self.appearance.ais_outline_size,
            configured_color(self.appearance.ais_outline_color),
        )?;
        Ok(())
    }

    pub fn show_mark(&mut self, x: i32, y: i32) -> Result<(), String> {
        if !self.clip().contains(f64::from(x), f64::from(y)) {
            return Ok(());
        }
        draw_filled_circle(
            &mut self.canvas,
            x,
            y,
            i32::from(self.appearance.mark_size),
            configured_color(self.appearance.measure_color),
        )?;
        Ok(())
    }

    pub fn show_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<(), String> {
        self.canvas
            .set_draw_color(configured_color(self.appearance.line_color));
        self.canvas
            .draw_line(Point::new(x1, y1), Point::new(x2, y2))
    }

    pub fn night_layer(&mut self, opacity: u8) -> Result<(), String> {
        self.canvas.set_blend_mode(BlendMode::Blend);
        let [red, green, blue, _] = self.appearance.night_overlay_color;
        self.canvas
            .set_draw_color(Color::RGBA(red, green, blue, opacity));

        let ecran_total = Rect::new(0, 0, self.screen_width, self.screen_height);
        self.canvas.fill_rect(ecran_total)?;
        Ok(())
    }

    pub fn show_text(&mut self, texte: &str, x: i32, y: i32) -> Result<(), String> {
        let texture_creator = self.canvas.texture_creator();
        let font = &self.font;

        let surface = font
            .render(texte)
            .blended(sdl2::pixels::Color::BLACK)
            .map_err(|e| e.to_string())?;
        let texture = texture_creator
            .create_texture_from_surface(&surface)
            .map_err(|e| e.to_string())?;

        let TextureQuery { width, height, .. } = texture.query();

        let cible = Rect::new(x, y, width, height);

        self.canvas.set_blend_mode(BlendMode::Blend);
        self.canvas.set_draw_color(Color::RGBA(255, 255, 255, 128));
        self.canvas.fill_rect(cible)?;

        self.canvas.copy(&texture, None, Some(cible))?;
        unsafe {
            texture.destroy();
        }
        Ok(())
    }
}

fn configured_color([red, green, blue, alpha]: [u8; 4]) -> Color {
    Color::RGBA(red, green, blue, alpha)
}

/// Extra pixels around the window inside which drawing is still allowed, so
/// that thick lines and symbols near the edge are not cut off abruptly.
const CLIP_MARGIN: f64 = 64.0;

/// Rectangle every coordinate must lie in before it is handed to the
/// SDL2_gfx primitives, which take `i16`. A point far off screen (a route
/// point on the other side of the world at a high zoom) would otherwise wrap
/// when cast and draw phantom lines across the chart.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipRect {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl ClipRect {
    fn for_screen(width: u32, height: u32) -> Self {
        Self {
            min_x: -CLIP_MARGIN,
            min_y: -CLIP_MARGIN,
            max_x: f64::from(width) + CLIP_MARGIN,
            max_y: f64::from(height) + CLIP_MARGIN,
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// The point as `i16` coordinates if it lies inside, otherwise `None`.
    fn point_i16(&self, point: Point) -> Option<(i16, i16)> {
        let (x, y) = (f64::from(point.x), f64::from(point.y));
        self.contains(x, y).then(|| (x as i16, y as i16))
    }

    /// The part of the segment inside the rectangle, as `i16` coordinates.
    fn segment_i16(&self, a: Point, b: Point) -> Option<((i16, i16), (i16, i16))> {
        let ((ax, ay), (bx, by)) = clip_segment(
            (f64::from(a.x), f64::from(a.y)),
            (f64::from(b.x), f64::from(b.y)),
            self,
        )?;
        Some((
            (ax.round() as i16, ay.round() as i16),
            (bx.round() as i16, by.round() as i16),
        ))
    }
}

/// Liang-Barsky clipping of the segment `a`-`b` against `rect`.
/// Returns the visible part, or `None` when the segment misses the rectangle.
fn clip_segment(a: (f64, f64), b: (f64, f64), rect: &ClipRect) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t_enter, mut t_exit) = (0.0_f64, 1.0_f64);
    let edges = [
        (-dx, a.0 - rect.min_x),
        (dx, rect.max_x - a.0),
        (-dy, a.1 - rect.min_y),
        (dy, rect.max_y - a.1),
    ];
    for (p, q) in edges {
        if p == 0.0 {
            if q < 0.0 {
                return None; // parallel to this edge and outside it
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            if t > t_exit {
                return None;
            }
            t_enter = t_enter.max(t);
        } else {
            if t < t_enter {
                return None;
            }
            t_exit = t_exit.min(t);
        }
    }
    Some((
        (a.0 + t_enter * dx, a.1 + t_enter * dy),
        (a.0 + t_exit * dx, a.1 + t_exit * dy),
    ))
}

fn draw_filled_circle(
    canvas: &mut WindowCanvas,
    center_x: i32,
    center_y: i32,
    radius: i32,
    color: Color,
) -> Result<(), String> {
    canvas.set_draw_color(color);
    let radius = radius.max(1);
    for dy in -radius..=radius {
        let dx = ((radius * radius - dy * dy).max(0) as f64).sqrt().floor() as i32;
        canvas.draw_line(
            Point::new(center_x - dx, center_y + dy),
            Point::new(center_x + dx, center_y + dy),
        )?;
    }
    Ok(())
}

fn draw_circle(
    canvas: &mut WindowCanvas,
    center_x: i32,
    center_y: i32,
    radius: i32,
    color: Color,
) -> Result<(), String> {
    canvas.set_draw_color(color);
    let radius = radius.max(1);
    let mut points = Vec::with_capacity(4 * radius as usize);
    let mut x = radius;
    let mut y = 0;
    let mut error = 1 - radius;

    while x >= y {
        points.push(Point::new(center_x + x, center_y + y));
        points.push(Point::new(center_x + y, center_y + x));
        points.push(Point::new(center_x - y, center_y + x));
        points.push(Point::new(center_x - x, center_y + y));
        points.push(Point::new(center_x - x, center_y - y));
        points.push(Point::new(center_x - y, center_y - x));
        points.push(Point::new(center_x + y, center_y - x));
        points.push(Point::new(center_x + x, center_y - y));

        y += 1;
        if error <= 0 {
            error += 2 * y + 1;
        } else {
            x -= 1;
            error += 2 * (y - x) + 1;
        }
    }
    canvas.draw_points(points.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: ClipRect = ClipRect {
        min_x: -64.0,
        min_y: -64.0,
        max_x: 1664.0,
        max_y: 864.0,
    };

    #[test]
    fn segment_inside_the_clip_rectangle_is_unchanged() {
        let clipped = clip_segment((100.0, 100.0), (800.0, 400.0), &RECT).unwrap();
        assert_eq!(clipped, ((100.0, 100.0), (800.0, 400.0)));
    }

    #[test]
    fn segment_entirely_outside_is_dropped() {
        assert_eq!(clip_segment((2000.0, 100.0), (3000.0, 200.0), &RECT), None);
        assert_eq!(clip_segment((100.0, -500.0), (800.0, -300.0), &RECT), None);
    }

    #[test]
    fn segment_crossing_the_edge_is_cut_at_the_boundary() {
        let ((ax, ay), (bx, by)) = clip_segment((800.0, 400.0), (800.0, 5000.0), &RECT).unwrap();
        assert_eq!((ax, ay), (800.0, 400.0));
        assert_eq!(bx, 800.0);
        assert!((by - 864.0).abs() < 1e-9, "{by}");
    }

    #[test]
    fn far_away_endpoints_are_brought_into_i16_range() {
        // A route point on the other side of the world at a high zoom is
        // millions of pixels away; the raw value wraps when cast to i16.
        let far = (-3_000_000.0, 2_500_000.0);
        let ((_, _), (bx, by)) = clip_segment((800.0, 400.0), far, &RECT).unwrap();
        assert!(i16::try_from(bx.round() as i64).is_ok(), "{bx}");
        assert!(i16::try_from(by.round() as i64).is_ok(), "{by}");
        assert!(bx >= RECT.min_x && by >= RECT.min_y);
    }

    #[test]
    fn points_outside_the_clip_rectangle_are_not_drawn() {
        assert_eq!(RECT.point_i16(Point::new(800, 400)), Some((800, 400)));
        assert_eq!(RECT.point_i16(Point::new(-100, 400)), None);
        assert_eq!(RECT.point_i16(Point::new(800, 40_000)), None);
        assert_eq!(RECT.point_i16(Point::new(i32::MIN, i32::MAX)), None);
    }
}
