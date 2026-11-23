//! Web Mercator maths and the geometry of a rotated, zoomed view.
//!
//! Pure functions only: no SDL, no SQLite, no state. "World" coordinates are
//! pixels on the full Mercator plane at a given (possibly fractional)
//! `display_zoom`; "normalised" coordinates are the same plane scaled to
//! 0..1. Angles are radians, courses are clockwise from north.

/// Side of a raster tile in pixels.
pub const TILE_SIZE: u32 = 256;

/// Longitude/latitude to normalised Mercator coordinates in 0..1.
pub fn project_norm(lon: f64, lat: f64) -> (f64, f64) {
    let lat_rad = lat.to_radians();
    let x = (lon + 180.0) / 360.0;
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0;
    (x, y)
}

/// Pixels across the whole world at `display_zoom`.
pub(super) fn world_scale(display_zoom: f64) -> f64 {
    TILE_SIZE as f64 * 2.0_f64.powf(display_zoom)
}

/// Longitude/latitude to world pixels at `display_zoom`.
pub(super) fn lonlat_to_world(lon: f64, lat: f64, display_zoom: f64) -> (f64, f64) {
    let (x, y) = project_norm(lon, lat);
    let scale = world_scale(display_zoom);
    (x * scale, y * scale)
}

/// World pixels at `display_zoom` back to longitude/latitude.
pub(super) fn world_to_lonlat(x: f64, y: f64, display_zoom: f64) -> (f64, f64) {
    let scale = world_scale(display_zoom);
    let lon = x / scale * 360.0 - 180.0;
    let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * y / scale;
    let lat = n.sinh().atan().to_degrees();
    (lon, lat)
}

/// Axis-aligned size of a `width` x `height` rectangle after rotating it by `angle`.
pub(super) fn rotated_bounds(width: u32, height: u32, angle: f64) -> (u32, u32) {
    let (sin, cos) = angle.sin_cos();
    let ceil_bound = |value: f64| (value - 1e-9).ceil().max(1.0) as u32;
    (
        ceil_bound(width as f64 * cos.abs() + height as f64 * sin.abs()),
        ceil_bound(width as f64 * sin.abs() + height as f64 * cos.abs()),
    )
}

/// The zoom level to read tiles from while displaying `display_zoom`:
/// the nearest level the chart has, unless that would mean shrinking tiles
/// by more than a factor of two, in which case the nearest lower level.
pub(super) fn source_zoom_for(display_zoom: f64, available_zooms: &[i32], fallback: i32) -> i32 {
    let lower_zoom = available_zooms
        .iter()
        .copied()
        .filter(|&zoom| zoom as f64 <= display_zoom)
        .max();
    let nearest_zoom = available_zooms.iter().copied().min_by(|a, b| {
        (*a as f64 - display_zoom)
            .abs()
            .total_cmp(&(*b as f64 - display_zoom).abs())
    });

    match (nearest_zoom, lower_zoom) {
        (Some(nearest), Some(lower)) if nearest as f64 > display_zoom + 1.0 => lower,
        (Some(nearest), _) => nearest,
        (None, _) => fallback,
    }
}

/// Size of the source-zoom image needed to fill `width` x `height` display
/// pixels when the source is scaled by `zoom_scale`.
pub(super) fn source_dimensions(width: u32, height: u32, zoom_scale: f64) -> (u32, u32) {
    if (zoom_scale - 1.0).abs() < f64::EPSILON {
        return (width, height);
    }
    (
        (width as f64 / zoom_scale).ceil() as u32 + 2,
        (height as f64 / zoom_scale).ceil() as u32 + 2,
    )
}

/// Smoothstep easing for the zoom animation.
pub(super) fn smooth_zoom_progress(progress: f64) -> f64 {
    progress * progress * (3.0 - 2.0 * progress)
}

/// A true course as it appears on a chart rotated by `rotation`.
pub(super) fn course_to_screen(course: f64, rotation: f64) -> f64 {
    (course - rotation).rem_euclid(std::f64::consts::TAU)
}

/// Rotates a world-space offset into screen space.
pub(super) fn world_to_screen_delta(dx: f64, dy: f64, angle: f64) -> (f64, f64) {
    let (sin, cos) = angle.sin_cos();
    (dx * cos + dy * sin, -dx * sin + dy * cos)
}

/// Rotates a screen-space offset back into world space.
pub(super) fn screen_to_world_delta(dx: f64, dy: f64, angle: f64) -> (f64, f64) {
    let (sin, cos) = angle.sin_cos();
    (dx * cos - dy * sin, dx * sin + dy * cos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_projection_round_trips_at_a_fractional_zoom() {
        let (lon, lat) = (-4.4861, 48.3904); // Brest
        let (x, y) = lonlat_to_world(lon, lat, 9.6);
        let (lon2, lat2) = world_to_lonlat(x, y, 9.6);
        assert!((lon - lon2).abs() < 1e-9, "lon {lon2}");
        assert!((lat - lat2).abs() < 1e-9, "lat {lat2}");
    }

    #[test]
    fn the_world_is_one_tile_wide_at_zoom_zero_and_doubles_per_level() {
        assert_eq!(world_scale(0.0), 256.0);
        assert_eq!(world_scale(3.0), 2048.0);
        // Greenwich on the equator sits in the middle of the world.
        assert_eq!(lonlat_to_world(0.0, 0.0, 0.0), (128.0, 128.0));
    }

    #[test]
    fn course_up_rotates_eastward_course_to_screen_top() {
        let (x, y) = world_to_screen_delta(1.0, 0.0, std::f64::consts::FRAC_PI_2);
        assert!(x.abs() < 1e-12);
        assert!((y + 1.0).abs() < 1e-12);
    }

    #[test]
    fn screen_delta_rotation_round_trips() {
        let original = (125.0, -42.0);
        let angle = 1.37;
        let screen = world_to_screen_delta(original.0, original.1, angle);
        let world = screen_to_world_delta(screen.0, screen.1, angle);

        assert!((world.0 - original.0).abs() < 1e-10);
        assert!((world.1 - original.1).abs() < 1e-10);
    }

    #[test]
    fn rotated_view_bounds_cover_swapped_quarter_turn() {
        let (width, height) = rotated_bounds(1600, 800, std::f64::consts::FRAC_PI_2);
        assert_eq!((width, height), (800, 1600));
    }

    #[test]
    fn texture_source_dimensions_follow_zoom_scale() {
        assert_eq!(source_dimensions(1600, 800, 1.0), (1600, 800));
        assert_eq!(source_dimensions(1600, 800, 2.0), (802, 402));
        assert_eq!(source_dimensions(1600, 800, 0.5), (3202, 1602));
    }

    #[test]
    fn course_arrow_points_up_when_course_up_is_enabled() {
        let screen_course =
            course_to_screen(std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        assert!(screen_course.abs() < 1e-12);
    }

    #[test]
    fn zoom_transition_uses_smoothstep_endpoints() {
        assert_eq!(smooth_zoom_progress(0.0), 0.0);
        assert_eq!(smooth_zoom_progress(0.5), 0.5);
        assert_eq!(smooth_zoom_progress(1.0), 1.0);
    }

    #[test]
    fn source_zoom_avoids_extreme_downscaling_for_sparse_charts() {
        assert_eq!(source_zoom_for(12.0, &[4, 18], 12), 4);
    }
}
