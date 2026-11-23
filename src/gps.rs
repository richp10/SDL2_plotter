//! Great-circle geometry on a spherical Earth.
//!
//! Every point is a `(lon, lat)` tuple in degrees, the same order as the rest
//! of the code base. Distances come in nautical miles or metres; bearings are
//! initial (forward) bearings in degrees clockwise from true north, 0..360.

/// Mean Earth radius in nautical miles.
pub const EARTH_RADIUS_NM: f64 = 3440.065;
/// Mean Earth radius in metres.
pub const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

/// Central angle in radians between two `(lon, lat)` points (haversine).
fn central_angle(a: (f64, f64), b: (f64, f64)) -> f64 {
	let (lon1, lat1) = a;
	let (lon2, lat2) = b;
	let lat1_rad = lat1.to_radians();
	let lat2_rad = lat2.to_radians();
	let delta_lat = (lat2 - lat1).to_radians();
	let delta_lon = (lon2 - lon1).to_radians();

	let h = (delta_lat / 2.0).sin().powi(2)
		+ lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
	// Rounding can push h a hair above 1 for antipodal points; asin would then be NaN.
	2.0 * h.sqrt().min(1.0).asin()
}

/// Great-circle distance in nautical miles between two `(lon, lat)` points.
pub fn distance_nm(a: (f64, f64), b: (f64, f64)) -> f64 {
	EARTH_RADIUS_NM * central_angle(a, b)
}

/// Great-circle distance in metres between two `(lon, lat)` points.
pub fn distance_meters(a: (f64, f64), b: (f64, f64)) -> f64 {
	EARTH_RADIUS_METERS * central_angle(a, b)
}

/// Initial bearing in degrees (0..360, clockwise from true north) from `a` to `b`.
pub fn bearing_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
	let (lon1, lat1) = a;
	let (lon2, lat2) = b;
	let lat1_rad = lat1.to_radians();
	let lat2_rad = lat2.to_radians();
	let delta_lon = (lon2 - lon1).to_radians();

	let y = delta_lon.sin() * lat2_rad.cos();
	let x = lat1_rad.cos() * lat2_rad.sin() - lat1_rad.sin() * lat2_rad.cos() * delta_lon.cos();
	(y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Distance in nautical miles and initial bearing in degrees from `a` to `b`.
pub fn distance_bearing_nm(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
	(distance_nm(a, b), bearing_deg(a, b))
}

#[cfg(test)]
mod tests {
	use super::*;

	// Independent source of truth: one degree of arc on a sphere of radius R is
	// R * pi / 180. With the radii above that is 60.04 nautical miles and
	// 111 195 metres. Points are (lon, lat) throughout.
	const ONE_DEGREE_NM: f64 = 60.04;
	const ONE_DEGREE_METERS: f64 = 111_195.0;

	#[test]
	fn one_degree_of_latitude_is_sixty_nautical_miles() {
		let d = distance_nm((0.0, 0.0), (0.0, 1.0));
		assert!((d - ONE_DEGREE_NM).abs() < 0.01, "got {d}");
	}

	#[test]
	fn one_degree_of_latitude_in_metres() {
		let d = distance_meters((0.0, 0.0), (0.0, 1.0));
		assert!((d - ONE_DEGREE_METERS).abs() < 1.0, "got {d}");
	}

	#[test]
	fn points_are_lon_lat_so_a_latitude_step_at_any_longitude_is_one_degree() {
		// If the arguments were read as (lat, lon) this would be a one degree
		// step of longitude at 47.5 N, about 40 nautical miles, not 60.
		let d = distance_nm((-122.3, 47.5), (-122.3, 48.5));
		assert!((d - ONE_DEGREE_NM).abs() < 0.01, "got {d}");
	}

	#[test]
	fn distance_is_zero_for_the_same_point_and_symmetric() {
		let a = (-4.5, 48.4);
		let b = (-1.6, 50.1);
		assert_eq!(distance_nm(a, a), 0.0);
		assert!((distance_nm(a, b) - distance_nm(b, a)).abs() < 1e-9);
	}

	#[test]
	fn bearing_follows_the_compass() {
		let origin = (0.0, 0.0);
		assert!((bearing_deg(origin, (0.0, 1.0)) - 0.0).abs() < 1e-9, "north");
		assert!((bearing_deg(origin, (1.0, 0.0)) - 90.0).abs() < 1e-9, "east");
		assert!((bearing_deg(origin, (0.0, -1.0)) - 180.0).abs() < 1e-9, "south");
		assert!((bearing_deg(origin, (-1.0, 0.0)) - 270.0).abs() < 1e-9, "west");
	}

	#[test]
	fn distance_and_bearing_agree_with_the_separate_functions() {
		let a = (-4.5, 48.4);
		let b = (-1.6, 50.1);
		let (d, brg) = distance_bearing_nm(a, b);
		assert_eq!(d, distance_nm(a, b));
		assert_eq!(brg, bearing_deg(a, b));
	}

	#[test]
	fn antipodal_points_do_not_produce_nan() {
		let d = distance_nm((0.0, 0.0), (180.0, 0.0));
		assert!(d.is_finite());
		assert!((d - 180.0 * ONE_DEGREE_NM).abs() < 1.0, "got {d}");
	}
}
