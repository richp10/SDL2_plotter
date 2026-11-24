//! The chart catalogue: every chart file in the chart directory, grouped
//! into layers (files from one source at different scales), each layer a
//! `Chart` or `Imagery`. The real zoom range and coverage of each file are
//! read from its tiles and cached with its size and modification time; what
//! the user decides (display name, layer, role, hidden, order) is kept in a
//! settings file that never stops the app starting and never touches the
//! chart files. Layers and roles of new files are guessed from their names,
//! as a starting point only. See docs/chart-catalogue-design.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped only together with a migration of older settings files.
const FORMAT_VERSION: u32 = 1;

/// The layer a file joins when its name names no known source.
pub const DEFAULT_LAYER: &str = "Charts";

/// What a layer shows. Imagery is a photo, never a navigational chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Chart,
    Imagery,
}

/// Coverage in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl Bounds {
    /// Whether `(lon, lat)` lies inside, including coverage that crosses the
    /// antimeridian (`west > east`).
    pub fn contains(&self, (lon, lat): (f64, f64)) -> bool {
        let in_longitude = if self.west <= self.east {
            (self.west..=self.east).contains(&lon)
        } else {
            lon >= self.west || lon <= self.east
        };
        in_longitude && (self.south..=self.north).contains(&lat)
    }

    /// Size in square degrees: only for comparing coverage.
    pub fn area(&self) -> f64 {
        let width = (self.east - self.west).rem_euclid(360.0);
        width * (self.north - self.south)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub role: Role,
}

/// One chart file as the catalogue knows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartFile {
    /// The file name in the chart directory; the key in the settings file.
    pub file_name: String,
    pub path: PathBuf,
    /// Display name, editable by the user.
    pub name: String,
    pub layer: String,
    /// Zoom levels the file actually contains, ascending.
    pub zooms: Vec<i32>,
    pub bounds: Bounds,
    pub hidden: bool,
    pub order: u32,
    /// False until the user has looked at a new file in the manager.
    pub reviewed: bool,
}

pub struct Catalogue {
    files: Vec<ChartFile>,
    layers: Vec<Layer>,
    /// Settings of files not present now (an unplugged drive), kept so they
    /// come back when the file does.
    missing: BTreeMap<String, FileSettings>,
    settings: PathBuf,
    /// The scan of each present file, saved so the next start can reuse it.
    scans: BTreeMap<String, Scan>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Settings {
    version: u32,
    layers: Vec<Layer>,
    files: BTreeMap<String, FileSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileSettings {
    name: String,
    layer: String,
    hidden: bool,
    order: u32,
    reviewed: bool,
    scan: Scan,
}

/// What was read from the file, valid while its size and mtime match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Scan {
    size: u64,
    modified: u64,
    zooms: Vec<i32>,
    bounds: Bounds,
}

impl Catalogue {
    /// Scans `dir` for `*.mbtiles`, merges the settings at `settings`, and
    /// saves them when anything new was learnt. Never fails: an unreadable
    /// chart is skipped with a WARN, a damaged settings file is set aside.
    pub fn load(dir: &Path, settings: &Path) -> Catalogue {
        let saved = read_settings(settings);
        let mut changed = false;
        let mut layers = saved.layers;
        let mut remaining = saved.files;
        let mut next_order = remaining
            .values()
            .map(|file| file.order + 1)
            .max()
            .unwrap_or(0);
        let mut files = Vec::new();
        let mut scans = BTreeMap::new();

        for path in chart_paths(dir) {
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let file_name = file_name.to_string();
            let previous = remaining.remove(&file_name);
            let stamp = file_stamp(&path).unwrap_or_default();

            let cached = previous
                .as_ref()
                .filter(|p| stamp == (p.scan.size, p.scan.modified))
                .map(|p| (p.scan.clone(), None));
            let (scan, metadata_name) = match cached {
                Some(cached) => cached,
                None => match scan(&path, stamp) {
                    Ok(fresh) => {
                        changed = true;
                        fresh
                    }
                    Err(error) => {
                        crate::diagnostics::record(
                            "WARN",
                            format!("Carte ignorée {}: {error}", path.display()),
                        );
                        if let Some(previous) = previous {
                            remaining.insert(file_name, previous);
                        }
                        continue;
                    }
                },
            };

            let entry = match previous {
                Some(previous) => FileSettings { scan, ..previous },
                None => {
                    let guess = guess(&file_name, metadata_name.as_deref());
                    if !layers.iter().any(|l| l.name == guess.layer) {
                        layers.push(Layer {
                            name: guess.layer.clone(),
                            role: guess.role,
                        });
                    }
                    next_order += 1;
                    FileSettings {
                        name: display_name(&file_name, metadata_name.as_deref()),
                        layer: guess.layer,
                        hidden: false,
                        order: next_order - 1,
                        reviewed: false,
                        scan,
                    }
                }
            };
            // A hand-edited file may name a layer that is not declared.
            if !layers.iter().any(|l| l.name == entry.layer) {
                layers.push(Layer {
                    name: entry.layer.clone(),
                    role: Role::Chart,
                });
                changed = true;
            }
            scans.insert(file_name.clone(), entry.scan.clone());
            files.push(ChartFile {
                file_name,
                path,
                name: entry.name,
                layer: entry.layer,
                zooms: entry.scan.zooms,
                bounds: entry.scan.bounds,
                hidden: entry.hidden,
                order: entry.order,
                reviewed: entry.reviewed,
            });
        }
        files.sort_by(|a, b| (a.order, &a.file_name).cmp(&(b.order, &b.file_name)));

        let catalogue = Catalogue {
            files,
            layers,
            missing: remaining,
            settings: settings.to_path_buf(),
            scans,
        };
        if changed || !settings.exists() {
            if let Err(error) = catalogue.save() {
                crate::diagnostics::record(
                    "ERROR",
                    format!(
                        "Erreur sauvegarde du catalogue {}: {error}",
                        settings.display()
                    ),
                );
            }
        }
        catalogue
    }

