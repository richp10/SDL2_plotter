//! Collections saved as versioned JSON under `default/`, with the one policy
//! every such file must follow: a missing file is an empty collection, a
//! damaged one is set aside as `<name>.corrupt` and treated as empty, an
//! older file beside it or an older shape of it is migrated once, and every
//! write is atomic. A saved file can therefore never stop the application
//! from starting.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;

/// A record a [`GeoFile`] stores: serialisable, with a position to validate.
pub trait GeoRecord: Serialize + DeserializeOwned {
    /// `(lon, lat)` in degrees.
    fn position(&self) -> (f64, f64);
}

/// The record route and track files hold.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct GeoPoint {
    pub longitude: f64,
    pub latitude: f64,
}

impl GeoRecord for GeoPoint {
    fn position(&self) -> (f64, f64) {
        (self.longitude, self.latitude)
    }
}

impl From<(f64, f64)> for GeoPoint {
    fn from((longitude, latitude): (f64, f64)) -> Self {
        GeoPoint {
            longitude,
            latitude,
        }
    }
}

impl From<GeoPoint> for (f64, f64) {
    fn from(point: GeoPoint) -> Self {
        (point.longitude, point.latitude)
    }
}

#[derive(Deserialize)]
struct Envelope<R> {
    version: u32,
    points: Vec<R>,
}

#[derive(Serialize)]
struct EnvelopeRef<'a, R> {
    version: u32,
    points: &'a [R],
}

/// True when both coordinates are finite and within range.
pub fn valid_position((longitude, latitude): (f64, f64)) -> bool {
    longitude.is_finite()
        && (-180.0..=180.0).contains(&longitude)
        && latitude.is_finite()
        && (-90.0..=90.0).contains(&latitude)
}

/// Moves a damaged file to `<name>.corrupt` so it can be inspected while the
/// application starts with defaults. Returns the new path.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".corrupt");
    let aside = PathBuf::from(aside);
    std::fs::rename(path, &aside)?;
    Ok(aside)
}

type LegacyFile<R> = Box<dyn Fn(&Path) -> io::Result<Vec<R>>>;
type LegacyShape<R> = Box<dyn Fn(&str) -> Option<Vec<R>>>;

/// One collection at one path. See the module documentation for the policy.
pub struct GeoFile<R: GeoRecord> {
    path: PathBuf,
    legacy_file: Option<(String, LegacyFile<R>)>,
    legacy_shape: Option<LegacyShape<R>>,
}

impl<R: GeoRecord> std::fmt::Debug for GeoFile<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeoFile").field("path", &self.path).finish()
    }
}

impl<R: GeoRecord> GeoFile<R> {
    pub fn new(path: &Path) -> Self {
        GeoFile {
            path: path.to_path_buf(),
            legacy_file: None,
            legacy_shape: None,
        }
    }

    /// A neighbour `path.with_extension(extension)` that `load` reads when
    /// the versioned file is missing. Its records are saved in the versioned
    /// format on that first load; the neighbour itself is left untouched.
    /// `Err(NotFound)` from `load` means there is nothing to migrate.
    pub fn with_legacy_file(
        mut self,
        extension: &str,
        load: impl Fn(&Path) -> io::Result<Vec<R>> + 'static,
    ) -> Self {
        self.legacy_file = Some((extension.to_string(), Box::new(load)));
        self
    }

    /// An older shape of the same file, tried before an unparseable file is
    /// set aside. `None` means the contents are not that shape either.
    pub fn with_legacy_shape(mut self, parse: impl Fn(&str) -> Option<Vec<R>> + 'static) -> Self {
        self.legacy_shape = Some(Box::new(parse));
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved collection. Never fails: a missing file is empty, a damaged
    /// one is set aside and logged, an unreadable one is logged; all three
    /// return an empty collection so the application starts.
    pub fn load(&self) -> Vec<R> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => match Self::parse(&contents) {
                Ok(records) => records,
                Err(error) => {
                    if let Some(parse_legacy) = &self.legacy_shape {
                        if let Some(records) = parse_legacy(&contents) {
                            if records
                                .iter()
                                .all(|record| valid_position(record.position()))
                            {
                                self.migrate(&records, "ancien format");
                                return records;
                            }
                        }
                    }
                    self.set_aside_and_log(&error);
                    Vec::new()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.load_legacy_file(),
            Err(error) => {
                crate::diagnostics::record(
                    "ERROR",
                    format!(
                        "{} illisible ({error}); collection vide utilisée",
                        self.path.display()
                    ),
                );
                Vec::new()
            }
        }
    }

    /// Writes the collection atomically. Rejects an invalid coordinate
    /// without touching the file, so a bad point never reaches disk.
    pub fn save(&self, records: &[R]) -> io::Result<()> {
        if records
            .iter()
            .any(|record| !valid_position(record.position()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "longitude or latitude is outside valid range",
            ));
        }
        let envelope = EnvelopeRef {
            version: FORMAT_VERSION,
            points: records,
        };
        let contents = serde_json::to_vec_pretty(&envelope)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        atomic_write(&self.path, &contents)
    }

