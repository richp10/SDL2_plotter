use sdl2::pixels::Color;
use sdl2::rect::Point;
use sdl2::render::WindowCanvas;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};
use chrono::Local;

use crate::mbtiles_renderer::{project_norm, MBTilesRenderer};

const MIN_TRACK_DISTANCE_METERS: f64 = 20.0;
const SIMPLIFICATION_TOLERANCE_METERS: f64 = 20.0;
const MAX_SIMPLIFIED_SEGMENT_METERS: f64 = 1_500.0;
const EARTH_RADIUS_METERS: f64 = 6_371_000.0;
#[cfg(test)]
const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct TrackPoint {
    lon: f64,
    lat: f64,
    nx: f64,
    ny: f64,
}

pub struct Track {
    points: Vec<TrackPoint>,
    save_pending: bool,
    last_save: Instant,
    checkpoint_interval: Duration,
	}

impl Track 
	{
    #[cfg(test)]
    pub fn new(path: &Path) -> io::Result<Self> {
		if path.extension().is_some_and(|extension| extension == "gpx") {
			let mut track = Self {
				points: Vec::new(),
				save_pending: false,
				last_save: Instant::now(),
				checkpoint_interval: DEFAULT_CHECKPOINT_INTERVAL,
			};
			match Self::load_gpx(path) {
				Ok(positions) => track.set_points(positions),
				Err(error) if error.kind() == io::ErrorKind::NotFound => {}
				Err(error) => return Err(error),
			}
			Ok(track)
		} else {
			Self::with_checkpoint_interval(path, DEFAULT_CHECKPOINT_INTERVAL)
		}
    }

    pub fn with_checkpoint_interval(path: &Path, checkpoint_interval: Duration) -> io::Result<Self>
		{
        let mut track = Self
			{
			points: Vec::new(), 
			save_pending: false,
			last_save: Instant::now(),
			checkpoint_interval,
			};

		let legacy_path = path.with_extension("gpx");
		let migrating = !path.exists() && legacy_path.exists();
		let positions = crate::geo_data::read_or_migrate(path, &legacy_path, Self::load_gpx)?;
		track.set_points(positions);
		println!("Trace chargée : {} / {} points", path.display(), track.points.len());
		if migrating {
			crate::diagnostics::record(
				"INFO",
				format!("Trace migrée de {} vers {}", legacy_path.display(), path.display()),
			);
		}

		Ok(track)
		}

    
    pub fn reset(&mut self) 
		{
		self.points.clear();
		self.points.shrink_to_fit();
		self.save_pending = true;
		}

	pub fn point_count(&self) -> usize {
		self.points.len()
	}

	pub fn save_pending(&self) -> bool {
		self.save_pending
	}

	pub fn checkpoint_due(&self) -> bool {
		self.save_pending && self.last_save.elapsed() >= self.checkpoint_interval
	}

	pub fn save_default_track(&mut self, path: &Path) {
		self.last_save = Instant::now();
		crate::diagnostics::record(
			"INFO",
			format!(
				"Sauvegarde de la trace démarrée: {} points vers {}",
				self.point_count(),
				path.display()
			),
		);

		if self.points.is_empty() {
			if self.save_pending {
				match std::fs::remove_file(path) {
					Ok(()) => crate::diagnostics::record("INFO", "Trace vide supprimée après effacement"),
					Err(error) if error.kind() == io::ErrorKind::NotFound => {}
					Err(error) => {
						crate::diagnostics::record(
							"ERROR",
							format!("Impossible de supprimer la trace effacée: {error}"),
						);
						return;
					}
				}
				self.save_pending = false;
			}
			return;
		}

		match self.save(path) {
			Ok(()) => {
				self.save_pending = false;
				crate::diagnostics::record("INFO", "Trace sauvegardée avec succès");
			}
			Err(error) => {
				crate::diagnostics::record("ERROR", format!("Erreur sauvegarde trace: {error}"));
				eprintln!("Erreur sauvegarde : {error}");
			}
		}
	}

	fn load_gpx(path: &Path) -> io::Result<Vec<(f64, f64)>> {
		let text = std::fs::read_to_string(path)?;
		let mut positions = Vec::new();
		for chunk in text.split('<').skip(1) {
			let tag = chunk.split('>').next().unwrap_or("").trim();
			if !is_track_point_tag(tag) {
				continue;
			}
			let lat = parse_gpx_attr(tag, "lat")
				.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "latitude GPX manquante ou invalide"))?;
			let lon = parse_gpx_attr(tag, "lon")
				.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "longitude GPX manquante ou invalide"))?;
			if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
				return Err(io::Error::new(io::ErrorKind::InvalidData, "latitude GPX hors limites"));
			}
			if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
				return Err(io::Error::new(io::ErrorKind::InvalidData, "longitude GPX hors limites"));
			}
			positions.push((lon, lat));
		}
		Ok(positions)
	}

	fn set_points(&mut self, positions: Vec<(f64, f64)>) {
		self.points = positions
			.into_iter()
			.map(|(lon, lat)| {
				let (nx, ny) = project_norm(lon, lat);
				TrackPoint { lon, lat, nx, ny }
			})
			.collect();
	}
		
	fn push_point(&mut self, pos: (f64, f64)) 
		{
		let (nx, ny) = project_norm(pos.0, pos.1);
		self.points.push(TrackPoint { lon: pos.0, lat: pos.1, nx, ny });
		}
    /// pos = (lon, lat)
	pub fn add(&mut self, pos: (f64, f64)) -> bool
		{
		if !pos.0.is_finite()
			|| !pos.1.is_finite()
			|| !(-180.0..=180.0).contains(&pos.0)
			|| !(-90.0..=90.0).contains(&pos.1)
			{
			return false;
			}
		if let Some(last) = self.points.last() {
			if distance_meters((last.lon, last.lat), pos) < MIN_TRACK_DISTANCE_METERS {
				return false;
			}
		}
		self.push_point(pos);
		self.save_pending = true;
		true
		}
		
    pub fn show(&self, canvas: &mut WindowCanvas, map: &MBTilesRenderer, boat: Option<(f64, f64)>) 
		{
		if self.points.is_empty() 
			{
			return;
			}

		let mut pts: Vec<Point> = self.points
			.iter()
			.map(|p| map.norm_to_point(p.nx, p.ny))
			.collect();

		if let Some((lon, lat)) = boat
			{
			let (nx, ny) = project_norm(lon, lat);
			pts.push(map.norm_to_point(nx, ny));
			}

		if pts.len() < 2 
			{
			return;
			}

		canvas.set_draw_color(Color::RGB(255, 0, 0));
		let _ = canvas.draw_lines(pts.as_slice());
		}
	
	pub fn save(&self, path: &Path) -> io::Result<()> {
		let positions = self.points.iter().map(|point| (point.lon, point.lat)).collect::<Vec<_>>();
		crate::geo_data::write(path, &positions)
	}

	pub fn export_gpx(&self, path: &Path) -> io::Result<()> {
		if self.points.is_empty() {
			return Err(io::Error::new(io::ErrorKind::InvalidInput, "trace vide"));
		}
		if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
			fs::create_dir_all(parent)?;
		}
		let file_name = path
			.file_name()
			.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nom de fichier GPX manquant"))?;
		let temp_path = path.with_file_name(format!(
			".{}.{}.tmp",
			file_name.to_string_lossy(),
			std::process::id()
		));
		let result = (|| {
			let now = Local::now();
			let mut writer = BufWriter::new(File::create(&temp_path)?);
			writeln!(writer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
			writeln!(writer, r#"<gpx version="1.1" creator="boat-tracker" xmlns="http://www.topografix.com/GPX/1/1">"#)?;
			writeln!(writer, "  <trk>")?;
			writeln!(writer, "    <name>Track {}</name>", now.format("%Y-%m-%d %H:%M:%S"))?;
			writeln!(writer, "    <trkseg>")?;
			for index in simplified_indices(&self.points) {
				let point = &self.points[index];
				writeln!(writer, r#"      <trkpt lat="{:.6}" lon="{:.6}"/>"#, point.lat, point.lon)?;
			}
			writeln!(writer, "    </trkseg>")?;
			writeln!(writer, "  </trk>")?;
			writeln!(writer, "</gpx>")?;
			writer.flush()?;
			writer.get_ref().sync_all()?;
			drop(writer);
			fs::rename(&temp_path, path)
		})();
		if result.is_err() {
			let _ = fs::remove_file(temp_path);
		}
		result
	}
	}

fn is_track_point_tag(tag: &str) -> bool
	{
	let Some(rest) = tag.strip_prefix("trkpt") else
        {
        return false;
        };
	rest.is_empty() || rest.starts_with('/') || rest.chars().next().is_some_and(char::is_whitespace)
	}

fn parse_gpx_attr(tag: &str, name: &str) -> Option<f64>
	{
	let key = format!("{name}=");
	let start = tag.find(&key)? + key.len();
	let value = tag[start..].trim_start();
	let quote = value.chars().next()?;
	if quote != '"' && quote != '\''
        {
        return None;
        }
	let value = &value[quote.len_utf8()..];
	let end = value.find(quote)?;
	value[..end].parse().ok()
	}

fn distance_meters(a: (f64, f64), b: (f64, f64)) -> f64 {
	let (lat1, lat2) = (a.1.to_radians(), b.1.to_radians());
	let delta_lat = lat2 - lat1;
	let delta_lon = (b.0 - a.0).to_radians();
	let haversine = (delta_lat / 2.0).sin().powi(2)
		+ lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
	2.0 * EARTH_RADIUS_METERS * haversine.sqrt().asin()
}

fn simplified_indices(points: &[TrackPoint]) -> Vec<usize> {
	if points.len() <= 2 {
		return (0..points.len()).collect();
	}

	let mut retained = vec![false; points.len()];
	retained[0] = true;
	retained[points.len() - 1] = true;
	let mut segments = vec![(0, points.len() - 1)];

	while let Some((start, end)) = segments.pop() {
		if end <= start + 1 {
			continue;
		}

		let a = (points[start].lon, points[start].lat);
		let c = (points[end].lon, points[end].lat);
		let mut farthest = None;
		let mut max_deviation = 0.0;

		for index in start + 1..end {
			let point = (points[index].lon, points[index].lat);
			let deviation = distance_to_segment_meters(a, point, c);
			if deviation > max_deviation {
				max_deviation = deviation;
				farthest = Some(index);
			}
		}

		let split = if max_deviation > SIMPLIFICATION_TOLERANCE_METERS {
			farthest
		} else if distance_meters(a, c) > MAX_SIMPLIFIED_SEGMENT_METERS {
			Some(midpoint_by_distance(points, start, end))
		} else {
			None
		};

		if let Some(index) = split {
			retained[index] = true;
			segments.push((start, index));
			segments.push((index, end));
		}
	}

	retained
		.into_iter()
		.enumerate()
		.filter_map(|(index, keep)| keep.then_some(index))
		.collect()
}

fn midpoint_by_distance(points: &[TrackPoint], start: usize, end: usize) -> usize {
	let total_distance = (start..end)
		.map(|index| {
			distance_meters(
				(points[index].lon, points[index].lat),
				(points[index + 1].lon, points[index + 1].lat),
			)
		})
		.sum::<f64>();
	let half_distance = total_distance / 2.0;
	let mut accumulated = 0.0;

	for index in start + 1..end {
		accumulated += distance_meters(
			(points[index - 1].lon, points[index - 1].lat),
			(points[index].lon, points[index].lat),
		);
		if accumulated >= half_distance {
			return index;
		}
	}

	start + (end - start) / 2
}

fn distance_to_segment_meters(
	start: (f64, f64),
	point: (f64, f64),
	end: (f64, f64),
) -> f64 {
	let lat_origin = start.1.to_radians();
	let local = |position: (f64, f64)| {
		let delta_lon = (position.0 - start.0 + 180.0).rem_euclid(360.0) - 180.0;
		(
			delta_lon.to_radians() * lat_origin.cos() * EARTH_RADIUS_METERS,
			(position.1 - start.1).to_radians() * EARTH_RADIUS_METERS,
		)
	};

	let (px, py) = local(point);
	let (ex, ey) = local(end);
	let length_squared = ex * ex + ey * ey;
	if length_squared == 0.0 {
		return px.hypot(py);
	}

	let projection = ((px * ex + py * ey) / length_squared).clamp(0.0, 1.0);
	(px - projection * ex).hypot(py - projection * ey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn loads_track_points_from_gpx() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "desktop_plotter_track_load_{}_{}.gpx",
            std::process::id(),
            unique_id
        ));
        std::fs::write(
            &path,
            r#"<gpx><trk><trkseg><trkpt lat="47.5" lon="-122.3"/><trkpt lon="-122.2" lat="47.6"/></trkseg></trk></gpx>"#,
        )
        .unwrap();

        let track = Track::new(&path).unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(track.points.len(), 2);
        assert_eq!((track.points[0].lon, track.points[0].lat), (-122.3, 47.5));
        assert_eq!((track.points[1].lon, track.points[1].lat), (-122.2, 47.6));
    }

    #[test]
    fn starts_with_empty_track_when_gpx_has_no_points() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "desktop_plotter_track_empty_{}_{}.gpx",
            std::process::id(),
            unique_id
        ));
        std::fs::write(&path, "<gpx><trk><trkseg></trkseg></trk></gpx>").unwrap();

        let track = Track::new(&path).unwrap();
        std::fs::remove_file(path).unwrap();

        assert!(track.points.is_empty());
    }

    #[test]
    fn filters_positions_by_distance_from_last_recorded_point() {
        let mut track = Track::new(Path::new("missing-track.gpx")).unwrap();
        let start = (-122.3, 47.5);
        let under_20m = (-122.3, 47.5001);
        let over_20m = (-122.3, 47.5003);

        assert!(track.add(start));
        assert!(!track.add(under_20m));
        assert_eq!(track.points.len(), 1);

        assert!(track.add(over_20m));
        assert_eq!(track.points.len(), 2);
        assert_eq!(
            (track.points[1].lon, track.points[1].lat),
            over_20m
        );
    }

	#[test]
	fn saves_track_atomically_and_can_load_the_saved_file() {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_nanos();
		let path = std::env::temp_dir().join(format!(
			"desktop_plotter_track_save_{}_{}.json",
			std::process::id(),
			unique_id
		));
		let mut original = Track::new(Path::new("missing-track.gpx")).unwrap();
		original.add((-122.3, 47.5));
		original.add((-122.3, 47.5003));

		original.save(&path).unwrap();
		let loaded = Track::new(&path).unwrap();

		assert_eq!(loaded.point_count(), 2);
		assert!(!path.with_file_name(format!(
			".{}.{}.tmp",
			path.file_name().unwrap().to_string_lossy(),
			std::process::id()
		)).exists());
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn migrates_legacy_gpx_to_json_without_modifying_gpx() {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_nanos();
		let dir = std::env::temp_dir().join(format!(
			"desktop_plotter_track_migration_{}_{}",
			std::process::id(),
			unique_id
		));
		let json_path = dir.join("track.json");
		let gpx_path = dir.join("track.gpx");
		let source = r#"<gpx><trk><trkseg><trkpt lat="47.5" lon="-122.3"/><trkpt lat="47.6" lon="-122.2"/></trkseg></trk></gpx>"#;
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(&gpx_path, source).unwrap();

		let track = Track::with_checkpoint_interval(&json_path, Duration::from_secs(60)).unwrap();

		assert_eq!(track.point_count(), 2);
		assert_eq!(crate::geo_data::read(&json_path).unwrap(), vec![(-122.3, 47.5), (-122.2, 47.6)]);
		assert_eq!(std::fs::read_to_string(&gpx_path).unwrap(), source);
		std::fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn clearing_track_removes_the_native_file_immediately() {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_nanos();
		let path = std::env::temp_dir().join(format!(
			"desktop_plotter_track_clear_{}_{}.json",
			std::process::id(),
			unique_id
		));
		let mut track =
			Track::with_checkpoint_interval(&path, Duration::from_secs(60)).unwrap();
		track.add((-122.3, 47.5));
		track.save(&path).unwrap();

		track.reset();
		track.save_default_track(&path);

		assert!(!path.exists());
	}

	#[test]
	fn exports_track_as_gpx_separately_from_native_json() {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_nanos();
		let dir = std::env::temp_dir().join(format!(
			"desktop_plotter_track_export_{}_{}",
			std::process::id(),
			unique_id
		));
		let json_path = dir.join("track.json");
		let gpx_path = dir.join("export/track.gpx");
		let mut track = Track::with_checkpoint_interval(&json_path, Duration::from_secs(60)).unwrap();
		track.add((-122.3, 47.5));
		track.add((-122.2, 47.6));
		track.save(&json_path).unwrap();
		track.export_gpx(&gpx_path).unwrap();

		let exported = std::fs::read_to_string(&gpx_path).unwrap();
		assert!(exported.contains("<trkpt"));
		assert_eq!(crate::geo_data::read(&json_path).unwrap().len(), 2);
		std::fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn simplifies_straight_track_but_preserves_turns() {
		let straight = [
			(0.0, 0.0),
			(0.001, 0.0),
			(0.002, 0.0),
			(0.003, 0.0),
		]
		.into_iter()
		.map(|(lon, lat)| {
			let (nx, ny) = project_norm(lon, lat);
			TrackPoint { lon, lat, nx, ny }
		})
		.collect::<Vec<_>>();
		assert_eq!(simplified_indices(&straight), vec![0, 3]);

		let turning = [
			(0.0, 0.0),
			(0.001, 0.0),
			(0.001, 0.001),
			(0.002, 0.001),
			(0.002, 0.002),
		]
			.into_iter()
			.map(|(lon, lat)| {
				let (nx, ny) = project_norm(lon, lat);
				TrackPoint { lon, lat, nx, ny }
			})
			.collect::<Vec<_>>();
		let retained = simplified_indices(&turning);
		assert!(retained.contains(&1));
		assert!(retained.contains(&2));
		assert!(retained.contains(&3));
		assert_eq!(retained.first(), Some(&0));
		assert_eq!(retained.last(), Some(&4));
	}

	#[test]
	fn simplification_splits_inside_segment_with_long_final_leg() {
		let points = [(0.0, 0.0), (0.0002, 0.0), (0.02, 0.0)]
			.into_iter()
			.map(|(lon, lat)| {
				let (nx, ny) = project_norm(lon, lat);
				TrackPoint { lon, lat, nx, ny }
			})
			.collect::<Vec<_>>();

		assert_eq!(simplified_indices(&points), vec![0, 1, 2]);
	}

    #[test]
    fn save_creates_parent_directories() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "desktop_plotter_track_save_{}_{}",
            std::process::id(),
            unique_id
        ));
        let path = dir.join("track").join("default.json");

        let mut track = Track::new(Path::new("missing-track.gpx")).unwrap();
        track.add((-122.3, 47.5));
        track.save(&path).unwrap();

        assert!(path.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