    /// Every file present, in the user's order.
    #[cfg(test)]
    pub fn files(&self) -> &[ChartFile] {
        &self.files
    }

    #[cfg(test)]
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The files not hidden, in order.
    pub fn visible(&self) -> impl Iterator<Item = &ChartFile> {
        self.files.iter().filter(|file| !file.hidden)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let mut files = self.missing.clone();
        for file in &self.files {
            let Some(scan) = self.scans.get(&file.file_name) else {
                continue;
            };
            files.insert(
                file.file_name.clone(),
                FileSettings {
                    name: file.name.clone(),
                    layer: file.layer.clone(),
                    hidden: file.hidden,
                    order: file.order,
                    reviewed: file.reviewed,
                    scan: scan.clone(),
                },
            );
        }
        let settings = Settings {
            version: FORMAT_VERSION,
            layers: self.layers.clone(),
            files,
        };
        let json = serde_json::to_vec_pretty(&settings)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        crate::geo_data::atomic_write(&self.settings, &json)
    }

    /// One line per file for the startup log: what the catalogue decided.
    pub fn describe(&self) -> Vec<String> {
        self.files
            .iter()
            .map(|file| {
                let role = match self.role_of(&file.layer) {
                    Role::Chart => "carte",
                    Role::Imagery => "imagerie",
                };
                let zooms = match (file.zooms.first(), file.zooms.last()) {
                    (Some(min), Some(max)) => format!("z{min}-{max}"),
                    _ => "sans zoom".to_string(),
                };
                let b = file.bounds;
                format!(
                    "Carte {}: \"{}\", couche {} ({role}), {zooms}, emprise {:.2},{:.2} à {:.2},{:.2}{}{}",
                    file.file_name,
                    file.name,
                    file.layer,
                    b.west,
                    b.south,
                    b.east,
                    b.north,
                    if file.hidden { ", masquée" } else { "" },
                    if file.reviewed {
                        ""
                    } else {
                        ", nouvelle: rôle à vérifier"
                    },
                )
            })
            .collect()
    }

    /// The layers with at least one visible file, in catalogue order: the
    /// layer switcher. Entry `i` of the Charts menu is entry `i` here.
    pub fn switchable_layers(&self) -> Vec<&Layer> {
        self.layers
            .iter()
            .filter(|layer| self.visible().any(|f| f.layer == layer.name))
            .collect()
    }

    /// The role of `layer`; an unknown layer is a chart.
    pub fn role_of(&self, layer: &str) -> Role {
        self.layers
            .iter()
            .find(|l| l.name == layer)
            .map_or(Role::Chart, |l| l.role)
    }

