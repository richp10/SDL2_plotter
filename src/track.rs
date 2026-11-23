use chrono::Local;
use sdl2::rect::Point;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::geo_data::{GeoFile, GeoPoint};
use crate::gps::{distance_meters, EARTH_RADIUS_METERS};
use crate::mbtiles_renderer::{project_norm, MBTilesRenderer};
use crate::screen::Screen;

const MIN_TRACK_DISTANCE_METERS: f64 = 20.0;
const SIMPLIFICATION_TOLERANCE_METERS: f64 = 20.0;
const MAX_SIMPLIFIED_SEGMENT_METERS: f64 = 1_500.0;
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
    /// The file the track is checkpointed to and saved on exit.
    file: GeoFile<GeoPoint>,
    save_pending: bool,
    last_save: Instant,
    checkpoint_interval: Duration,
}

impl Track {
    /// A track over `path`. A `.gpx` path loads that file directly (tests);
    /// anything else is the versioned JSON file with the default checkpoint.
    #[cfg(test)]
    pub fn new(path: &Path) -> Self {
        if path.extension().is_some_and(|extension| extension == "gpx") {
            let mut track = Self::empty(GeoFile::new(path), DEFAULT_CHECKPOINT_INTERVAL);
            if let Ok(positions) = Self::load_gpx(path) {
                track.set_points(positions);
            }
            track
        } else {
            Self::with_checkpoint_interval(path, DEFAULT_CHECKPOINT_INTERVAL)
        }
    }

    /// Loads the track saved at `path`. A missing file is an empty track, a
    /// damaged one is set aside, and a legacy `.gpx` beside it is migrated
    /// once (see `geo_data::GeoFile`). Never fails: a saved file cannot stop
    /// the app starting.
    pub fn with_checkpoint_interval(path: &Path, checkpoint_interval: Duration) -> Self {
        let file = GeoFile::new(path).with_legacy_file("gpx", |legacy| {
            Self::load_gpx(legacy).map(|points| points.into_iter().map(Into::into).collect())
        });
        let positions: Vec<(f64, f64)> = file.load().into_iter().map(Into::into).collect();
        let mut track = Self::empty(file, checkpoint_interval);
        track.set_points(positions);
        crate::diagnostics::record(
            "INFO",
            format!(
                "Trace chargée: {} / {} points",
                path.display(),
                track.points.len()
            ),
        );
        track
    }

    fn empty(file: GeoFile<GeoPoint>, checkpoint_interval: Duration) -> Self {
        Self {
            points: Vec::new(),
            file,
            save_pending: false,
            last_save: Instant::now(),
            checkpoint_interval,
        }
    }

    pub fn reset(&mut self) {
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

    /// Checkpoints the track to its file, or removes the file when the track
    /// was cleared. Errors are logged, never propagated: the loop goes on.
    pub fn save_default_track(&mut self) {
        self.last_save = Instant::now();
        crate::diagnostics::record(
            "INFO",
            format!(
                "Sauvegarde de la trace démarrée: {} points vers {}",
                self.point_count(),
                self.file.path().display()
            ),
        );

        if self.points.is_empty() {
            if self.save_pending {
                match self.file.remove() {
                    Ok(()) => {
                        crate::diagnostics::record("INFO", "Trace vide supprimée après effacement")
                    }
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

        match self.save() {
            Ok(()) => {
                self.save_pending = false;
                crate::diagnostics::record("INFO", "Trace sauvegardée avec succès");
            }
            Err(error) => {
                crate::diagnostics::record("ERROR", format!("Erreur sauvegarde trace: {error}"));
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
            let lat = parse_gpx_attr(tag, "lat").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "latitude GPX manquante ou invalide",
                )
            })?;
            let lon = parse_gpx_attr(tag, "lon").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "longitude GPX manquante ou invalide",
                )
            })?;
            if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "latitude GPX hors limites",
                ));
            }
            if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "longitude GPX hors limites",
                ));
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

    fn push_point(&mut self, pos: (f64, f64)) {
        let (nx, ny) = project_norm(pos.0, pos.1);
        self.points.push(TrackPoint {
            lon: pos.0,
            lat: pos.1,
            nx,
            ny,
        });
    }
    /// pos = (lon, lat)
    pub fn add(&mut self, pos: (f64, f64)) -> bool {
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

    pub fn show(&self, screen: &mut Screen, renderer: &MBTilesRenderer, boat: Option<(f64, f64)>) {
        if self.points.is_empty() {
            return;
        }

        let mut pts: Vec<Point> = self
            .points
            .iter()
            .map(|p| renderer.norm_to_point(p.nx, p.ny))
            .collect();

        if let Some((lon, lat)) = boat {
            let (nx, ny) = project_norm(lon, lat);
            pts.push(renderer.norm_to_point(nx, ny));
        }

        // Decoration: a failed draw must not abort the frame.
        let _ = screen.show_track(&pts);
    }

    pub fn save(&self) -> io::Result<()> {
        let records: Vec<GeoPoint> = self
            .points
            .iter()
            .map(|point| GeoPoint::from((point.lon, point.lat)))
            .collect();
        self.file.save(&records)
    }

    pub fn export_gpx(&self, path: &Path) -> io::Result<()> {
        if self.points.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "trace vide"));
        }
        let now = Local::now();
        let mut gpx = String::new();
        gpx.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
