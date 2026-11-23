mod gpx;
const DEFAULT_ROUTE: &str = "default/route.json";
const LEGACY_ROUTE: &str = "default/route.gpx";
 
use crate::mbtiles_renderer::MBTilesRenderer;
use crate::screen::Map;
use crate::gps;
use crate::Mode;
use std::io;
use std::path::Path;
use sdl2::rect::Point;
 
 #[derive(Debug)]
 pub struct Route
	{
	track: Vec<(f64, f64)>,
	distance: Vec<f64>,
	restante: Vec<f64>,
	longueur :f64,
	}
	
impl Route
	{
	pub fn new() -> Result<Self, String> 
		{
		let json_path = Path::new(DEFAULT_ROUTE);
		let legacy_path = Path::new(LEGACY_ROUTE);
		let migrating = !json_path.exists() && legacy_path.exists();
		let track = crate::geo_data::read_or_migrate(
			json_path,
			legacy_path,
			gpx::load_gpx,
		)
		.map_err(|error| format!("impossible de charger la route: {error}"))?;
		if migrating {
			crate::diagnostics::record(
				"INFO",
				format!("Route migrée de {LEGACY_ROUTE} vers {DEFAULT_ROUTE}"),
			);
		}
		crate::diagnostics::record("INFO", format!("Route chargée: {} points", track.len()));
		Ok(Route::from_points(track))
		}

	fn from_points(track: Vec<(f64, f64)>) -> Self
		{
		let mut route = Route
			{
			track,
			distance: Vec::new(),
			restante: Vec::new(),
			longueur: 0.0,
			};
		route.calc_route();
		route
		}
	pub fn show_route_since_id(&mut self,renderer: &MBTilesRenderer,map: &mut Map,mode: Mode,id: usize)
		{
		// `id` may come from a saved state and exceed the current route length.
		let rte = self.track.get(id..).unwrap_or(&[]);
		if rte.len() >= 1 {
		    let pts: Vec<Point> = rte
						.iter()
						.map(|&pos| renderer.gps_to_point(pos))
						.collect();
		    map.show_route(&pts);
		    }
	     if mode == Mode::EditRoute
			{
			 if rte.len() >= 2
				{
				let mid_points = self.midpoints();
				let mid_pts: Vec<Point> = mid_points
						.get(id..)
						.unwrap_or(&[])
						.iter()
						.map(|&pos| renderer.gps_to_point(pos))
						.collect();
				map.draw_mid_handles(&mid_pts);
				}
			}
		}
	pub fn show_route_from_boat(&mut self,renderer: &MBTilesRenderer,map: &mut Map, mode: Mode,id: usize,boat: (f64,f64))
		{
		let Some(id) = self.active_waypoint(id) else {
			return;
		};
		map.show_route(&[
						renderer.gps_to_point(boat),
						renderer.gps_to_point(self.track[id])
						]);
		self.show_route_since_id(renderer, map,mode, id);
		}

	pub fn show_route_len(&self) -> String{
		// An empty sum is -0.0 in Rust, which would print as "-0.0".
		format!("route : {:.1} Mn", self.longueur.max(0.0))
		}

	/// Index of the waypoint being navigated to, clamped to the last point.
	/// `None` when the route is empty.
	pub fn active_waypoint(&self, id: usize) -> Option<usize>
		{
		let last = self.track.len().checked_sub(1)?;
		Some(id.min(last))
		}
			
		pub fn save(&self) -> io::Result<()> {
			crate::geo_data::write(Path::new(DEFAULT_ROUTE), &self.track)
		}

		pub fn export_gpx(&self, path: &Path) -> io::Result<()> {
			gpx::save_route_gpx(path, "Plotter", &self.track)
		}
		
		pub fn clear(&mut self) -> io::Result<()> {
		self.track.clear();
			self.distance.clear();
			self.restante.clear();
		self.longueur = 0.0;
			self.save()
		}
		
	pub fn remove(&mut self, i:usize) {
		self.track.remove(i);
		self.calc_route();
		}
		
	pub fn modify(&mut self, i:usize, longitude:f64, latitude:f64) {
		self.track[i] = (longitude, latitude);
		self.calc_route();
		}
		
	pub fn add(&mut self, longitude:f64, latitude:f64) {
		self.track.push((longitude,latitude));
		self.calc_route();
		}
		
	pub fn insert (&mut self, i:usize, longitude:f64, latitude:f64) {
		self.track.insert(i+1,(longitude,latitude));
		self.calc_route();
		}
		
	pub fn calc_route(&mut self) {
		self.distance.clear();

		for w in self.track.windows(2) 
			{
			self.distance.push(gps::distance_nm(w[0], w[1]));
			}

		self.longueur = self.distance.iter().sum();
		
		self.restante = vec![0.0; self.track.len()];
		let mut reste = 0.0;
		for i in (0..self.distance.len()).rev() 
			{
			reste += self.distance[i];
			self.restante[i] = reste;
			}
		}
		
	/// Distance and bearing from the boat to the active waypoint, plus the
	/// distance to the end of the route. `None` when the route is empty.
	pub fn calc_route_mn_from_boat(&self, id :usize, boat :(f64,f64)) -> Option<(f64,f64,f64)>{
		let id = self.active_waypoint(id)?;
		let (dist_wp, bearing_wp) = gps::distance_bearing_nm(boat, self.track[id]);
		let dist_total = dist_wp + self.restante[id];
		Some((dist_wp, bearing_wp, dist_total))
		}
		
	pub fn find_point_at(
		&mut self,
		renderer: &MBTilesRenderer,
		mouse: (i32, i32),
		radius: i32,
		) -> Option<usize> {
			let r2 = (radius as i64) * (radius as i64);
			self.track
				.iter()
				.enumerate()
				.map(|(i, &(lon, lat))| {
					let pt = renderer.gps_to_point((lon, lat));
					let dx = pt.x as i64 - mouse.0 as i64;
					let dy = pt.y as i64 - mouse.1 as i64;
					(i, dx * dx + dy * dy)
				})
				.filter(|&(_, d2)| d2 <= r2)
				.min_by_key(|&(_, d2)| d2)
				.map(|(i, _)| i)
				}
	
	pub fn find_midpoint_at(
		&mut self,
		renderer: &MBTilesRenderer,
		mouse: (i32, i32),
		radius: i32,
		) -> Option<usize> {
			let r2 = (radius as i64) * (radius as i64);
			self.midpoints()
				.iter()
				.enumerate()
				.map(|(i, &(lon, lat))| {
					let pt = renderer.gps_to_point((lon, lat));
					let dx = pt.x as i64 - mouse.0 as i64;
					let dy = pt.y as i64 - mouse.1 as i64;
					(i, dx * dx + dy * dy)
				})
				.filter(|&(_, d2)| d2 <= r2)
				.min_by_key(|&(_, d2)| d2)
				.map(|(i, _)| i)
				}
				
	pub fn midpoints(&mut self) -> Vec<(f64, f64)> {
		self.track
			.windows(2)
			.map(|w| ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0))
			.collect()
		}
		/*
	pub fn get_wp(&self, id: usize) -> Option<(f64, f64)> 
		{
		let last = self.track.len().checked_sub(1)?; // None si le Vec est vide
		Some(self.track[id.min(last)])
		}
	*/
	pub fn get_nb_wp(&self) -> usize{
		self.track.len()
		}
	}