    /// Which file to show for `request`, and what the navigator must be told
    /// about it. See the design's decisions 5, 6 and 8.
    pub fn choose<'a>(&'a self, request: Request<'a>) -> Choice<'a> {
        let Request {
            layer,
            lock,
            centre,
            zoom,
        } = request;
        let locked = lock.and_then(|lock| self.files.iter().find(|f| f.file_name == lock));

        let mut fallback_from = None;
        let file = match locked {
            Some(locked) => Some(locked),
            None => layer
                .and_then(|layer| self.best_in(layer, centre, zoom))
                .or_else(|| {
                    // Nothing in the chosen layer here: the best chart instead,
                    // never imagery, and say so.
                    fallback_from = layer;
                    self.layers
                        .iter()
                        .filter(|l| l.role == Role::Chart && Some(l.name.as_str()) != layer)
                        .filter_map(|l| self.best_in(&l.name, centre, zoom))
                        .min_by(|a, b| detail_order(a, b))
                }),
        };
        if file.is_some() && layer.is_none() {
            fallback_from = None;
        }

        let shown_layer = file.map(|f| f.layer.as_str());
        let covering: Vec<&ChartFile> = shown_layer
            .map(|layer| self.covering(layer, centre))
            .unwrap_or_default();
        let top = |f: &ChartFile| f.zooms.last().copied().unwrap_or(zoom);
        Choice {
            file,
            overscale: file.map(|f| zoom - top(f)).filter(|&levels| levels >= 1),
            more_detail: file.and_then(|shown| {
                covering
                    .iter()
                    .copied()
                    .filter(|f| f.file_name != shown.file_name && top(f) > top(shown))
                    .min_by(|a, b| detail_order(a, b))
            }),
            fallback_from,
            outside_lock: locked.is_some_and(|f| !f.bounds.contains(centre)),
            // How deep the shown layer goes here, and the chosen layer too:
            // its files may start deeper than a fallback that shows now, and
            // zooming in must reach them. With nothing shown, every chart.
            zoom_limit: self
                .layers
                .iter()
                .filter(|l| {
                    Some(l.name.as_str()) == shown_layer
                        || Some(l.name.as_str()) == layer
                        || (file.is_none() && l.role == Role::Chart)
                })
                .flat_map(|l| self.covering(&l.name, centre))
                .filter_map(|f| f.zooms.last().copied())
                .chain(file.map(top))
                .max(),
            beneath: file.and_then(|shown| {
                covering
                    .iter()
                    .copied()
                    .filter(|f| {
                        f.file_name != shown.file_name
                            && f.zooms.first().is_some_and(|&min| min <= zoom)
                    })
                    .min_by(|a, b| detail_order(a, b))
            }),
        }
    }

    /// The visible files of `layer` whose coverage contains `centre`.
    fn covering(&self, layer: &str, centre: (f64, f64)) -> Vec<&ChartFile> {
        self.visible()
            .filter(|f| f.layer == layer && f.bounds.contains(centre))
            .collect()
    }

    /// The file of `layer` to show at `centre` and `zoom`: the most detailed
    /// one whose zoom levels include `zoom`, else, past the top of all of
    /// them, the one going deepest (shown overscaled). A file whose first
    /// level is deeper than `zoom` cannot draw yet and is passed over.
    fn best_in(&self, layer: &str, centre: (f64, f64), zoom: i32) -> Option<&ChartFile> {
        let in_range = |f: &&ChartFile| matches!((f.zooms.first(), f.zooms.last()), (Some(&min), Some(&max)) if min <= zoom && zoom <= max);
        let candidates = self.covering(layer, centre);
        candidates
            .iter()
            .copied()
            .filter(in_range)
            .min_by(|a, b| detail_order(a, b))
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .filter(|f| f.zooms.last().is_some_and(|&max| max < zoom))
                    .min_by(|a, b| detail_order(a, b))
            })
    }
}

/// Most detailed first: the deepest top zoom, then the smaller coverage,
/// then the navigator's order.
fn detail_order(a: &ChartFile, b: &ChartFile) -> std::cmp::Ordering {
    let top = |f: &ChartFile| f.zooms.last().copied().unwrap_or(i32::MIN);
    top(b)
        .cmp(&top(a))
        .then(a.bounds.area().total_cmp(&b.bounds.area()))
        .then(a.order.cmp(&b.order))
}

/// What `Catalogue::choose` decides from.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The layer the navigator chose; `None` before any choice.
    pub layer: Option<&'a str>,
    /// The file name the navigator locked, if any.
    pub lock: Option<&'a str>,
    /// The position at the centre of the screen.
    pub centre: (f64, f64),
    /// The whole zoom level being shown or animated to.
    pub zoom: i32,
}

/// The file to show and the indications that go with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Choice<'a> {
    /// `None`: nothing in the catalogue fits; show the default chart.
    pub file: Option<&'a ChartFile>,
    /// Zoom levels past the file's most detailed level, when at least one.
    pub overscale: Option<i32>,
    /// A more detailed file of the shown layer that covers the centre but
    /// that the zoom or the lock keeps off screen.
    pub more_detail: Option<&'a ChartFile>,
    /// The requested layer, when it has nothing here and a chart layer is
    /// shown instead. Never set the other way: imagery is no fallback.
    pub fallback_from: Option<&'a str>,
    /// The locked file does not cover the centre.
    pub outside_lock: bool,
    /// The deepest zoom worth zooming in to here, if any file fits.
    pub zoom_limit: Option<i32>,
    /// The next best file of the same layer, drawn beneath `file` so that
    /// beyond its edge (or outside a locked file) the screen is never blank.
    pub beneath: Option<&'a ChartFile>,
}

/// Known sources by a word in the file or chart name: the layer a file joins
/// and that layer's role. Checked in order; the first match wins.
const SOURCES: [(&str, &str, Role); 7] = [
    ("navionics", "Navionics", Role::Chart),
    ("openseamap", "OpenSeaMap", Role::Chart),
    ("noaa", "NOAA", Role::Chart),
    ("bing", "Bing satellite", Role::Imagery),
    ("google", "Google satellite", Role::Imagery),
    ("arcgis", "ArcGIS satellite", Role::Imagery),
    ("esri", "Esri satellite", Role::Imagery),
];

