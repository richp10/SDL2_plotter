use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct GeoDataFile {
    version: u32,
    points: Vec<GeoPoint>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GeoPoint {
    longitude: f64,
    latitude: f64,
}

pub fn read(path: &Path) -> io::Result<Vec<(f64, f64)>> {
    let contents = std::fs::read_to_string(path)?;
    let file: GeoDataFile = serde_json::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if file.version != FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported geo data version {}", file.version),
        ));
    }

    file.points
        .into_iter()
        .map(|point| {
            if !point.longitude.is_finite()
                || !(-180.0..=180.0).contains(&point.longitude)
                || !point.latitude.is_finite()
                || !(-90.0..=90.0).contains(&point.latitude)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "longitude or latitude is outside valid range",
                ));
            }
            Ok((point.longitude, point.latitude))
        })
        .collect()
}

pub fn read_or_migrate(
    path: &Path,
    legacy_path: &Path,
    load_legacy: impl FnOnce(&Path) -> io::Result<Vec<(f64, f64)>>,
) -> io::Result<Vec<(f64, f64)>> {
    match read(path) {
        Ok(points) => Ok(points),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match load_legacy(legacy_path) {
            Ok(points) => {
                write(path, &points)?;
                Ok(points)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

pub fn write(path: &Path, points: &[(f64, f64)]) -> io::Result<()> {
    if points.iter().any(|&(longitude, latitude)| {
        !longitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude)
            || !latitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "longitude or latitude is outside valid range",
        ));
    }

    let file = GeoDataFile {
        version: FORMAT_VERSION,
        points: points
            .iter()
            .map(|&(longitude, latitude)| GeoPoint {
                longitude,
                latitude,
            })
            .collect(),
    };
    let contents = serde_json::to_vec_pretty(&file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(path, &contents)
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

    #[test]
    fn migrates_legacy_data_without_changing_the_source() {
        let path = temp_path();
        let legacy_path = path.with_extension("gpx");
        let legacy_contents = "<gpx>legacy data</gpx>";
        std::fs::write(&legacy_path, legacy_contents).unwrap();

        let migrated = read_or_migrate(&path, &legacy_path, |_| Ok(vec![(-122.3, 47.5)])).unwrap();

        assert_eq!(migrated, vec![(-122.3, 47.5)]);
        assert_eq!(read(&path).unwrap(), migrated);
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

        let result = read_or_migrate(&path, &legacy_path, |_| Ok(vec![(0.0, 0.0)]));

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        std::fs::remove_file(path).unwrap();
    }
}
