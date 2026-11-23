//! GPX 1.1 in and out, in one place. Three writers (route, track,
//! waypoints) that produce the document as a `String` for the caller to
//! save atomically, and one reader that accepts route points, track points
//! and waypoints alike and validates every coordinate. Points are `(lon,
//! lat)` as everywhere else in the code; GPX itself writes `lat` first.

use std::io;
use std::path::Path;

const CREATOR: &str = "SDL2 Plotter";
const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

fn open_tag() -> String {
    format!(
        "<gpx version=\"1.1\" creator=\"{CREATOR}\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n"
    )
}

/// Escapes the five characters XML reserves, so any name is safe in text
/// content and in attributes.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// A `<rte>` named `name` over `points`.
pub fn write_route(name: &str, points: &[(f64, f64)]) -> String {
    let mut gpx = String::from(HEADER);
    gpx.push_str(&open_tag());
    gpx.push_str("  <rte>\n");
    gpx.push_str(&format!("    <name>{}</name>\n", escape(name)));
    for &(lon, lat) in points {
        gpx.push_str(&format!("    <rtept lat=\"{lat:.6}\" lon=\"{lon:.6}\"/>\n"));
    }
    gpx.push_str("  </rte>\n</gpx>\n");
    gpx
}

/// A `<trk>` named `name` with one segment over `points`.
pub fn write_track(name: &str, points: &[(f64, f64)]) -> String {
    let mut gpx = String::from(HEADER);
    gpx.push_str(&open_tag());
    gpx.push_str("  <trk>\n");
    gpx.push_str(&format!("    <name>{}</name>\n", escape(name)));
    gpx.push_str("    <trkseg>\n");
    for &(lon, lat) in points {
        gpx.push_str(&format!(
            "      <trkpt lat=\"{lat:.6}\" lon=\"{lon:.6}\"/>\n"
        ));
    }
    gpx.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    gpx
}

/// One `<wpt>` per `(name, (lon, lat))`.
pub fn write_waypoints<'a>(waypoints: impl IntoIterator<Item = (&'a str, (f64, f64))>) -> String {
    let mut gpx = String::from(HEADER);
    gpx.push_str(&open_tag());
    for (name, (lon, lat)) in waypoints {
        gpx.push_str(&format!(
            "  <wpt lat=\"{lat:.6}\" lon=\"{lon:.6}\"><name>{}</name></wpt>\n",
            escape(name)
        ));
    }
    gpx.push_str("</gpx>\n");
    gpx
}

/// Every `<trkpt>`, `<rtept>` and `<wpt>` in `text`, in document order, as
/// `(lon, lat)`. A point with a missing, unparseable or out-of-range
/// coordinate is an `InvalidData` error: a chart position must never be
/// guessed.
pub fn read_points(text: &str) -> io::Result<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for chunk in text.split('<').skip(1) {
        let tag = chunk.split('>').next().unwrap_or("").trim();
        if !is_point_tag(tag) {
            continue;
        }
        let lat = attribute(tag, "lat").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "latitude GPX manquante ou invalide",
            )
        })?;
        let lon = attribute(tag, "lon").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "longitude GPX manquante ou invalide",
            )
        })?;
        if !crate::geo_data::valid_position((lon, lat)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("position GPX hors limites: lat={lat}, lon={lon}"),
            ));
        }
        points.push((lon, lat));
    }
    Ok(points)
}

/// `read_points` on a file. A missing file is `NotFound`, as the migration
/// in `geo_data::GeoFile` expects.
pub fn load_points(path: &Path) -> io::Result<Vec<(f64, f64)>> {
    read_points(&std::fs::read_to_string(path)?)
}

fn is_point_tag(tag: &str) -> bool {
    ["trkpt", "rtept", "wpt"].iter().any(|name| {
        tag.strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    })
}

/// The numeric value of `name="..."` or `name='...'` in a tag.
fn attribute(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=");
    let start = tag.find(&key)? + key.len();
    let rest = &tag[start..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &rest[quote.len_utf8()..];
    let end = value.find(quote)?;
    value[..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTS: [(f64, f64); 2] = [(-122.3, 47.5), (178.1, -17.7)];

    #[test]
    fn route_track_and_waypoint_documents_round_trip_through_the_reader() {
        let route = write_route("Passage", &POINTS);
        assert!(route.contains("<rte>") && route.contains("<rtept") && !route.contains("<trk>"));
        assert_eq!(read_points(&route).unwrap(), POINTS);

        let track = write_track("Track 1", &POINTS);
        assert!(track.contains("<trkseg>") && track.contains("<trkpt"));
        assert_eq!(read_points(&track).unwrap(), POINTS);

        let waypoints = write_waypoints([("A", POINTS[0]), ("B", POINTS[1])]);
        assert!(waypoints.contains("<wpt lat=\"47.500000\" lon=\"-122.300000\">"));
        assert_eq!(read_points(&waypoints).unwrap(), POINTS);
    }

    #[test]
    fn every_document_names_the_application_as_creator() {
        for document in [
            write_route("r", &[]),
            write_track("t", &[]),
            write_waypoints([]),
        ] {
            assert!(document.contains("creator=\"SDL2 Plotter\""), "{document}");
            assert!(document.starts_with("<?xml"));
        }
    }

    #[test]
    fn names_are_escaped_including_quotes() {
        let document = write_waypoints([("A & B <C> \"D\" 'E'", (0.0, 0.0))]);
        assert!(document.contains("<name>A &amp; B &lt;C&gt; &quot;D&quot; &apos;E&apos;</name>"));
    }

    #[test]
    fn single_quotes_whitespace_and_reversed_attribute_order_are_read() {
        let text = "<gpx><trk><trkseg>\n  <trkpt lon='1.5' lat='2.5' />\n</trkseg></trk></gpx>";
        assert_eq!(read_points(text).unwrap(), vec![(1.5, 2.5)]);
    }

    #[test]
    fn a_point_outside_the_valid_range_or_missing_a_coordinate_is_an_error() {
        for text in [
            "<gpx><wpt lat=\"95.0\" lon=\"0.0\"/></gpx>",
            "<gpx><rte><rtept lat=\"10.0\"/></rte></gpx>",
            "<gpx><trk><trkseg><trkpt lat=\"abc\" lon=\"0\"/></trkseg></trk></gpx>",
        ] {
            assert_eq!(
                read_points(text).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{text}"
            );
        }
    }

    #[test]
    fn a_document_without_points_reads_as_empty_and_other_tags_are_ignored() {
        let text = "<gpx><metadata><name>wptless</name></metadata><trk><trkseg/></trk></gpx>";
        assert_eq!(read_points(text).unwrap(), Vec::<(f64, f64)>::new());
    }
}