/// Words that mark imagery from an unknown source.
const IMAGERY_WORDS: [&str; 5] = ["sat", "imagery", "aerial", "ortho", "photo"];

/// The layer an imagery file of unknown source joins.
const IMAGERY_LAYER: &str = "Imagery";

/// Below this length a metadata name ("test", "map") says less than the
/// file name, so the file name is shown instead.
const MEANINGFUL_NAME: usize = 6;

struct Guess {
    layer: String,
    role: Role,
}

/// A starting layer and role for a new file, from its names. Imagery only
/// when a name clearly says so; everything else is a chart in the default
/// layer, so a new file works at once and is flagged for review.
fn guess(file_name: &str, metadata_name: Option<&str>) -> Guess {
    let text = format!("{file_name} {}", metadata_name.unwrap_or_default()).to_lowercase();
    if let Some(&(_, layer, role)) = SOURCES.iter().find(|(word, ..)| text.contains(word)) {
        return Guess {
            layer: layer.to_string(),
            role,
        };
    }
    if IMAGERY_WORDS.iter().any(|word| text.contains(word)) {
        return Guess {
            layer: IMAGERY_LAYER.to_string(),
            role: Role::Imagery,
        };
    }
    Guess {
        layer: DEFAULT_LAYER.to_string(),
        role: Role::Chart,
    }
}

fn display_name(file_name: &str, metadata_name: Option<&str>) -> String {
    match metadata_name.map(str::trim) {
        Some(name) if name.chars().count() >= MEANINGFUL_NAME => name.to_string(),
        _ => Path::new(file_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(file_name)
            .to_string(),
    }
}

/// The `*.mbtiles` files in `dir`, sorted by name. A missing directory is
/// an empty one.
fn chart_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mbtiles"))
        })
        .collect();
    paths.sort();
    paths
}

/// Size and modification time in nanoseconds: the cache key of a scan.
fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((metadata.len(), modified))
}

/// Reads the zoom levels and coverage of a chart, and its metadata name.
fn scan(path: &Path, (size, modified): (u64, u64)) -> Result<(Scan, Option<String>), String> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| e.to_string())?;
    let metadata = |key: &str| -> Option<String> {
        conn.query_row("SELECT value FROM metadata WHERE name = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    };

    let zooms: Vec<i32> = conn
        .prepare("SELECT DISTINCT zoom_level FROM tiles ORDER BY zoom_level")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, i32>(0))?
                .collect::<rusqlite::Result<Vec<i32>>>()
        })
        .map_err(|e| e.to_string())?;
    let Some(&min_zoom) = zooms.first() else {
        return Err("aucune tuile".to_string());
    };

    let bounds = match metadata("bounds").as_deref().and_then(parse_bounds) {
        Some(bounds) => bounds,
        None => tile_extent(&conn, min_zoom).map_err(|e| e.to_string())?,
    };
    let name = metadata("name");
    Ok((
        Scan {
            size,
            modified,
            zooms,
            bounds,
        },
        name,
    ))
}

/// MBTiles `bounds`: "west,south,east,north", validated.
fn parse_bounds(text: &str) -> Option<Bounds> {
    let values: Vec<f64> = text
        .split(',')
        .map(|v| v.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let [west, south, east, north] = values[..] else {
        return None;
    };
    let valid = crate::geo_data::valid_position((west, south))
        && crate::geo_data::valid_position((east, north))
        && south < north;
    valid.then_some(Bounds {
        west,
        south,
        east,
        north,
    })
}

/// The area the tiles at `zoom` cover (rows are TMS, counted from the south).
fn tile_extent(conn: &rusqlite::Connection, zoom: i32) -> rusqlite::Result<Bounds> {
    let (min_x, max_x, min_row, max_row): (i64, i64, i64, i64) = conn.query_row(
        "SELECT MIN(tile_column), MAX(tile_column), MIN(tile_row), MAX(tile_row)
         FROM tiles WHERE zoom_level = ?1",
        [zoom],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let n = (1i64 << zoom) as f64;
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| {
        (std::f64::consts::PI * (1.0 - 2.0 * y / n))
            .sinh()
            .atan()
            .to_degrees()
    };
    // TMS row r is XYZ row n - 1 - r, whose top edge is at y = n - 1 - r.
    Ok(Bounds {
        west: lon(min_x as f64),
        east: lon(max_x as f64 + 1.0),
        north: lat(n - 1.0 - max_row as f64),
        south: lat(n - min_row as f64),
    })
}

/// The saved settings, or defaults. A damaged file is set aside with an
/// ERROR so it can be inspected, and never stops the app.
fn read_settings(path: &Path) -> Settings {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Settings::default();
        }
        Err(error) => {
            crate::diagnostics::record(
                "ERROR",
                format!(
                    "Lecture du catalogue {} impossible: {error}",
                    path.display()
                ),
            );
            return Settings::default();
        }
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(settings) if settings.version == FORMAT_VERSION => settings,
        Ok(settings) => set_aside(path, &format!("version {} inconnue", settings.version)),
        Err(error) => set_aside(path, &error.to_string()),
    }
}