    /// Deletes the file; a file that is already gone is not an error.
    pub fn remove(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn parse(contents: &str) -> io::Result<Vec<R>> {
        let envelope: Envelope<R> = serde_json::from_str(contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if envelope.version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported geo data version {}", envelope.version),
            ));
        }
        if envelope
            .points
            .iter()
            .any(|record| !valid_position(record.position()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "longitude or latitude is outside valid range",
            ));
        }
        Ok(envelope.points)
    }

    fn load_legacy_file(&self) -> Vec<R> {
        let Some((extension, load)) = &self.legacy_file else {
            return Vec::new();
        };
        let legacy_path = self.path.with_extension(extension);
        match load(&legacy_path) {
            Ok(records)
                if records
                    .iter()
                    .all(|record| valid_position(record.position())) =>
            {
                self.migrate(&records, &legacy_path.display().to_string());
                records
            }
            Ok(_) => {
                crate::diagnostics::record(
                    "ERROR",
                    format!(
                        "{} contient des coordonnées invalides; ignoré",
                        legacy_path.display()
                    ),
                );
                Vec::new()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                crate::diagnostics::record(
                    "ERROR",
                    format!("{} illisible ({error}); ignoré", legacy_path.display()),
                );
                Vec::new()
            }
        }
    }

    fn migrate(&self, records: &[R], from: &str) {
        match self.save(records) {
            Ok(()) => crate::diagnostics::record(
                "INFO",
                format!(
                    "{} points migrés de {from} vers {}",
                    records.len(),
                    self.path.display()
                ),
            ),
            Err(error) => crate::diagnostics::record(
                "ERROR",
                format!(
                    "Migration vers {} impossible ({error}); données gardées en mémoire",
                    self.path.display()
                ),
            ),
        }
    }

    fn set_aside_and_log(&self, error: &io::Error) {
        match set_aside(&self.path) {
            Ok(aside) => crate::diagnostics::record(
                "ERROR",
                format!(
                    "{} illisible ({error}); déplacé vers {} et collection vide utilisée",
                    self.path.display(),
                    aside.display()
                ),
            ),
            Err(rename_error) => crate::diagnostics::record(
                "ERROR",
                format!(
                    "{} illisible ({error}) et impossible à déplacer ({rename_error}); collection vide utilisée",
                    self.path.display()
                ),
            ),
        }
    }
}