",
        );
        gpx.push_str("<gpx version=\"1.1\" creator=\"boat-tracker\" xmlns=\"http://www.topografix.com/GPX/1/1\">
");
        gpx.push_str(
            "  <trk>
",
        );
        gpx.push_str(&format!(
            "    <name>Track {}</name>
",
            now.format("%Y-%m-%d %H:%M:%S")
        ));
        gpx.push_str(
            "    <trkseg>
",
        );
        for index in simplified_indices(&self.points) {
            let point = &self.points[index];
            gpx.push_str(&format!(
                "      <trkpt lat=\"{:.6}\" lon=\"{:.6}\"/>
",
                point.lat, point.lon
            ));
        }
        gpx.push_str(
            "    </trkseg>
",
        );
        gpx.push_str(
            "  </trk>
",
        );
        gpx.push_str(
            "</gpx>
",
        );
        crate::geo_data::atomic_write(path, gpx.as_bytes())
    }
}

fn is_track_point_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix("trkpt") else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/') || rest.chars().next().is_some_and(char::is_whitespace)
}

fn parse_gpx_attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=");
    let start = tag.find(&key)? + key.len();
    let value = tag[start..].trim_start();
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &value[quote.len_utf8()..];
    let end = value.find(quote)?;
    value[..end].parse().ok()
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

        for (index, point) in points.iter().enumerate().take(end).skip(start + 1) {
            let deviation = distance_to_segment_meters(a, (point.lon, point.lat), c);
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

fn distance_to_segment_meters(start: (f64, f64), point: (f64, f64), end: (f64, f64)) -> f64 {
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

        let track = Track::new(&path);
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

        let track = Track::new(&path);
        std::fs::remove_file(path).unwrap();

        assert!(track.points.is_empty());
    }

    #[test]
    fn filters_positions_by_distance_from_last_recorded_point() {
        let mut track = Track::new(Path::new("missing-track.gpx"));
        let start = (-122.3, 47.5);
        let under_20m = (-122.3, 47.5001);
        let over_20m = (-122.3, 47.5003);

        assert!(track.add(start));
        assert!(!track.add(under_20m));
        assert_eq!(track.points.len(), 1);

        assert!(track.add(over_20m));
        assert_eq!(track.points.len(), 2);
        assert_eq!((track.points[1].lon, track.points[1].lat), over_20m);
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
        let mut original = Track::new(&path);
        original.add((-122.3, 47.5));
        original.add((-122.3, 47.5003));

        original.save().unwrap();
        let loaded = Track::new(&path);

        assert_eq!(loaded.point_count(), 2);
        assert!(!path
            .with_file_name(format!(
                ".{}.{}.tmp",
                path.file_name().unwrap().to_string_lossy(),
                std::process::id()
            ))
            .exists());
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

        let track = Track::with_checkpoint_interval(&json_path, Duration::from_secs(60));

        assert_eq!(track.point_count(), 2);
        assert_eq!(
            crate::geo_data::read(&json_path).unwrap(),
            vec![(-122.3, 47.5), (-122.2, 47.6)]
        );
        assert_eq!(std::fs::read_to_string(&gpx_path).unwrap(), source);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_corrupt_track_file_is_set_aside_and_starts_empty() {
        let path = std::env::temp_dir().join(format!(
            "desktop_plotter_track_corrupt_{}_{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "garbage").unwrap();
        let track = Track::with_checkpoint_interval(&path, Duration::from_secs(60));
        assert_eq!(track.point_count(), 0);
        assert!(path.with_extension("json.corrupt").exists());
        std::fs::remove_file(path.with_extension("json.corrupt")).unwrap();
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
        let mut track = Track::with_checkpoint_interval(&path, Duration::from_secs(60));
        track.add((-122.3, 47.5));
        track.save().unwrap();

        track.reset();
        track.save_default_track();

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
        let mut track = Track::with_checkpoint_interval(&json_path, Duration::from_secs(60));
        track.add((-122.3, 47.5));
        track.add((-122.2, 47.6));
        track.save().unwrap();
        track.export_gpx(&gpx_path).unwrap();

        let exported = std::fs::read_to_string(&gpx_path).unwrap();
        assert!(exported.contains("<trkpt"));
        assert_eq!(crate::geo_data::read(&json_path).unwrap().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn simplifies_straight_track_but_preserves_turns() {
        let straight = [(0.0, 0.0), (0.001, 0.0), (0.002, 0.0), (0.003, 0.0)]
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

        let mut track = Track::new(&path);
        track.add((-122.3, 47.5));
        track.save().unwrap();

        assert!(path.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
