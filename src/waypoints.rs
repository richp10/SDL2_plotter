use serde::{Deserialize, Serialize};
use std::{io, path::Path};

use crate::geo_data::{GeoFile, GeoRecord};
use crate::mbtiles_renderer::MBTilesRenderer;
use crate::screen::Screen;

const WAYPOINTS_FILE: &str = "default/waypoints.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Waypoint {
    longitude: f64,
    latitude: f64,
    #[serde(default)]
    name: String,
}

impl GeoRecord for Waypoint {
    fn position(&self) -> (f64, f64) {
        (self.longitude, self.latitude)
    }
}

/// The shape waypoints.json had before it joined the versioned format: a
/// bare array of `{lon, lat, name?}`.
#[derive(Deserialize)]
struct LegacyWaypoint {
    lon: f64,
    lat: f64,
    #[serde(default)]
    name: String,
}

pub struct Waypoints {
    waypoints: Vec<Waypoint>,
    /// The file this list is saved to.
    file: GeoFile<Waypoint>,
}

impl Waypoints {
    pub fn new() -> Self {
        Self::load(Path::new(WAYPOINTS_FILE))
    }

    /// Loads the list saved at `path`. A missing file is an empty list, a
    /// damaged one is set aside, and the old bare-array shape is migrated
    /// once (see `geo_data::GeoFile`). `save` writes back to the same path.
    pub fn load(path: &Path) -> Self {
        let file = Self::file_at(path);
        let waypoints = file.load();
        crate::diagnostics::record(
            "INFO",
            format!("Waypoints chargés: {} points", waypoints.len()),
        );
        Self { waypoints, file }
    }

    /// An empty list that `save` will write to `path`.
    #[cfg(test)]
    pub(crate) fn empty_at(path: &Path) -> Self {
        Self {
            waypoints: Vec::new(),
            file: Self::file_at(path),
        }
    }

    fn file_at(path: &Path) -> GeoFile<Waypoint> {
        GeoFile::new(path).with_legacy_shape(|contents| {
            serde_json::from_str::<Vec<LegacyWaypoint>>(contents)
                .ok()
                .map(|old| {
                    old.into_iter()
                        .map(|w| Waypoint {
                            longitude: w.lon,
                            latitude: w.lat,
                            name: w.name,
                        })
                        .collect()
                })
        })
    }

    pub fn add(&mut self, pos: (f64, f64), name: String) {
        let (longitude, latitude) = pos;
        if !longitude.is_finite()
            || !latitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude)
            || !(-90.0..=90.0).contains(&latitude)
        {
            return;
        }
        self.waypoints.push(Waypoint {
            longitude,
            latitude,
            name,
        });
    }

    pub fn modify(&mut self, i: usize, pos: (f64, f64)) {
        let (longitude, latitude) = pos;
        if let Some(waypoint) = self.waypoints.get_mut(i) {
            waypoint.longitude = longitude;
            waypoint.latitude = latitude;
        }
    }

    pub fn find_point_at(
        &self,
        renderer: &MBTilesRenderer,
        mouse: (i32, i32),
        radius: i32,
    ) -> Option<usize> {
        let r2 = (radius as i64) * (radius as i64);
        self.waypoints
            .iter()
            .enumerate()
            .map(|(i, waypoint)| {
                let pt = renderer.gps_to_point((waypoint.longitude, waypoint.latitude));
                let dx = pt.x as i64 - mouse.0 as i64;
                let dy = pt.y as i64 - mouse.1 as i64;
                (i, dx * dx + dy * dy)
            })
            .filter(|&(_, d2)| d2 <= r2)
            .min_by_key(|&(_, d2)| d2)
            .map(|(i, _)| i)
    }

    pub fn remove(&mut self, i: usize) {
        self.waypoints.remove(i);
    }

    pub fn show(&self, screen: &mut Screen<'_>, renderer: &MBTilesRenderer) -> Result<(), String> {
        for (i, wp) in self.waypoints.iter().enumerate() {
            let point = renderer.gps_to_point((wp.longitude, wp.latitude));
            if screen.point_i16(point).is_none() {
                continue; // off screen: neither the pin nor its label is drawn
            }
            screen.show_waypoint(point)?;
            let label = if wp.name.is_empty() {
                format!("Waypoint {}", i + 1)
            } else {
                wp.name.clone()
            };
            screen.show_text(&label, point.x + 10, point.y - 12)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.waypoints.clear();
        self.waypoints.shrink_to_fit();
    }

    pub fn save(&self) -> io::Result<()> {
        self.file.save(&self.waypoints)
    }

    pub fn export_gpx(&self, path: &Path) -> io::Result<()> {
        let document = crate::gpx::write_waypoints(
            self.waypoints
                .iter()
                .map(|w| (w.name.as_str(), (w.longitude, w.latitude))),
        );
        crate::geo_data::atomic_write(path, document.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "plotter-waypoints-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn save_and_load_waypoints() {
        let path = temp_path("roundtrip");
        let mut original = Waypoints::empty_at(&path);
        original.add((178.1, -17.7), "Mouillage".to_string());
        original.save().unwrap();

        let loaded = Waypoints::load(&path);

        assert_eq!(loaded.waypoints.len(), 1);
        assert_eq!(loaded.waypoints[0].longitude, 178.1);
        assert_eq!(loaded.waypoints[0].latitude, -17.7);
        assert_eq!(loaded.waypoints[0].name, "Mouillage");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_creates_missing_parent_directories_and_leaves_no_temp_file() {
        let dir = temp_path("dir").with_extension("");
        let path = dir.join("default").join("waypoints.json");
        let mut waypoints = Waypoints::empty_at(&path);
        waypoints.add((178.1, -17.7), "Mouillage".to_string());

        waypoints.save().unwrap();

        assert_eq!(Waypoints::load(&path).waypoints.len(), 1);
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["waypoints.json".to_string()]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_old_bare_array_format_is_migrated_and_names_default_to_empty() {
        let path = temp_path("legacy");
        fs::write(&path, r#"[{"lon":178.1,"lat":-17.7}]"#).unwrap();

        let loaded = Waypoints::load(&path);

        assert_eq!(loaded.waypoints.len(), 1);
        assert_eq!(loaded.waypoints[0].name, "");
        assert!(
            fs::read_to_string(&path).unwrap().contains("\"version\""),
            "rewritten in the versioned format"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_file_is_a_valid_empty_startup() {
        let path = temp_path("missing");
        assert!(Waypoints::load(&path).waypoints.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn a_corrupt_waypoint_file_is_set_aside_and_loads_empty() {
        let path = temp_path("corrupt");
        fs::write(&path, "{ nope").unwrap();
        assert!(Waypoints::load(&path).waypoints.is_empty());
        assert!(path.with_extension("json.corrupt").exists());
        fs::remove_file(path.with_extension("json.corrupt")).unwrap();
    }

    #[test]
    fn exports_named_waypoints_as_gpx_and_escapes_xml() {
        let path = temp_path("export").with_extension("gpx");
        let mut waypoints = Waypoints::empty_at(Path::new("unused.json"));
        waypoints.add((-122.3, 47.5), "A & B <C>".to_string());

        waypoints.export_gpx(&path).unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert!(contents.contains(r#"<wpt lat="47.500000" lon="-122.300000">"#));
        assert!(contents.contains("<name>A &amp; B &lt;C&gt;</name>"));
        fs::remove_file(path).unwrap();
    }
}
