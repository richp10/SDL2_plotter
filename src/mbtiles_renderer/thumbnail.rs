//! Small previews of a chart file: the whole tile under a position, scaled
//! down. The photo button shows one. Each (file, tile) is read and scaled
//! once, then served from a cache, so asking every frame costs a lookup.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{imageops, RgbaImage};
use lru::LruCache;
use rusqlite::{Connection, OpenFlags};

use super::projection::{lonlat_to_world, TILE_SIZE};
use super::tile_store::fetch_tile_bytes;

/// Scaled tiles kept, whether found or not.
const IMAGES: usize = 32;
/// Files kept open.
const FILES: usize = 4;

type TileAt = (PathBuf, i32, i64, i64, u32);

/// One open file and the zoom levels it holds.
struct Source {
    conn: Connection,
    levels: Vec<i32>,
}

pub struct Thumbnails {
    images: LruCache<TileAt, Option<Arc<RgbaImage>>>,
    files: LruCache<PathBuf, Option<Source>>,
}

impl Thumbnails {
    pub fn new() -> Self {
        Thumbnails {
            images: LruCache::new(NonZeroUsize::new(IMAGES).unwrap()),
            files: LruCache::new(NonZeroUsize::new(FILES).unwrap()),
        }
    }

    /// The tile of `path` containing `centre` (lon, lat), at `zoom` or the
    /// file's deepest level below it (its first level if it starts deeper),
    /// scaled to `size` x `size`. `None` if the file cannot be opened or
    /// has no tile there.
    pub fn get(
        &mut self,
        path: &Path,
        centre: (f64, f64),
        zoom: i32,
        size: u32,
    ) -> Option<Arc<RgbaImage>> {
        if !self.files.contains(path) {
            let source = open(path);
            if source.is_none() {
                crate::diagnostics::record(
                    "WARN",
                    format!("Vignette impossible: {}", path.display()),
                );
            }
            self.files.put(path.to_path_buf(), source);
        }
        let source = self.files.get(path)?.as_ref()?;
        let level = source
            .levels
            .iter()
            .rev()
            .find(|&&level| level <= zoom)
            .or(source.levels.first())
            .copied()?;
        let n = 1i64 << level;
        let (wx, wy) = lonlat_to_world(centre.0, centre.1, level as f64);
        let tile = |world: f64| ((world / TILE_SIZE as f64).floor() as i64).clamp(0, n - 1);
        let (x, y) = (tile(wx), tile(wy));
        let key = (path.to_path_buf(), level, x, y, size);
        if let Some(image) = self.images.get(&key) {
            return image.clone();
        }
        let image = fetch_tile_bytes(&source.conn, level, x, y)
            .ok()
            .flatten()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|tile| {
                Arc::new(imageops::resize(
                    &tile.to_rgba8(),
                    size,
                    size,
                    imageops::FilterType::Triangle,
                ))
            });
        self.images.put(key, image.clone());
        image
    }
}

fn open(path: &Path) -> Option<Source> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let levels = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT zoom_level FROM tiles ORDER BY zoom_level")
            .ok()?;
        let rows = stmt.query_map([], |row| row.get::<_, i32>(0)).ok()?;
        rows.flatten().collect()
    };
    Some(Source { conn, levels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbtiles_renderer::test_support::{fixture_chart, fixture_chart_with};
    use image::Rgba;

    #[test]
    fn a_thumbnail_is_the_tile_under_the_centre_scaled_down() {
        let path = fixture_chart_with(&[2], Rgba([10, 200, 30, 255]));
        let mut thumbnails = Thumbnails::new();
        let image = thumbnails.get(&path, (5.0, 5.0), 2, 72).expect("a tile");
        assert_eq!(image.dimensions(), (72, 72));
        assert_eq!(image.get_pixel(36, 36), &Rgba([10, 200, 30, 255]));
        drop(thumbnails);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_same_tile_is_not_cut_twice() {
        let path = fixture_chart(&[2]);
        let mut thumbnails = Thumbnails::new();
        let first = thumbnails.get(&path, (5.0, 5.0), 2, 72).unwrap();
        let second = thumbnails.get(&path, (6.0, 6.0), 2, 72).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        drop(thumbnails);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn no_tile_under_the_centre_gives_none() {
        let path = fixture_chart(&[2]);
        Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM tiles WHERE tile_column != 0", [])
            .unwrap();
        let mut thumbnails = Thumbnails::new();
        assert!(thumbnails.get(&path, (100.0, 5.0), 2, 72).is_none());
        assert!(thumbnails.get(&path, (-170.0, 5.0), 2, 72).is_some());
        let missing = std::env::temp_dir().join(format!(
            "plotter-missing-{}.mbtiles",
            crate::mbtiles_renderer::test_support::unique_name()
        ));
        assert!(thumbnails.get(&missing, (5.0, 5.0), 2, 72).is_none());
        drop(thumbnails);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_zoom_past_the_files_levels_uses_its_deepest_level() {
        let path = fixture_chart(&[1, 2]);
        let mut thumbnails = Thumbnails::new();
        assert!(thumbnails.get(&path, (5.0, 5.0), 9, 72).is_some());
        assert!(thumbnails.get(&path, (5.0, 5.0), 0, 72).is_some());
        drop(thumbnails);
        std::fs::remove_file(path).unwrap();
    }
}