/// Strict read of a point file, for tests and tools: any damage is an error.
#[cfg(test)]
pub fn read(path: &Path) -> io::Result<Vec<(f64, f64)>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(GeoFile::<GeoPoint>::parse(&contents)?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// Writes a point file in the versioned format, atomically.
#[cfg(test)]
pub fn write(path: &Path, points: &[(f64, f64)]) -> io::Result<()> {
    let records: Vec<GeoPoint> = points.iter().copied().map(Into::into).collect();
    GeoFile::new(path).save(&records)
}

/// Writes `contents` to `path` so that a crash or power cut leaves either the
/// previous file or the complete new one, never a truncated mix. The data
/// goes to a temporary file in the same directory, is flushed to disk, and
/// is then renamed over the destination. Missing parent directories are
/// created.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let temp_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| {
        use std::io::Write;
        let mut output = std::fs::File::create(&temp_path)?;
        output.write_all(contents)?;
        output.sync_all()?;
        drop(output);
        std::fs::rename(&temp_path, path)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> std::path::PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("plotter-geo-data-{}-{id}.json", std::process::id()))
    }

    #[test]
    fn versioned_data_round_trips() {
        let path = temp_path();
        let points = [(-122.3, 47.5), (178.1, -17.7)];
        write(&path, &points).unwrap();
        assert_eq!(read(&path).unwrap(), points);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unknown_versions_and_invalid_coordinates() {
        let path = temp_path();
        std::fs::write(&path, r#"{"version":2,"points":[]}"#).unwrap();
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);

        std::fs::write(
            &path,
            r#"{"version":1,"points":[{"longitude":181.0,"latitude":0.0}]}"#,
        )
        .unwrap();
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        std::fs::remove_file(path).unwrap();
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Named {
        longitude: f64,
        latitude: f64,
        name: String,
    }

    impl GeoRecord for Named {
        fn position(&self) -> (f64, f64) {
            (self.longitude, self.latitude)
        }
    }

    fn points(path: &Path) -> GeoFile<GeoPoint> {
        GeoFile::new(path)
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_collection() {
        let path = temp_path();
        assert!(points(&path).load().is_empty());
        assert!(!path.exists(), "loading creates nothing");
    }

    #[test]
    fn named_records_round_trip_through_a_versioned_file() {
        let path = temp_path();
        let file = GeoFile::<Named>::new(&path);
        let records = vec![Named {
            longitude: -4.5,
            latitude: 48.4,
            name: "Mouillage".to_string(),
        }];
        file.save(&records).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"version\""));
        assert_eq!(file.load(), records);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_loads_empty() {
        let path = temp_path();
        std::fs::write(&path, "{ not json").unwrap();

        assert!(points(&path).load().is_empty());

        let aside = path.with_extension("json.corrupt");
        assert!(
            !path.exists(),
            "the damaged file is moved, not left in place"
        );
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), "{ not json");
        std::fs::remove_file(aside).unwrap();
    }

    #[test]
    fn an_unsupported_version_or_an_invalid_coordinate_is_set_aside() {
        for contents in [
            r#"{"version":2,"points":[]}"#,
            r#"{"version":1,"points":[{"longitude":181.0,"latitude":0.0}]}"#,
        ] {
            let path = temp_path();
            std::fs::write(&path, contents).unwrap();
            assert!(points(&path).load().is_empty(), "{contents}");
            let aside = path.with_extension("json.corrupt");
            assert!(aside.exists(), "{contents}");
            std::fs::remove_file(aside).unwrap();
        }
    }

    #[test]
    fn save_rejects_an_invalid_coordinate_and_leaves_the_file_alone() {
        let path = temp_path();
        let file = points(&path);
        file.save(&[GeoPoint::from((1.0, 2.0))]).unwrap();
        let error = file.save(&[GeoPoint::from((1.0, 95.0))]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(file.load(), vec![GeoPoint::from((1.0, 2.0))]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_older_shape_at_the_same_path_is_migrated_on_first_load() {
        let path = temp_path();
        std::fs::write(&path, r#"[{"lon":178.1,"lat":-17.7}]"#).unwrap();
        let file = GeoFile::<GeoPoint>::new(&path).with_legacy_shape(|contents| {
            #[derive(Deserialize)]
            struct Old {
                lon: f64,
                lat: f64,
            }
            serde_json::from_str::<Vec<Old>>(contents).ok().map(|old| {
                old.into_iter()
                    .map(|o| GeoPoint::from((o.lon, o.lat)))
                    .collect()
            })
        });

        assert_eq!(file.load(), vec![GeoPoint::from((178.1, -17.7))]);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"version\""));
        assert_eq!(
            file.load(),
            vec![GeoPoint::from((178.1, -17.7))],
            "second load is versioned"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migrates_legacy_data_without_changing_the_source() {
        let path = temp_path();
        let legacy_path = path.with_extension("gpx");
        let legacy_contents = "<gpx>legacy data</gpx>";
        std::fs::write(&legacy_path, legacy_contents).unwrap();

        let file =
            points(&path).with_legacy_file("gpx", |_| Ok(vec![GeoPoint::from((-122.3, 47.5))]));
        let migrated = file.load();

        assert_eq!(migrated, vec![GeoPoint::from((-122.3, 47.5))]);
        assert_eq!(read(&path).unwrap(), vec![(-122.3, 47.5)]);
        assert_eq!(
            std::fs::read_to_string(&legacy_path).unwrap(),
            legacy_contents
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(legacy_path).unwrap();
    }

    #[test]
    fn does_not_fall_back_to_legacy_data_when_json_is_invalid() {
        let path = temp_path();
        let legacy_path = path.with_extension("gpx");
        std::fs::write(&path, r#"{"version":2,"points":[]}"#).unwrap();

        std::fs::write(&legacy_path, "<gpx/>").unwrap();
        let file = points(&path).with_legacy_file("gpx", |_| Ok(vec![GeoPoint::from((0.0, 0.0))]));

        assert!(
            file.load().is_empty(),
            "set aside, not replaced by the neighbour"
        );
        assert!(path.with_extension("json.corrupt").exists());
        std::fs::remove_file(path.with_extension("json.corrupt")).unwrap();
        std::fs::remove_file(legacy_path).unwrap();
    }
}
