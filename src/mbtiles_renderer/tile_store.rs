//! Tiles from an MBTiles file: the SQLite connection, the chart metadata,
//! the LRU cache of decoded tiles and the loader thread that decodes in the
//! background. Knows nothing about screens, zoom animation or rotation.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use image::RgbaImage;
use lru::LruCache;
use rusqlite::{Connection, OpenFlags};

/// A decoded tile. `opaque` lets the renderer copy instead of alpha-blend.
pub(super) struct Tile {
    pub image: RgbaImage,
    pub opaque: bool,
}

/// `None` is remembered too, so a hole in the chart is not re-queried.
pub(super) type CachedTile = Option<Arc<Tile>>;

/// `(zoom, column, row)` in XYZ order; the TMS flip happens at the SQL.
pub(crate) type TileKey = (i32, i64, i64);

/// Decodes tiles on its own thread with its own read-only connection, so a
/// frame is never held up by PNG decoding. Dropping it joins the thread.
struct TileLoader {
    requests: Option<Sender<TileKey>>,
    results: Receiver<(TileKey, CachedTile)>,
    in_flight: HashSet<TileKey>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TileLoader {
    fn start(mbtiles_path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            mbtiles_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        let (request_tx, request_rx) = channel::<TileKey>();
        let (result_tx, result_rx) = channel::<(TileKey, CachedTile)>();
        let worker = std::thread::Builder::new()
            .name("tile-loader".into())
            .spawn(move || {
                for key in request_rx {
                    let (z, x, y) = key;
                    let tile = decode_tile(fetch_tile_bytes(&conn, z, x, y).unwrap_or(None));
                    if result_tx.send((key, tile)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn tile loader thread");
        Ok(TileLoader {
            requests: Some(request_tx),
            results: result_rx,
            in_flight: HashSet::new(),
            worker: Some(worker),
        })
    }

    fn is_pending(&self, key: &TileKey) -> bool {
        self.in_flight.contains(key)
    }

    fn request(&mut self, key: TileKey) {
        if self.in_flight.insert(key) {
            if let Some(requests) = &self.requests {
                let _ = requests.send(key);
            }
        }
    }

    fn drain_into(&mut self, cache: &mut LruCache<TileKey, CachedTile>) -> usize {
        let mut received = 0;
        while let Ok((key, tile)) = self.results.try_recv() {
            self.in_flight.remove(&key);
            cache.put(key, tile);
            received += 1;
        }
        received
    }
}

impl Drop for TileLoader {
    /// Closes the request channel so the worker's loop ends, then waits for
    /// it, so the file is no longer open when the store is gone.
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Raw tile bytes, or `None` when the chart has no tile there. MBTiles rows
/// are TMS (row 0 at the south), so the XYZ row is flipped.
fn fetch_tile_bytes(
    conn: &Connection,
    z: i32,
    x: i64,
    y: i64,
) -> rusqlite::Result<Option<Vec<u8>>> {
    let n: i64 = 1i64 << z;
    let tms_y = n - 1 - y;
    conn.query_row(
        "SELECT tile_data FROM tiles WHERE zoom_level=?1 AND tile_column=?2 AND tile_row=?3",
        (z, x, tms_y),
        |row| row.get::<_, Vec<u8>>(0),
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

fn decode_tile(bytes: Option<Vec<u8>>) -> CachedTile {
    bytes
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|dyn_img| {
            let has_alpha = dyn_img.color().has_alpha();
            let image = dyn_img.to_rgba8();
            let opaque = !has_alpha || image.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255);
            Arc::new(Tile { image, opaque })
        })
}

/// Reads a chart centre from MBTiles metadata: the `center` key
/// ("lon,lat[,zoom]") first, then the middle of `bounds` ("w,s,e,n").
fn center_from_metadata(meta: &HashMap<String, String>) -> Option<(f64, f64)> {
    fn numbers(value: &str) -> Vec<f64> {
        value
            .split(',')
            .filter_map(|part| part.trim().parse::<f64>().ok())
            .collect()
    }
    fn valid(lon: f64, lat: f64) -> Option<(f64, f64)> {
        (lon.is_finite()
            && lat.is_finite()
            && (-180.0..=180.0).contains(&lon)
            && (-90.0..=90.0).contains(&lat))
        .then_some((lon, lat))
    }

    if let Some(parts) = meta.get("center").map(|v| numbers(v)) {
        if parts.len() >= 2 {
            if let Some(center) = valid(parts[0], parts[1]) {
                return Some(center);
            }
        }
    }
    if let Some(parts) = meta.get("bounds").map(|v| numbers(v)) {
        if parts.len() == 4 {
            return valid((parts[0] + parts[2]) / 2.0, (parts[1] + parts[3]) / 2.0);
        }
    }
    None
}

/// One open MBTiles chart with its decoded-tile cache.
///
/// Zoom levels come from the `tiles` table, not the metadata, so
/// `available_zooms` is what can actually be drawn and may have gaps.
/// `min_zoom`/`max_zoom` prefer the metadata and fall back to the table.
pub(super) struct TileStore {
    conn: Connection,
    metadata: HashMap<String, String>,
    min_zoom: i32,
    max_zoom: i32,
    available_zooms: Vec<i32>,
    cache: LruCache<TileKey, CachedTile>,
    loader: TileLoader,
}

impl TileStore {
    /// Opens the chart read-only and reads its metadata and zoom levels.
    /// `cache_size` is the number of decoded tiles kept (at least one).
    pub(super) fn open(mbtiles_path: &str, cache_size: usize) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            mbtiles_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;

        // The metadata table may be missing altogether; cope without it.
        let mut metadata = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT name, value FROM metadata") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (name, value) in rows.flatten() {
                    metadata.insert(name, value);
                }
            }
        }

        let parsed_min = metadata.get("minzoom").and_then(|v| v.parse::<i32>().ok());
        let parsed_max = metadata.get("maxzoom").and_then(|v| v.parse::<i32>().ok());
        let (min_zoom, max_zoom) = match (parsed_min, parsed_max) {
            (Some(min), Some(max)) => (min, max),
            _ => {
                let (min_row, max_row): (Option<i64>, Option<i64>) = conn.query_row(
                    "SELECT MIN(zoom_level), MAX(zoom_level) FROM tiles",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                (min_row.unwrap_or(0) as i32, max_row.unwrap_or(18) as i32)
            }
        };

        let mut available_zooms = Vec::new();
        if let Ok(mut stmt) =
            conn.prepare("SELECT DISTINCT zoom_level FROM tiles ORDER BY zoom_level")
        {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i32>(0)) {
                available_zooms = rows.flatten().collect();
            }
        }

        Ok(TileStore {
            conn,
            metadata,
            min_zoom,
            max_zoom,
            available_zooms,
            cache: LruCache::new(NonZeroUsize::new(cache_size.max(1)).unwrap()),
            loader: TileLoader::start(mbtiles_path)?,
        })
    }

    #[cfg(test)]
    pub(super) fn min_zoom(&self) -> i32 {
        self.min_zoom
    }

    pub(super) fn available_zooms(&self) -> &[i32] {
        &self.available_zooms
    }

    pub(super) fn clamp_zoom(&self, zoom: i32) -> i32 {
        zoom.clamp(self.min_zoom, self.max_zoom)
    }

    /// The level the chart has that is closest to `target`, if it has any.
    pub(super) fn nearest_zoom(&self, target: i32) -> Option<i32> {
        self.available_zooms
            .iter()
            .copied()
            .min_by_key(|&z| (z - target).abs())
    }

    /// Centre declared in the metadata (`center`, then `bounds`).
    pub(super) fn metadata_center(&self) -> Option<(f64, f64)> {
        center_from_metadata(&self.metadata)
    }

    /// Centre of the tiles actually present at the minimum zoom. Handles
    /// charts that straddle the antimeridian (Pacific) by finding the
    /// largest empty gap in the occupied columns.
    pub(super) fn tiles_center(&self) -> Option<(f64, f64)> {
        let z = self.min_zoom;
        let n = (1i64 << z) as f64;
        let ni = 1i64 << z;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT tile_column FROM tiles WHERE zoom_level=?1 ORDER BY tile_column",
            )
            .ok()?;
        let cols: Vec<i64> = stmt
            .query_map([z], |r| r.get::<_, i64>(0))
            .ok()?
            .flatten()
            .collect();
        if cols.is_empty() {
            return None;
        }

        let mut best_gap = -1i64;
        let mut start = cols[0];
        for i in 0..cols.len() {
            let cur = cols[i];
            let next = if i + 1 < cols.len() {
                cols[i + 1]
            } else {
                cols[0] + ni
            };
            let gap = next - cur - 1;
            if gap > best_gap {
                best_gap = gap;
                start = next % ni;
            }
        }

        let span = (ni - best_gap) as f64;
        let center_col = (start as f64 + span / 2.0) % n;
        let mut lon = center_col / n * 360.0 - 180.0;
        if lon >= 180.0 {
            lon -= 360.0;
        }

        let (rmin, rmax): (i64, i64) = self
            .conn
            .query_row(
                "SELECT MIN(tile_row), MAX(tile_row) FROM tiles WHERE zoom_level=?1",
                [z],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        let y_top = (ni - 1 - rmax) as f64;
        let y_bottom = (ni - 1 - rmin) as f64 + 1.0;
        let y_mid = (y_top + y_bottom) / 2.0;

        let merc = std::f64::consts::PI * (1.0 - 2.0 * y_mid / n);
        let lat = merc.sinh().atan().to_degrees();
        Some((lon, lat))
    }

    /// The tile from the cache, or fetched and decoded now on this thread
    /// when a frame needs it before the loader has delivered it.
    pub(super) fn tile(&mut self, key: TileKey) -> CachedTile {
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let (z, x, y) = key;
        let tile = decode_tile(fetch_tile_bytes(&self.conn, z, x, y).unwrap_or(None));
        self.cache.put(key, tile.clone());
        tile
    }

    /// True when the tile is cached or already on its way from the loader.
    pub(super) fn is_known(&self, key: &TileKey) -> bool {
        self.cache.contains(key) || self.loader.is_pending(key)
    }

    /// Asks the loader thread for a tile. Cheap; never decodes here.
    pub(super) fn request(&mut self, key: TileKey) {
        self.loader.request(key);
    }

    /// Moves every tile the loader has finished into the cache.
    pub(super) fn drain(&mut self) -> usize {
        self.loader.drain_into(&mut self.cache)
    }

    pub(super) fn has_in_flight(&self) -> bool {
        !self.loader.in_flight.is_empty()
    }

    #[cfg(test)]
    pub(super) fn cached_count(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(super) fn fetch_tile_bytes(
        &self,
        z: i32,
        x: i64,
        y: i64,
    ) -> rusqlite::Result<Option<Vec<u8>>> {
        fetch_tile_bytes(&self.conn, z, x, y)
    }
}

/// Fixture charts shared by the store's and the renderer's tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::super::projection::TILE_SIZE;
    use image::{Rgba, RgbaImage};
    use rusqlite::Connection;

    /// A name no other test of this run uses: the process id, the clock and
    /// a counter, because tests run in parallel and two of them can read the
    /// same clock value (Windows counts in 100 ns steps).
    pub(crate) fn unique_name() -> String {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let count = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}-{nanos}-{count}", std::process::id())
    }

    /// Writes a small raster MBTiles file holding every tile of the given
    /// zoom levels, each a solid-colour 256x256 PNG.
    pub(crate) fn fixture_chart(zooms: &[i32]) -> std::path::PathBuf {
        fixture_chart_with(zooms, Rgba([0, 120, 200, 255]))
    }

    pub(crate) fn fixture_chart_with(zooms: &[i32], colour: Rgba<u8>) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("plotter-fixture-{}.mbtiles", unique_name()));
        let mut png = Vec::new();
        RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, colour)
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageOutputFormat::Png,
            )
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (name TEXT, value TEXT);
             CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);",
        )
        .unwrap();
        for &z in zooms {
            let n = 1i64 << z;
            for x in 0..n {
                for y in 0..n {
                    conn.execute("INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)", (z, x, y, &png))
                        .unwrap();
                }
            }
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fixture_chart;
    use super::*;

    #[test]
    fn zoom_range_and_levels_come_from_the_tiles_when_metadata_is_silent() {
        let path = fixture_chart(&[2, 5]);
        let store = TileStore::open(path.to_str().unwrap(), 8).unwrap();
        assert_eq!(store.available_zooms(), &[2, 5]);
        assert_eq!(store.min_zoom(), 2);
        assert_eq!(store.clamp_zoom(9), 5);
        assert_eq!(store.clamp_zoom(0), 2);
        assert_eq!(store.nearest_zoom(4), Some(5));
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn metadata_zoom_range_wins_over_the_tiles() {
        let path = fixture_chart(&[3]);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO metadata VALUES ('minzoom', '1'); INSERT INTO metadata VALUES ('maxzoom', '7');",
        )
        .unwrap();
        drop(conn);
        let store = TileStore::open(path.to_str().unwrap(), 8).unwrap();
        assert_eq!(store.min_zoom(), 1);
        assert_eq!(store.clamp_zoom(9), 7);
        assert_eq!(
            store.available_zooms(),
            &[3],
            "levels still reflect the tiles present"
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_present_tile_decodes_and_a_missing_one_is_remembered_as_none() {
        let path = fixture_chart(&[1]);
        let mut store = TileStore::open(path.to_str().unwrap(), 8).unwrap();
        let tile = store.tile((1, 0, 1)).expect("tile (1,0,1) exists");
        assert_eq!(tile.image.dimensions(), (256, 256));
        assert!(tile.opaque);
        assert!(
            store.tile((4, 0, 0)).is_none(),
            "zoom 4 is not in the chart"
        );
        assert!(store.is_known(&(4, 0, 0)), "the miss is cached too");
        assert_eq!(store.cached_count(), 2);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn requested_tiles_arrive_through_drain() {
        let path = fixture_chart(&[1]);
        let mut store = TileStore::open(path.to_str().unwrap(), 8).unwrap();
        store.request((1, 1, 1));
        assert!(store.is_known(&(1, 1, 1)), "pending counts as known");
        assert!(store.has_in_flight());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while store.cached_count() == 0 {
            assert!(std::time::Instant::now() < deadline, "tile never arrived");
            std::thread::sleep(std::time::Duration::from_millis(5));
            store.drain();
        }
        assert!(!store.has_in_flight());
        assert!(store.tile((1, 1, 1)).is_some());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn metadata_center_is_read_from_center_then_bounds() {
        let mut meta = HashMap::new();
        meta.insert("bounds".to_string(), "10.0, 40.0, 30.0, 50.0".to_string());
        assert_eq!(center_from_metadata(&meta), Some((20.0, 45.0)));

        meta.insert("center".to_string(), "26.98,36.95,12".to_string());
        assert_eq!(center_from_metadata(&meta), Some((26.98, 36.95)));

        meta.insert("center".to_string(), "garbage".to_string());
        assert_eq!(center_from_metadata(&meta), Some((20.0, 45.0)));

        assert_eq!(center_from_metadata(&HashMap::new()), None);
    }
}
