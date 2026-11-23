use std::fs;
use std::path::Path;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn save_route_gpx(path: &Path, name: &str, route: &[(f64, f64)]) -> std::io::Result<()> {
    let mut gpx = String::new();
    gpx.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>
");
    gpx.push_str("<gpx version=\"1.1\" creator=\"ma_carte\" xmlns=\"http://www.topografix.com/GPX/1/1\">
");
    gpx.push_str("  <rte>
");
    gpx.push_str(&format!("    <name>{}</name>
", xml_escape(name)));
    for &(lon, lat) in route {
        gpx.push_str(&format!("    <rtept lat=\"{:.6}\" lon=\"{:.6}\"/>
", lat, lon));
    }
    gpx.push_str("  </rte>
");
    gpx.push_str("</gpx>
");
    crate::geo_data::atomic_write(path, gpx.as_bytes())
}

pub fn load_gpx(path: &Path) -> std::io::Result<Vec<(f64, f64)>> {
    let text = fs::read_to_string(path)?;
    let mut route = Vec::new();

    for chunk in text.split('<').skip(1) {
        let tag = chunk.split('>').next().unwrap_or("");
        if tag.starts_with("trkpt") || tag.starts_with("rtept") {
            if let (Some(lat), Some(lon)) = (parse_attr(tag, "lat"), parse_attr(tag, "lon")) {
                route.push((lon, lat)); // retour en (lon, lat), comme votre route
            }
        }
    }
    Ok(route)
}

fn parse_attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    tag[start..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn exports_route_points_using_gpx_route_elements() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "desktop_plotter_route_export_{}_{}.gpx",
            std::process::id(),
            unique_id
        ));
        let points = [(-122.3, 47.5), (-122.2, 47.6)];

        save_route_gpx(&path, "Test route", &points).unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert!(contents.contains("<rte>"));
        assert!(contents.contains("<rtept"));
        assert!(!contents.contains("<trk>"));
        assert_eq!(load_gpx(&path).unwrap(), points);
        fs::remove_file(path).unwrap();
    }
}