#[cfg(test)]
mod tests {
	use super::Route;

	#[test]
	fn empty_route_has_no_active_waypoint_and_no_navigation_data() {
		let route = Route::from_points(Vec::new());
		assert_eq!(route.active_waypoint(0), None);
		assert_eq!(route.calc_route_mn_from_boat(0, (0.0, 0.0)), None);
	}

	#[test]
	fn active_waypoint_index_is_clamped_to_the_last_point() {
		let route = Route::from_points(vec![(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)]);
		assert_eq!(route.active_waypoint(1), Some(1));
		assert_eq!(route.active_waypoint(7), Some(2));
	}

	#[test]
	fn navigation_from_the_last_waypoint_position_reports_zero_distance() {
		let route = Route::from_points(vec![(0.0, 0.0), (0.0, 1.0)]);
		let (to_waypoint, _bearing, to_end) =
			route.calc_route_mn_from_boat(9, (0.0, 1.0)).unwrap();
		assert!(to_waypoint.abs() < 1e-9);
		assert!(to_end.abs() < 1e-9);
	}

	#[test]
	fn navigation_distance_includes_the_remaining_legs() {
		// 1 degree of latitude is 60 nautical miles.
		let route = Route::from_points(vec![(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)]);
		let (to_waypoint, bearing, to_end) =
			route.calc_route_mn_from_boat(1, (0.0, 0.0)).unwrap();
		assert!((to_waypoint - 60.0).abs() < 0.1, "{to_waypoint}");
		assert!(bearing.abs() < 1e-6, "{bearing}");
		assert!((to_end - 120.0).abs() < 0.2, "{to_end}");
	}

	#[test]
	fn empty_route_length_is_not_negative_zero() {
		let route = Route::from_points(Vec::new());
		assert_eq!(route.show_route_len(), "route : 0.0 Mn");
	}
}