fn set_aside(path: &Path, reason: &str) -> Settings {
    let moved = crate::geo_data::set_aside(path)
        .map(|aside| format!("mis de côté: {}", aside.display()))
        .unwrap_or_else(|error| format!("impossible de le mettre de côté: {error}"));
    crate::diagnostics::record(
        "ERROR",
        format!("Catalogue {} illisible ({reason}), {moved}", path.display()),
    );
    Settings::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "plotter-catalogue-{label}-{}",
            crate::mbtiles_renderer::test_support::unique_name()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An MBTiles file named `name` in `dir` with one tile (TMS rows) per
    /// `(zoom, column, row)` and the given metadata rows. The tiles hold no
    /// image: the catalogue never decodes them.
    fn chart(dir: &Path, name: &str, tiles: &[(i32, i64, i64)], metadata: &[(&str, &str)]) {
        let conn = Connection::open(dir.join(name)).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (name TEXT, value TEXT);
             CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);",
        )
        .unwrap();
        for &(z, x, y) in tiles {
            conn.execute("INSERT INTO tiles VALUES (?1, ?2, ?3, x'00')", (z, x, y))
                .unwrap();
        }
        for (key, value) in metadata {
            conn.execute("INSERT INTO metadata VALUES (?1, ?2)", (key, value))
                .unwrap();
        }
    }

    fn whole_world() -> Vec<(&'static str, &'static str)> {
        vec![("bounds", "-180,-85,180,85")]
    }

    fn file<'a>(catalogue: &'a Catalogue, file_name: &str) -> &'a ChartFile {
        catalogue
            .files()
            .iter()
            .find(|f| f.file_name == file_name)
            .unwrap_or_else(|| panic!("{file_name} not in catalogue"))
    }

    fn role(catalogue: &Catalogue, layer: &str) -> Role {
        catalogue
            .layers()
            .iter()
            .find(|l| l.name == layer)
            .unwrap_or_else(|| panic!("no layer {layer}"))
            .role
    }

    #[test]
    fn new_files_are_grouped_by_source_and_imagery_is_recognised_by_name() {
        let dir = temp_dir("guess");
        let tiles = [(3, 0, 0)];
        for name in [
            "Azores_BingSat.mbtiles",
            "Azores_Navionics.mbtiles",
            "Atlantic_Navionics.mbtiles",
            "harbour_ortho.mbtiles",
            "harbour-scan-2019.mbtiles",
        ] {
            chart(&dir, name, &tiles, &whole_world());
        }
        std::fs::write(dir.join("notes.txt"), "not a chart").unwrap();

        let catalogue = Catalogue::load(&dir, &dir.join("settings/charts.json"));

        assert_eq!(catalogue.files().len(), 5, "only .mbtiles files");
        assert_eq!(
            file(&catalogue, "Azores_Navionics.mbtiles").layer,
            "Navionics"
        );
        assert_eq!(
            file(&catalogue, "Atlantic_Navionics.mbtiles").layer,
            "Navionics"
        );
        assert_eq!(role(&catalogue, "Navionics"), Role::Chart);
        assert_eq!(
            file(&catalogue, "Azores_BingSat.mbtiles").layer,
            "Bing satellite"
        );
        assert_eq!(role(&catalogue, "Bing satellite"), Role::Imagery);
        assert_eq!(file(&catalogue, "harbour_ortho.mbtiles").layer, "Imagery");
        assert_eq!(role(&catalogue, "Imagery"), Role::Imagery);
        let bland = file(&catalogue, "harbour-scan-2019.mbtiles");
        assert_eq!(bland.layer, DEFAULT_LAYER);
        assert_eq!(role(&catalogue, DEFAULT_LAYER), Role::Chart);
        assert!(catalogue.files().iter().all(|f| !f.reviewed), "all new");
        assert!(
            dir.join("settings/charts.json").exists(),
            "what was learnt is saved"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_zoom_range_comes_from_the_tiles_not_the_metadata() {
        let dir = temp_dir("zooms");
        chart(
            &dir,
            "a.mbtiles",
            &[(7, 0, 0), (9, 0, 0), (11, 0, 0)],
            &[
                ("minzoom", "8"),
                ("maxzoom", "12"),
                ("bounds", "-10,30,0,40"),
            ],
        );
        let catalogue = Catalogue::load(&dir, &dir.join("charts.json"));
        assert_eq!(file(&catalogue, "a.mbtiles").zooms, vec![7, 9, 11]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn coverage_comes_from_bounds_or_else_from_the_tiles() {
        let dir = temp_dir("bounds");
        chart(
            &dir,
            "declared.mbtiles",
            &[(2, 0, 0)],
            &[("bounds", "-33.75, 36.6, -22.5, 41")],
        );
        // Zoom 1 has 2x2 tiles; TMS row 1 is the northern half, column 1 the
        // eastern half: the tile covering 0..180 E, 0..85 N.
        chart(&dir, "silent.mbtiles", &[(1, 1, 1), (3, 0, 0)], &[]);
        chart(
            &dir,
            "nonsense.mbtiles",
            &[(1, 0, 0)],
            &[("bounds", "west,south")],
        );

        let catalogue = Catalogue::load(&dir, &dir.join("charts.json"));

        let declared = file(&catalogue, "declared.mbtiles").bounds;
        assert_eq!(
            (declared.west, declared.south, declared.east, declared.north),
            (-33.75, 36.6, -22.5, 41.0)
        );
        let silent = file(&catalogue, "silent.mbtiles").bounds;
        assert!((silent.west - 0.0).abs() < 1e-9 && (silent.east - 180.0).abs() < 1e-9);
        assert!(
            silent.south.abs() < 1e-9 && (silent.north - 85.051).abs() < 0.01,
            "{silent:?}"
        );
        let nonsense = file(&catalogue, "nonsense.mbtiles").bounds;
        assert!(
            (nonsense.west + 180.0).abs() < 1e-9 && nonsense.north.abs() < 1e-9,
            "{nonsense:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_users_choices_survive_a_restart() {
        let dir = temp_dir("choices");
        chart(&dir, "Azores_BingSat.mbtiles", &[(3, 0, 0)], &whole_world());
        chart(&dir, "b.mbtiles", &[(3, 0, 0)], &whole_world());
        let settings = dir.join("charts.json");
        drop(Catalogue::load(&dir, &settings));

        // The user renames, regroups, hides, reorders and reviews in the
        // manager; here the saved file is edited as the manager would.
        let mut saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let bing = &mut saved["files"]["Azores_BingSat.mbtiles"];
        bing["name"] = "Azores photo".into();
        bing["hidden"] = true.into();
        bing["reviewed"] = true.into();
        bing["order"] = 5.into();
        saved["files"]["b.mbtiles"]["layer"] = "Harbours".into();
        saved["layers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "Harbours", "role": "chart"}));
        std::fs::write(&settings, saved.to_string()).unwrap();

        let catalogue = Catalogue::load(&dir, &settings);
        let bing = file(&catalogue, "Azores_BingSat.mbtiles");
        assert_eq!(bing.name, "Azores photo");
        assert!(bing.hidden && bing.reviewed);
        assert_eq!(file(&catalogue, "b.mbtiles").layer, "Harbours");
        assert_eq!(
            catalogue
                .files()
                .iter()
                .map(|f| f.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["b.mbtiles", "Azores_BingSat.mbtiles"],
            "user order"
        );
        assert_eq!(
            catalogue
                .visible()
                .map(|f| f.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["b.mbtiles"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_cached_scan_is_reused_while_the_file_is_unchanged() {
        let dir = temp_dir("cache");
        chart(&dir, "a.mbtiles", &[(3, 0, 0)], &whole_world());
        let settings = dir.join("charts.json");
        drop(Catalogue::load(&dir, &settings));

        let mut saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        saved["files"]["a.mbtiles"]["scan"]["zooms"] = serde_json::json!([4, 5]);
        std::fs::write(&settings, saved.to_string()).unwrap();
        assert_eq!(
            file(&Catalogue::load(&dir, &settings), "a.mbtiles").zooms,
            vec![4, 5],
            "unchanged file: the cache is trusted"
        );

        // The file changes (a zoom level is added): it is scanned again.
        let conn = Connection::open(dir.join("a.mbtiles")).unwrap();
        for x in 0..64 {
            conn.execute("INSERT INTO tiles VALUES (6, ?1, 0, zeroblob(4096))", [x])
                .unwrap();
        }
        drop(conn);
        assert_eq!(
            file(&Catalogue::load(&dir, &settings), "a.mbtiles").zooms,
            vec![3, 6]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_settings_file_is_set_aside_and_the_charts_still_load() {
        let dir = temp_dir("corrupt");
        chart(&dir, "a.mbtiles", &[(3, 0, 0)], &whole_world());
        let settings = dir.join("charts.json");
        std::fs::write(&settings, "{ not json").unwrap();

        let catalogue = Catalogue::load(&dir, &settings);

        assert_eq!(catalogue.files().len(), 1);
        assert!(dir.join("charts.json.corrupt").exists());
        assert!(settings.exists(), "fresh settings written");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_of_a_missing_file_come_back_with_the_file() {
        let dir = temp_dir("missing");
        chart(&dir, "a.mbtiles", &[(3, 0, 0)], &whole_world());
        let settings = dir.join("charts.json");
        drop(Catalogue::load(&dir, &settings));
        let mut saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        saved["files"]["a.mbtiles"]["name"] = "Kept".into();
        std::fs::write(&settings, saved.to_string()).unwrap();

        let away = dir.join("a.away");
        std::fs::rename(dir.join("a.mbtiles"), &away).unwrap();
        let without = Catalogue::load(&dir, &settings);
        assert!(without.files().is_empty());
        without.save().unwrap();

        std::fs::rename(&away, dir.join("a.mbtiles")).unwrap();
        assert_eq!(
            file(&Catalogue::load(&dir, &settings), "a.mbtiles").name,
            "Kept"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_chart_is_skipped() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join("broken.mbtiles"), "not sqlite at all").unwrap();
        chart(&dir, "empty.mbtiles", &[], &whole_world());
        chart(&dir, "good.mbtiles", &[(3, 0, 0)], &whole_world());
        let catalogue = Catalogue::load(&dir, &dir.join("charts.json"));
        assert_eq!(
            catalogue
                .files()
                .iter()
                .map(|f| f.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["good.mbtiles"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_chart_directory_gives_an_empty_catalogue() {
        let dir = temp_dir("nodir");
        let catalogue = Catalogue::load(&dir.join("absent"), &dir.join("charts.json"));
        assert!(catalogue.files().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    // ----- choosing the file to show -----

    /// An ocean chart and two more detailed ones, from one chart source,
    /// plus satellite imagery of the islands only:
    /// - Atlantic_Navionics: z7-11, 98 W..3 W, 5 S..52 N
    /// - Azores_Navionics: z7-17, 34 W..22 W, 36 N..41 N
    /// - GB_Navionics: z10-15, 9 W..3 E, 49 N..60 N
    /// - Azores_BingSat: z7-17, the Azores
    struct Charts {
        dir: PathBuf,
        catalogue: Catalogue,
    }

    impl Drop for Charts {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn charts() -> Charts {
        let dir = temp_dir("choose");
        let zooms = |range: std::ops::RangeInclusive<i32>| -> Vec<(i32, i64, i64)> {
            range.map(|z| (z, 0, 0)).collect()
        };
        chart(
            &dir,
            "Atlantic_Navionics.mbtiles",
            &zooms(7..=11),
            &[("bounds", "-98,-5,-3,52")],
        );
        chart(
            &dir,
            "Azores_Navionics.mbtiles",
            &zooms(7..=17),
            &[("bounds", "-34,36,-22,41")],
        );
        chart(
            &dir,
            "GB_Navionics.mbtiles",
            &zooms(10..=15),
            &[("bounds", "-9,49,3,60")],
        );
        chart(
            &dir,
            "Azores_BingSat.mbtiles",
            &zooms(7..=17),
            &[("bounds", "-34,36,-22,41")],
        );
        let catalogue = Catalogue::load(&dir, &dir.join("charts.json"));
        Charts { dir, catalogue }
    }

    const AZORES: (f64, f64) = (-28.0, 38.5);
    const MID_ATLANTIC: (f64, f64) = (-40.0, 30.0);
    const CORNWALL: (f64, f64) = (-5.0, 50.2);

    fn navionics(centre: (f64, f64), zoom: i32) -> Request<'static> {
        Request {
            layer: Some("Navionics"),
            lock: None,
            centre,
            zoom,
        }
    }

    fn shown<'a>(choice: &Choice<'a>) -> &'a str {
        choice
            .file
            .map_or("(default chart)", |f| f.file_name.as_str())
    }

    #[test]
    fn the_most_detailed_file_covering_the_centre_is_shown() {
        let c = charts();
        let choice = c.catalogue.choose(navionics(AZORES, 9));
        assert_eq!(shown(&choice), "Azores_Navionics.mbtiles");
        assert_eq!(choice.overscale, None);
        let choice = c.catalogue.choose(navionics(MID_ATLANTIC, 9));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(choice.fallback_from, None);
    }

    #[test]
    fn past_the_top_zoom_the_file_stays_on_overscaled() {
        let c = charts();
        let choice = c.catalogue.choose(navionics(MID_ATLANTIC, 13));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(choice.overscale, Some(2));
        assert_eq!(choice.zoom_limit, Some(11), "nothing deeper here");
    }

    #[test]
    fn a_more_detailed_file_the_zoom_hides_is_announced() {
        let c = charts();
        // Off Cornwall at zoom 8 the GB chart (z10-15) cannot draw yet.
        let choice = c.catalogue.choose(navionics(CORNWALL, 8));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(
            choice.more_detail.map(|f| f.file_name.as_str()),
            Some("GB_Navionics.mbtiles")
        );
        assert_eq!(choice.zoom_limit, Some(15), "zooming in reaches it");
        let choice = c.catalogue.choose(navionics(CORNWALL, 10));
        assert_eq!(shown(&choice), "GB_Navionics.mbtiles");
        assert_eq!(choice.more_detail, None);
    }

    #[test]
    fn imagery_shows_only_where_it_exists_and_a_chart_is_the_fallback() {
        let c = charts();
        let bing = |centre| Request {
            layer: Some("Bing satellite"),
            lock: None,
            centre,
            zoom: 9,
        };
        let choice = c.catalogue.choose(bing(AZORES));
        assert_eq!(shown(&choice), "Azores_BingSat.mbtiles");
        assert_eq!(choice.fallback_from, None);

        let choice = c.catalogue.choose(bing(MID_ATLANTIC));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(choice.fallback_from, Some("Bing satellite"));
    }

    #[test]
    fn with_no_layer_chosen_the_best_chart_is_shown_never_imagery() {
        let c = charts();
        let choice = c.catalogue.choose(Request {
            layer: None,
            ..navionics(AZORES, 12)
        });
        assert_eq!(shown(&choice), "Azores_Navionics.mbtiles");
        assert_eq!(choice.fallback_from, None);
    }

    #[test]
    fn below_every_files_zoom_or_outside_all_coverage_the_default_chart_is_left_to_show() {
        let c = charts();
        let choice = c.catalogue.choose(navionics(AZORES, 5));
        assert_eq!(shown(&choice), "(default chart)");
        assert_eq!(choice.zoom_limit, Some(17), "zooming in reaches the charts");
        let choice = c.catalogue.choose(navionics((150.0, -30.0), 9));
        assert_eq!(shown(&choice), "(default chart)");
        assert_eq!(choice.fallback_from, Some("Navionics"));
        assert_eq!(choice.zoom_limit, None);
    }

    #[test]
    fn a_hidden_file_is_never_chosen() {
        let mut c = charts();
        c.catalogue
            .files
            .iter_mut()
            .find(|f| f.file_name == "Azores_Navionics.mbtiles")
            .unwrap()
            .hidden = true;
        let choice = c.catalogue.choose(navionics(AZORES, 9));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(choice.more_detail, None);
    }

    #[test]
    fn a_locked_file_stays_shown_and_says_what_it_hides_or_misses() {
        let c = charts();
        let locked = |centre, zoom| Request {
            lock: Some("Atlantic_Navionics.mbtiles"),
            ..navionics(centre, zoom)
        };
        let choice = c.catalogue.choose(locked(AZORES, 12));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert_eq!(choice.overscale, Some(1));
        assert_eq!(
            choice.more_detail.map(|f| f.file_name.as_str()),
            Some("Azores_Navionics.mbtiles")
        );
        assert!(!choice.outside_lock);

        let choice = c.catalogue.choose(locked((0.0, 57.0), 12));
        assert_eq!(shown(&choice), "Atlantic_Navionics.mbtiles");
        assert!(
            choice.outside_lock,
            "the North Sea is beyond the Atlantic chart"
        );

        let choice = c.catalogue.choose(Request {
            lock: Some("gone.mbtiles"),
            ..navionics(AZORES, 9)
        });
        assert_eq!(
            shown(&choice),
            "Azores_Navionics.mbtiles",
            "a stale lock is ignored"
        );
    }

    #[test]
    fn the_next_best_file_of_the_layer_is_drawn_beneath() {
        let c = charts();
        let beneath = |request| {
            c.catalogue
                .choose(request)
                .beneath
                .map(|f| f.file_name.clone())
        };
        assert_eq!(
            beneath(navionics(AZORES, 9)).as_deref(),
            Some("Atlantic_Navionics.mbtiles")
        );
        assert_eq!(
            beneath(navionics(AZORES, 14)).as_deref(),
            Some("Atlantic_Navionics.mbtiles"),
            "overscaled beneath is better than blank"
        );
        assert_eq!(
            beneath(navionics(MID_ATLANTIC, 9)),
            None,
            "nothing else here"
        );
        let bing = Request {
            layer: Some("Bing satellite"),
            ..navionics(AZORES, 9)
        };
        assert_eq!(beneath(bing), None, "imagery never mixes with a chart");
        let locked_outside = Request {
            lock: Some("Atlantic_Navionics.mbtiles"),
            ..navionics((0.0, 57.0), 12)
        };
        assert_eq!(
            beneath(locked_outside).as_deref(),
            Some("GB_Navionics.mbtiles"),
            "outside a locked file the best file there fills in"
        );
    }

    #[test]
    fn the_layer_switcher_lists_layers_with_something_to_show() {
        let mut c = charts();
        let names = |c: &Charts| -> Vec<String> {
            c.catalogue
                .switchable_layers()
                .iter()
                .map(|l| l.name.clone())
                .collect()
        };
        assert_eq!(names(&c), vec!["Navionics", "Bing satellite"]);
        c.catalogue
            .files
            .iter_mut()
            .filter(|f| f.layer == "Bing satellite")
            .for_each(|f| f.hidden = true);
        assert_eq!(names(&c), vec!["Navionics"], "all its files hidden");
    }

    #[test]
    fn coverage_across_the_antimeridian_is_understood() {
        let pacific = Bounds {
            west: 170.0,
            south: -20.0,
            east: -170.0,
            north: -10.0,
        };
        assert!(pacific.contains((179.0, -15.0)));
        assert!(pacific.contains((-175.0, -15.0)));
        assert!(!pacific.contains((0.0, -15.0)));
        assert!((pacific.area() - 200.0).abs() < 1e-9);
    }
}
