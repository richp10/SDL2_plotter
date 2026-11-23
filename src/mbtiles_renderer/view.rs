//! A snapshot of what the screen is looking at, and the conversions between
//! screen pixels and chart positions that follow from it.
//!
//! `ChartView` is `Copy` and holds no chart data, so input handling, hit
//! testing and overlays can convert without borrowing the renderer, and
//! tests need no chart file. Pixels are absolute screen pixels with the
//! origin top-left; positions are `(lon, lat)` in degrees. A pixel that maps
//! outside the Mercator plane (above or below the poles) has no position.

use sdl2::rect::Point;

use super::projection::{
    course_to_screen, lonlat_to_world, screen_to_world_delta, world_scale, world_to_lonlat,
    world_to_screen_delta,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartView {
    center_world: (f64, f64),
    display_zoom: f64,
    rotation: f64,
    width: u32,
    height: u32,
}

impl ChartView {
    /// A view of `width` x `height` pixels whose centre pixel shows the world
    /// point `center_world` at `display_zoom`, rotated by `rotation` radians.
    pub(super) fn new(
        center_world: (f64, f64),
        display_zoom: f64,
        rotation: f64,
        width: u32,
        height: u32,
    ) -> Self {
        ChartView {
            center_world,
            display_zoom,
            rotation,
            width,
            height,
        }
    }

    /// A view centred on a position, for tests and tools.
    #[cfg(test)]
    pub(crate) fn centred_on(
        center: (f64, f64),
        display_zoom: f64,
        rotation: f64,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new(
            lonlat_to_world(center.0, center.1, display_zoom),
            display_zoom,
            rotation,
            width,
            height,
        )
    }

    /// The screen pixel showing a position. Off-screen positions give pixels
    /// outside `size()`, which callers cull or clip; nothing wraps.
    pub fn to_pixel(self, (lon, lat): (f64, f64)) -> Point {
        let (world_x, world_y) = lonlat_to_world(lon, lat, self.display_zoom);
        self.world_to_pixel(world_x, world_y)
    }

    /// The screen pixel showing a normalised Mercator point (0..1 on both
    /// axes), as the track stores them.
    pub fn norm_to_pixel(&self, nx: f64, ny: f64) -> Point {
        let scale = world_scale(self.display_zoom);
        self.world_to_pixel(nx * scale, ny * scale)
    }

    /// The position under a screen pixel. Longitude is wrapped into
    /// -180..180 so a click across the antimeridian is still a position;
    /// `None` when the pixel lies above or below the Mercator world square
    /// (beyond about 85 degrees of latitude there is no chart at all).
    pub fn to_lonlat(self, pixel: Point) -> Option<(f64, f64)> {
        let dx = pixel.x as f64 - self.width as f64 / 2.0;
        let dy = pixel.y as f64 - self.height as f64 / 2.0;
        let (world_dx, world_dy) = screen_to_world_delta(dx, dy, self.rotation);
        let world_y = self.center_world.1 + world_dy;
        if !(0.0..=world_scale(self.display_zoom)).contains(&world_y) {
            return None;
        }
        let (lon, lat) =
            world_to_lonlat(self.center_world.0 + world_dx, world_y, self.display_zoom);
        if !lon.is_finite() || !lat.is_finite() {
            return None;
        }
        let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        Some((lon, lat))
    }

    /// The position that would sit at the screen centre after dragging the
    /// chart by `(dx, dy)` pixels: the point now `(dx, dy)` left and above
    /// the centre.
    pub fn panned_center(&self, dx: i32, dy: i32) -> Option<(f64, f64)> {
        self.to_lonlat(Point::new(
            self.width as i32 / 2 - dx,
            self.height as i32 / 2 - dy,
        ))
    }

    /// The index of the position drawn closest to `pixel`, if any lies
    /// within `radius` pixels.
    pub fn nearest(
        &self,
        positions: impl IntoIterator<Item = (f64, f64)>,
        pixel: Point,
        radius: i32,
    ) -> Option<usize> {
        let r2 = i64::from(radius) * i64::from(radius);
        positions
            .into_iter()
            .enumerate()
            .map(|(index, position)| {
                let p = self.to_pixel(position);
                let dx = i64::from(p.x) - i64::from(pixel.x);
                let dy = i64::from(p.y) - i64::from(pixel.y);
                (index, dx * dx + dy * dy)
            })
            .filter(|&(_, d2)| d2 <= r2)
            .min_by_key(|&(_, d2)| d2)
            .map(|(index, _)| index)
    }

    /// True when `pixel` is on screen or within `margin` pixels of it.
    pub fn contains(&self, pixel: Point, margin: i32) -> bool {
        pixel.x >= -margin
            && pixel.y >= -margin
            && pixel.x <= self.width as i32 + margin
            && pixel.y <= self.height as i32 + margin
    }

    /// A true course as it appears on this (possibly rotated) view.
    pub fn course_to_screen(&self, course: f64) -> f64 {
        course_to_screen(course, self.rotation)
    }

    fn world_to_pixel(&self, world_x: f64, world_y: f64) -> Point {
        let (x, y) = world_to_screen_delta(
            world_x - self.center_world.0,
            world_y - self.center_world.1,
            self.rotation,
        );
        Point::new(
            (x + self.width as f64 / 2.0).round() as i32,
            (y + self.height as f64 / 2.0).round() as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BREST: (f64, f64) = (-4.4861, 48.3904);

    fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-6 && (actual.1 - expected.1).abs() < 1e-6,
            "got {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn the_screen_centre_shows_the_viewed_position() {
        let view = ChartView::centred_on(BREST, 9.0, 0.0, 512, 256);
        assert_eq!(view.to_pixel(BREST), Point::new(256, 128));
        assert_close(view.to_lonlat(Point::new(256, 128)).unwrap(), BREST);
    }

    #[test]
    fn pixel_and_position_round_trip_under_rotation() {
        let view = ChartView::centred_on(BREST, 10.3, 0.7, 800, 600);
        let pixel = Point::new(123, 456);
        let position = view.to_lonlat(pixel).unwrap();
        let back = view.to_pixel(position);
        assert!(
            (back.x - pixel.x).abs() <= 1 && (back.y - pixel.y).abs() <= 1,
            "{back:?}"
        );
    }

    #[test]
    fn a_click_across_the_antimeridian_wraps_into_range() {
        let view = ChartView::centred_on((179.9, 0.0), 4.0, 0.0, 512, 256);
        let (lon, lat) = view.to_lonlat(Point::new(511, 128)).unwrap();
        assert!((-180.0..=180.0).contains(&lon), "lon {lon}");
        assert!(
            lon < 0.0,
            "east of the antimeridian is western longitude, got {lon}"
        );
        assert!(lat.abs() < 1e-9);
    }

    #[test]
    fn a_click_beyond_the_poles_has_no_position() {
        // Zoom 0: the whole world is 256 px tall, so half a screen above the
        // centre of a view on 60 N is off the top of the Mercator square.
        let view = ChartView::centred_on((0.0, 60.0), 0.0, 0.0, 512, 256);
        assert_eq!(view.to_lonlat(Point::new(256, 0)), None);
        assert!(view.to_lonlat(Point::new(256, 128)).is_some());
    }

    #[test]
    fn nearest_picks_the_closest_position_within_the_radius() {
        let view = ChartView::centred_on((0.0, 0.0), 6.0, 0.0, 512, 256);
        let points = [(0.0, 0.0), (0.02, 0.0), (1.0, 1.0)];
        let second = view.to_pixel(points[1]);
        assert_eq!(
            view.nearest(points, Point::new(second.x + 3, second.y), 8),
            Some(1)
        );
        assert_eq!(
            view.nearest(points, Point::new(second.x + 30, second.y), 8),
            None
        );
        assert_eq!(view.nearest([], Point::new(0, 0), 8), None);
    }

    #[test]
    fn dragging_the_chart_right_moves_the_centre_west() {
        let view = ChartView::centred_on((0.0, 0.0), 4.0, 0.0, 512, 256);
        let (lon, lat) = view.panned_center(100, 0).unwrap();
        assert!(lon < 0.0, "lon {lon}");
        assert!(lat.abs() < 1e-9);
    }

    #[test]
    fn contains_applies_the_margin() {
        let view = ChartView::centred_on((0.0, 0.0), 4.0, 0.0, 512, 256);
        assert!(view.contains(Point::new(-20, 10), 32));
        assert!(!view.contains(Point::new(-40, 10), 32));
        assert!(view.contains(Point::new(512, 256), 0));
    }

    #[test]
    fn a_course_on_a_course_up_view_points_to_the_top() {
        let view = ChartView::centred_on((0.0, 0.0), 4.0, std::f64::consts::FRAC_PI_2, 512, 256);
        assert!(view.course_to_screen(std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }
}
