//! The chart view: which part of the world is on screen, at what zoom and
//! rotation, and how it becomes RGBA pixels for the SDL texture.
//!
//! The maths lives in [`projection`] and the tiles in [`tile_store`];
//! `MBTilesRenderer` composes the two and is the only type the rest of the
//! application sees.

mod projection;
mod tile_store;
mod view;

use std::time::{Duration, Instant};

use image::{imageops, ImageBuffer, Rgba, RgbaImage};
use lru::LruCache;
#[cfg(test)]
use sdl2::rect::Point;
use sdl2::rect::Rect;
use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::config::RenderingConfig;
pub use projection::project_norm;
use projection::{
    lonlat_to_world, rotated_bounds, screen_to_world_delta, smooth_zoom_progress,
    source_dimensions, source_zoom_for, world_to_lonlat, TILE_SIZE,
};
#[cfg(test)]
pub(crate) use tile_store::test_support;
use tile_store::{TileKey, TileStore};
pub use view::ChartView;

const ZOOM_ANIMATION_DURATION: Duration = Duration::from_millis(240);
const ROTATION_MARGIN: u32 = 4;
const MAP_BACKGROUND: Rgba<u8> = Rgba([245, 245, 245, 255]);

struct Beneath {
    path: String,
    store: TileStore,
    scaled: LruCache<TileKey, Option<Arc<RgbaImage>>>,
}

pub struct MBTilesRenderer {
    width: u32,
    height: u32,
    render_width: u32,
    render_height: u32,
    zoom: i32,
    display_zoom: f64,
    zoom_animation: Option<(f64, Instant)>,
    rotation: f64,
    rendering_config: RenderingConfig,
    /// The chart file being drawn.
    path: String,
    /// The less detailed file drawn where `store` has no tile (or a
    /// transparent one), and its tiles already cut and scaled to the main
    /// file's grid. See `beneath_tile`.
    beneath: Option<Beneath>,
    /// What the screen is looking at, as the application states it.
    view: View,
    /// `view` projected to world pixels at the current `display_zoom` and
    /// `rotation`. Derived; recomputed by `reproject` whenever either input
    /// changes, so input and rendering always agree on the geometry.
    center_world: (f64, f64),
    store: TileStore,
}

/// The position shown at the centre of the screen, shifted by a pan offset
/// in screen pixels (the chart is dragged by `offset`, so the position
/// appears `offset` away from the centre).
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    lon: f64,
    lat: f64,
    offset: (i32, i32),
}

impl MBTilesRenderer {
    pub fn new(
        mbtiles_path: &str,
        zoom: i32,
        width: u32,
        height: u32,
        config: &RenderingConfig,
    ) -> rusqlite::Result<Self> {
        let (render_width, render_height) = Self::texture_dimensions(width, height);
        let store = TileStore::open(mbtiles_path, config.tile_cache_size)?;
        let zoom = store
            .nearest_zoom(zoom)
            .unwrap_or_else(|| store.clamp_zoom(zoom));
        crate::diagnostics::record("DEBUG", format!("Zoom initial {zoom} pour {mbtiles_path}"));

        Ok(Self {
            width,
            height,
            render_width,
            render_height,
            zoom,
            display_zoom: zoom as f64,
            zoom_animation: None,
            rotation: 0.0,
            rendering_config: config.clone(),
            path: mbtiles_path.to_string(),
            beneath: None,
            view: View {
                lon: 0.0,
                lat: 0.0,
                offset: (0, 0),
            },
            center_world: lonlat_to_world(0.0, 0.0, zoom as f64),
            store,
        })
    }

    /// Points the view at `(lon, lat)`, dragged by `offset` screen pixels.
    /// Call it whenever the application's idea of the view changes; input
    /// and rendering both read the result, so there is no stale geometry
    /// between a state change and the next frame.
    pub fn look_at(&mut self, lon: f64, lat: f64, offset: (i32, i32)) {
        self.view = View { lon, lat, offset };
        self.reproject();
    }

    fn reproject(&mut self) {
        let (mut x, mut y) = lonlat_to_world(self.view.lon, self.view.lat, self.display_zoom);
        let (offset_x, offset_y) = screen_to_world_delta(
            self.view.offset.0 as f64,
            self.view.offset.1 as f64,
            self.rotation,
        );
        x -= offset_x;
        y -= offset_y;
        self.center_world = (x, y);
    }

    pub fn texture_dimensions(width: u32, height: u32) -> (u32, u32) {
        let side = ((width as f64).hypot(height as f64)).ceil() as u32 + ROTATION_MARGIN;
        let scaled_side = side * 2 + ROTATION_MARGIN;
        (scaled_side, scaled_side)
    }

    fn rotation_degrees(&self) -> f64 {
        -self.rotation.to_degrees()
    }

    /// Everything `screen::Screen` needs to place the chart texture this frame.
    pub fn placement(&self) -> crate::screen::ChartPlacement {
        let (display_width, display_height) = self.display_dimensions();
        crate::screen::ChartPlacement {
            source: self.texture_rect(),
            display_width,
            display_height,
            rotation_degrees: self.rotation_degrees(),
        }
    }

    fn render_rect(&self) -> Rect {
        let (width, height) = if self.rotation.abs() < f64::EPSILON {
            (self.width, self.height)
        } else {
            let (width, height) = rotated_bounds(self.width, self.height, self.rotation);
            (width + ROTATION_MARGIN, height + ROTATION_MARGIN)
        };
        Rect::new(
            (self.render_width - width) as i32 / 2,
            (self.render_height - height) as i32 / 2,
            width,
            height,
        )
    }

    fn texture_rect(&self) -> Rect {
        let display_rect = self.render_rect();
        let zoom_scale = 2.0_f64.powf(self.display_zoom - self.source_zoom() as f64);
        let (width, height) =
            source_dimensions(display_rect.width(), display_rect.height(), zoom_scale);
        Rect::new(
            (self.render_width - width) as i32 / 2,
            (self.render_height - height) as i32 / 2,
            width,
            height,
        )
    }

    fn display_dimensions(&self) -> (u32, u32) {
        let rect = self.render_rect();
        (rect.width(), rect.height())
    }

    /// Collects tiles the loader has finished and asks it for up to `budget`
    /// missing tiles around the view. Never decodes on the calling thread.
    /// Returns true while tiles are still missing or being loaded.
    pub fn prefetch_step(&mut self, budget: usize) -> bool {
        self.store.drain();
        let source_zoom = self.source_zoom();
        let n_tiles: i64 = 1i64 << source_zoom;
        let tile_size = TILE_SIZE as f64;
        let zoom_scale = 2.0_f64.powf(self.display_zoom - source_zoom as f64);
        let source_center_x = self.center_world.0 / zoom_scale;
        let source_center_y = self.center_world.1 / zoom_scale;
        let (view_width, view_height) = rotated_bounds(self.width, self.height, self.rotation);
        let origin_x = source_center_x - view_width as f64 / zoom_scale / 2.0;
        let origin_y = source_center_y - view_height as f64 / zoom_scale / 2.0;
        let margin = self.rendering_config.prefetch_margin;

        let first_tx = (origin_x / tile_size).floor() as i64 - margin;
        let first_ty = (origin_y / tile_size).floor() as i64 - margin;
        let last_tx =
            ((origin_x + view_width as f64 / zoom_scale) / tile_size).floor() as i64 + margin;
        let last_ty =
            ((origin_y + view_height as f64 / zoom_scale) / tile_size).floor() as i64 + margin;

        let mut requested = 0;
        for tx in first_tx..=last_tx {
            for ty in first_ty..=last_ty {
                if ty < 0 || ty >= n_tiles {
                    continue;
                }
                let key = (source_zoom, tx.rem_euclid(n_tiles), ty);
                if self.store.is_known(&key) {
                    continue;
                }
                if requested >= budget {
                    return true; // il en reste
                }
                self.store.request(key);
                requested += 1;
            }
        }
        self.store.has_in_flight()
    }

    fn source_zoom(&self) -> i32 {
        source_zoom_for(self.display_zoom, self.store.available_zooms(), self.zoom)
    }

    /// Animates to `zoom`. Which levels make sense is the catalogue's
    /// decision, not this file's: past the file's deepest level the tiles
    /// are drawn overscaled, above its first level underscaled. Asking for
    /// the current level changes nothing, so the tile cache is kept.
    pub fn zoom_to(&mut self, zoom: i32) {
        if self.zoom == zoom {
            return;
        }
        self.zoom = zoom;
        if (self.display_zoom - zoom as f64).abs() < f64::EPSILON {
            self.zoom_animation = None;
        } else {
            self.zoom_animation = Some((self.display_zoom, Instant::now()));
        }
    }

    /// The whole zoom level shown or being animated to.
    pub fn zoom(&self) -> i32 {
        self.zoom
    }

    /// The deepest level the open file holds.
    pub fn top_zoom(&self) -> i32 {
        self.store
            .available_zooms()
            .last()
            .copied()
            .unwrap_or(self.zoom)
    }

    /// The chart file being drawn.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Draws `path` beneath the chart wherever the chart has no tile, or
    /// nothing beneath with `None`. Opening the same file again is free; a
    /// file that fails to open leaves nothing beneath.
    pub fn set_beneath(&mut self, path: Option<&str>) -> rusqlite::Result<()> {
        if path == self.beneath_path() {
            return Ok(());
        }
        self.beneath = None;
        if let Some(path) = path {
            let cache = self.rendering_config.tile_cache_size;
            self.beneath = Some(Beneath {
                path: path.to_string(),
                store: TileStore::open(path, cache)?,
                scaled: LruCache::new(NonZeroUsize::new(cache.max(1)).unwrap()),
            });
        }
        Ok(())
    }

    pub fn beneath_path(&self) -> Option<&str> {
        self.beneath.as_ref().map(|b| b.path.as_str())
    }

    pub fn advance_zoom_animation(&mut self) -> bool {
        let Some((start_zoom, started_at)) = self.zoom_animation else {
            return false;
        };

        let progress =
            (started_at.elapsed().as_secs_f64() / ZOOM_ANIMATION_DURATION.as_secs_f64()).min(1.0);
        let eased = smooth_zoom_progress(progress);
        let next_zoom = start_zoom + (self.zoom as f64 - start_zoom) * eased;
        self.display_zoom = next_zoom;

        if progress >= 1.0 {
            self.display_zoom = self.zoom as f64;
            self.zoom_animation = None;
        }
        self.reproject();
        true
    }

    pub fn set_rotation(&mut self, radians: f64) {
        self.rotation = if radians.is_finite() {
            radians.rem_euclid(std::f64::consts::TAU)
        } else {
            0.0
        };
        self.reproject();
    }

    pub fn change_map(&mut self, mbtiles_path: &str) -> rusqlite::Result<()> {
        // On construit le nouveau renderer d'abord : si ça échoue, self n'est pas touché.
        // Le cache est neuf (donc vide), les métadonnées et les zooms sont rechargés,
        // et le zoom courant est ramené dans [min_zoom, max_zoom] de la nouvelle carte.
        let mut new_renderer = Self::new(
            mbtiles_path,
            self.zoom,
            self.width,
            self.height,
            &self.rendering_config,
        )?;

        // The view and the zoom carry over, mid-animation included: which
        // file fits the zoom was decided by the caller.
        new_renderer.zoom = self.zoom;
        new_renderer.display_zoom = self.display_zoom;
        new_renderer.zoom_animation = self.zoom_animation;
        // The file beneath stays until the caller says otherwise; its cut
        // tiles were scaled for the old file's levels.
        new_renderer.beneath = self.beneath.take().map(|mut beneath| {
            beneath.scaled.clear();
            beneath
        });
        new_renderer.set_rotation(self.rotation);
        new_renderer.look_at(self.view.lon, self.view.lat, self.view.offset);

        *self = new_renderer;
        Ok(())
    }

    /// Position to show when no position has been saved yet. Prefers the
    /// centre of the tiles actually present, then the chart metadata, and
    /// finally (0, 0) so that a sparse or empty chart never stops startup.
    pub fn initial_center(&self) -> (f64, f64) {
        if let Some(center) = self.store.tiles_center() {
            return center;
        }
        match self.store.metadata_center() {
            Some(center) => {
                crate::diagnostics::record(
                    "WARN",
                    "Aucune tuile au zoom minimal; centre pris dans les métadonnées de la carte",
                );
                center
            }
            None => {
                crate::diagnostics::record(
                    "WARN",
                    "Aucune tuile ni métadonnée de centre; carte centrée sur (0, 0)",
                );
                (0.0, 0.0)
            }
        }
    }

    /// Centre of the tiles actually present at the minimum zoom.
    #[cfg(test)]
    pub fn tiles_center(&self) -> Option<(f64, f64)> {
        self.store.tiles_center()
    }

    /// Renders the current view (see `look_at`) into RGBA pixels and returns
    /// them with the texture rectangle they fill.
    pub fn render_frame(&mut self) -> (Vec<u8>, Rect) {
        let (center_x, center_y) = self.center_world;
        let (image, rect) = self.render_view(center_x, center_y);
        (image.into_raw(), rect)
    }

    pub fn center_with_offset(&self, center: (f64, f64), offset: (i32, i32)) -> (f64, f64) {
        let (center_x, center_y) = self.project(center.0, center.1);
        let (offset_x, offset_y) =
            screen_to_world_delta(offset.0 as f64, offset.1 as f64, self.rotation);
        self.pixel_to_lonlat(center_x - offset_x, center_y - offset_y)
    }

    /// The screen-to-chart conversions for the current view, as a copyable
    /// snapshot. Input, hit testing and overlays all go through it.
    pub fn view(&self) -> ChartView {
        ChartView::new(
            self.center_world,
            self.display_zoom,
            self.rotation,
            self.width,
            self.height,
        )
    }

    #[cfg(test)]
    pub fn gps_to_point(&self, pos: (f64, f64)) -> Point {
        self.view().to_pixel(pos)
    }

    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        lonlat_to_world(lon, lat, self.display_zoom)
    }

    fn pixel_to_lonlat(&self, x: f64, y: f64) -> (f64, f64) {
        world_to_lonlat(x, y, self.display_zoom)
    }

    fn render_view(&mut self, center_x: f64, center_y: f64) -> (RgbaImage, Rect) {
        let source_zoom = self.source_zoom();
        let zoom_scale = 2.0_f64.powf(self.display_zoom - source_zoom as f64);
        let rect = self.texture_rect();
        let view = self.render_tiles(
            source_zoom,
            center_x / zoom_scale,
            center_y / zoom_scale,
            rect.width(),
            rect.height(),
        );
        (view, rect)
    }

    fn render_tiles(
        &mut self,
        zoom: i32,
        center_x: f64,
        center_y: f64,
        width: u32,
        height: u32,
    ) -> RgbaImage {
        let n_tiles: i64 = 1i64 << zoom;
        let tile_size = TILE_SIZE as f64;

        let origin_x = center_x - width as f64 / 2.0;
        let origin_y = center_y - height as f64 / 2.0;

        let mut canvas: RgbaImage = ImageBuffer::from_pixel(width, height, MAP_BACKGROUND);

        let first_tx = (origin_x / tile_size).floor() as i64;
        let first_ty = (origin_y / tile_size).floor() as i64;
        let last_tx = ((origin_x + width as f64) / tile_size).floor() as i64;
        let last_ty = ((origin_y + height as f64) / tile_size).floor() as i64;

        for tx in first_tx..=last_tx {
            for ty in first_ty..=last_ty {
                if ty < 0 || ty >= n_tiles {
                    continue;
                }

                let wrapped_x = tx.rem_euclid(n_tiles);

                let px = (tx as f64 * tile_size - origin_x).round() as i64;
                let py = (ty as f64 * tile_size - origin_y).round() as i64;
                let tile = self.store.tile((zoom, wrapped_x, ty));
                if tile.as_ref().is_none_or(|t| !t.opaque) {
                    if let Some(under) = self.beneath_tile((zoom, wrapped_x, ty)) {
                        imageops::replace(&mut canvas, under.as_ref(), px, py);
                    }
                }
                if let Some(tile) = tile {
                    if tile.opaque {
                        // Plain copy: no per-pixel alpha arithmetic needed.
                        imageops::replace(&mut canvas, &tile.image, px, py);
                    } else {
                        // Alpha blend, for overlay charts with transparency.
                        imageops::overlay(&mut canvas, &tile.image, px, py);
                    }
                }
            }
        }

        canvas
    }

    /// The square of the file beneath that lies under main tile `key`, cut
    /// from its closest level at or above the main level and scaled up to a
    /// whole tile. Cached, so each square is cut once.
    fn beneath_tile(&mut self, key: TileKey) -> Option<Arc<RgbaImage>> {
        let beneath = self.beneath.as_mut()?;
        if let Some(scaled) = beneath.scaled.get(&key) {
            return scaled.clone();
        }
        let (zoom, x, y) = key;
        let level = beneath
            .store
            .available_zooms()
            .iter()
            .copied()
            .filter(|&z| z <= zoom)
            .max();
        let scaled = level.and_then(|level| {
            let d = (zoom - level).min(30);
            let tile = beneath.store.tile((level, x >> d, y >> d))?;
            let size = TILE_SIZE;
            let part = (size >> d.min(8)).max(1);
            let mask = (1i64 << d) - 1;
            let offset = |v: i64| (((v & mask) * size as i64) >> d) as u32;
            let piece =
                imageops::crop_imm(&tile.image, offset(x), offset(y), part, part).to_image();
            Some(Arc::new(imageops::resize(
                &piece,
                size,
                size,
                imageops::FilterType::Nearest,
            )))
        });
        beneath.scaled.put(key, scaled.clone());
        scaled
    }
}

#[cfg(test)]
mod tests {
    use super::tile_store::test_support::{fixture_chart, fixture_chart_with};
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn texture_dimensions_cover_full_screen_after_rotation() {
        let (width, height) = MBTilesRenderer::texture_dimensions(1600, 800);
        assert_eq!(width, height);
        assert!(width >= 2 * 1600.0_f64.hypot(800.0) as u32);
    }

    /// Renders a 512x256 view centred on (0, 0) and returns the centre pixel.
    fn centre_pixel_of(path: &std::path::Path) -> Rgba<u8> {
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 2, 512, 256, &RenderingConfig::default()).unwrap();
        renderer.look_at(0.0, 0.0, (0, 0));
        let (pixels, rect) = renderer.render_frame();
        let (width, height) = (rect.width() as usize, rect.height() as usize);
        let index = ((height / 2) * width + width / 2) * 4;
        Rgba([
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ])
    }

    #[test]
    fn opaque_tiles_are_copied_exactly() {
        let path = fixture_chart_with(&[2], Rgba([10, 20, 30, 255]));
        assert_eq!(centre_pixel_of(&path), Rgba([10, 20, 30, 255]));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn translucent_tiles_blend_over_the_map_background() {
        // Black at half opacity over the light grey background.
        let path = fixture_chart_with(&[2], Rgba([0, 0, 0, 128]));
        let Rgba([r, g, b, a]) = centre_pixel_of(&path);
        let expected = MAP_BACKGROUND.0[0] as i32 / 2; // about 122
        assert!((r as i32 - expected).abs() <= 2, "r={r}");
        assert_eq!((r, g), (g, b));
        assert!(a >= 254, "a={a}"); // blending rounds the alpha down by one
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn initial_center_falls_back_when_the_chart_has_no_tiles() {
        let path = fixture_chart(&[]);
        let path_str = path.to_str().unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("INSERT INTO metadata VALUES ('bounds', '-10,50,0,60')", [])
            .unwrap();
        drop(conn);

        let renderer =
            MBTilesRenderer::new(path_str, 8, 512, 256, &RenderingConfig::default()).unwrap();
        assert_eq!(renderer.tiles_center(), None);
        assert_eq!(renderer.initial_center(), (-5.0, 55.0));
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn prefetch_loads_tiles_in_the_background_without_blocking_the_caller() {
        let path = fixture_chart(&[3]); // 64 tiles, all in view plus margin at 512x256
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 3, 512, 256, &RenderingConfig::default()).unwrap();
        renderer.look_at(0.0, 0.0, (0, 0));
        assert_eq!(renderer.store.cached_count(), 0);

        let started = Instant::now();
        renderer.prefetch_step(64);
        let first_call = started.elapsed();

        let deadline = Instant::now() + Duration::from_secs(15);
        while renderer.store.cached_count() < 20 {
            assert!(
                Instant::now() < deadline,
                "only {} tiles arrived",
                renderer.store.cached_count()
            );
            std::thread::sleep(Duration::from_millis(10));
            renderer.prefetch_step(64);
        }
        // Decoding twenty or more PNG tiles synchronously takes far longer
        // than this, even in release; the caller must only enqueue work.
        assert!(
            first_call < Duration::from_millis(50),
            "prefetch_step took {first_call:?}"
        );
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn zooming_to_the_current_level_keeps_the_tile_cache() {
        let path = fixture_chart(&[2, 3]);
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 3, 512, 256, &RenderingConfig::default()).unwrap();
        assert_eq!(renderer.zoom, 3);
        renderer.look_at(0.0, 0.0, (0, 0));
        while renderer.prefetch_step(64) {}
        let cached_before = renderer.store.cached_count();
        assert!(cached_before > 0);

        renderer.zoom_to(3);

        assert_eq!(renderer.store.cached_count(), cached_before);
        assert!(renderer.zoom_animation.is_none());
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    fn renderer_for(path: &std::path::Path, zoom: i32) -> MBTilesRenderer {
        let path_str = path.to_str().unwrap();
        MBTilesRenderer::new(path_str, zoom, 512, 256, &RenderingConfig::default()).unwrap()
    }

    fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-6 && (actual.1 - expected.1).abs() < 1e-6,
            "got {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn the_screen_centre_maps_to_the_viewed_position_before_any_frame_is_rendered() {
        let path = fixture_chart(&[2]);
        let mut renderer = renderer_for(&path, 2);
        let brest = (-4.4861, 48.3904);

        renderer.look_at(brest.0, brest.1, (0, 0));

        assert_close(
            renderer.view().to_lonlat(Point::new(256, 128)).unwrap(),
            brest,
        );
        assert_eq!(renderer.gps_to_point(brest), Point::new(256, 128));
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_pan_offset_moves_the_viewed_position_away_from_the_screen_centre() {
        let path = fixture_chart(&[2]);
        let mut renderer = renderer_for(&path, 2);

        // Dragging the chart 100 px right shows the position 100 px right of centre.
        renderer.look_at(0.0, 0.0, (100, 0));

        assert_eq!(renderer.gps_to_point((0.0, 0.0)), Point::new(356, 128));
        assert_close(
            renderer.view().to_lonlat(Point::new(356, 128)).unwrap(),
            (0.0, 0.0),
        );
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    /// The colour drawn `dx` pixels right of the centre of the frame.
    fn pixel_right_of_centre(renderer: &mut MBTilesRenderer, dx: i64) -> [u8; 4] {
        let (pixels, rect) = renderer.render_frame();
        let (width, height) = (rect.width() as i64, rect.height() as i64);
        let index = (((height / 2) * width + width / 2 + dx) * 4) as usize;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    #[test]
    fn where_the_chart_has_no_tile_the_file_beneath_fills_in() {
        // The chart has only column 1 at zoom 2 (90 W..0, west of centre);
        // the file beneath covers the whole world at zoom 1.
        let chart = fixture_chart_with(&[2], Rgba([200, 0, 0, 255]));
        Connection::open(&chart)
            .unwrap()
            .execute("DELETE FROM tiles WHERE tile_column != 1", [])
            .unwrap();
        let under = fixture_chart_with(&[1], Rgba([0, 0, 200, 255]));
        let mut renderer = renderer_for(&chart, 2);
        renderer.look_at(0.0, 0.0, (0, 0));
        let red = [200, 0, 0, 255];
        let blue = [0, 0, 200, 255];
        let background = MAP_BACKGROUND.0;

        assert_eq!(pixel_right_of_centre(&mut renderer, -50), red);
        assert_eq!(pixel_right_of_centre(&mut renderer, 50), background);

        renderer.set_beneath(Some(under.to_str().unwrap())).unwrap();
        assert_eq!(renderer.beneath_path(), under.to_str());
        assert_eq!(
            pixel_right_of_centre(&mut renderer, -50),
            red,
            "the chart wins"
        );
        assert_eq!(
            pixel_right_of_centre(&mut renderer, 50),
            blue,
            "the gap is filled"
        );

        renderer.set_beneath(None).unwrap();
        assert_eq!(pixel_right_of_centre(&mut renderer, 50), background);
        assert!(renderer.set_beneath(Some("missing.mbtiles")).is_err());
        assert_eq!(renderer.beneath_path(), None);
        drop(renderer);
        std::fs::remove_file(chart).unwrap();
        std::fs::remove_file(under).unwrap();
    }

    #[test]
    fn changing_file_keeps_the_zoom_even_past_the_new_files_levels() {
        let first = fixture_chart(&[2, 3, 4]);
        let second = fixture_chart(&[2]);
        let mut renderer = renderer_for(&first, 4);
        renderer.look_at(10.0, 20.0, (0, 0));
        renderer.change_map(second.to_str().unwrap()).unwrap();
        assert_eq!(renderer.zoom(), 4, "shown overscaled, not snapped to 2");
        assert_eq!(renderer.top_zoom(), 2);
        assert_eq!(renderer.path(), second.to_str().unwrap());
        assert_close(
            renderer.view().to_lonlat(Point::new(256, 128)).unwrap(),
            (10.0, 20.0),
        );
        drop(renderer);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn the_viewed_position_stays_centred_through_a_zoom_animation() {
        let path = fixture_chart(&[2, 3]);
        let mut renderer = renderer_for(&path, 2);
        let brest = (-4.4861, 48.3904);
        renderer.look_at(brest.0, brest.1, (0, 0));

        renderer.zoom_to(3);
        std::thread::sleep(Duration::from_millis(120)); // mid-animation
        assert!(renderer.advance_zoom_animation());
        assert_close(
            renderer.view().to_lonlat(Point::new(256, 128)).unwrap(),
            brest,
        );

        std::thread::sleep(ZOOM_ANIMATION_DURATION);
        renderer.advance_zoom_animation();
        assert_eq!(renderer.gps_to_point(brest), Point::new(256, 128));
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn changing_chart_keeps_the_viewed_position() {
        let first = fixture_chart(&[2]);
        let second = fixture_chart(&[3]);
        let mut renderer = renderer_for(&first, 2);
        renderer.look_at(10.0, 20.0, (0, 0));

        renderer.change_map(second.to_str().unwrap()).unwrap();

        assert_close(
            renderer.view().to_lonlat(Point::new(256, 128)).unwrap(),
            (10.0, 20.0),
        );
        drop(renderer);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn render_frame_draws_the_viewed_position_at_the_centre_of_the_texture() {
        let path = fixture_chart_with(&[2], Rgba([10, 20, 30, 255]));
        let mut renderer = renderer_for(&path, 2);
        renderer.look_at(0.0, 0.0, (0, 0));

        let (pixels, rect) = renderer.render_frame();
        let (width, height) = (rect.width() as usize, rect.height() as usize);
        let index = ((height / 2) * width + width / 2) * 4;
        assert_eq!(&pixels[index..index + 4], &[10, 20, 30, 255]);
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    // ----- benchmarks: cargo test --release bench_ -- --ignored --nocapture -----

    /// The real charts to benchmark against that are present (they are
    /// gitignored): path and the zoom to render at.
    fn bench_charts() -> Vec<(&'static str, i32)> {
        [
            ("map/OpenSeaMap-test.mbtiles", 8),
            ("assets/OSM.mbtiles", 6),
        ]
        .into_iter()
        .filter(|(path, _)| std::path::Path::new(path).exists())
        .collect()
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
        let start = Instant::now();
        let value = f();
        (value, start.elapsed())
    }

    #[test]
    #[ignore]
    fn bench_render_frame_cold_and_warm() {
        for (path, zoom) in bench_charts() {
            let config = RenderingConfig::default();
            let probe = MBTilesRenderer::new(path, zoom, 1600, 800, &config).unwrap();
            let (lon, lat) = probe.tiles_center().unwrap();

            let cold = median(
                (0..5)
                    .map(|_| {
                        let mut renderer =
                            MBTilesRenderer::new(path, zoom, 1600, 800, &config).unwrap();
                        renderer.look_at(lon, lat, (0, 0));
                        timed(|| renderer.render_frame()).1
                    })
                    .collect(),
            );
            let mut renderer = MBTilesRenderer::new(path, zoom, 1600, 800, &config).unwrap();
            renderer.look_at(lon, lat, (0, 0));
            renderer.render_frame();
            let warm = median(
                (0..20)
                    .map(|_| timed(|| renderer.render_frame()).1)
                    .collect(),
            );
            println!(
                "bench render 1600x800 {path} z{}: cold {cold:?}, warm {warm:?}, {} tiles cached",
                renderer.zoom,
                renderer.store.cached_count()
            );
        }
    }

    /// A sparse chart (OpenSeaMap test: tiles only near the Baltic coast)
    /// with the world chart beneath it filling every gap: the worst case for
    /// the cut-and-scale fill, cold and warm.
    #[test]
    #[ignore]
    fn bench_render_frame_with_a_file_beneath() {
        let (chart, beneath) = ("map/OpenSeaMap-test.mbtiles", "assets/OSM.mbtiles");
        if !std::path::Path::new(chart).exists() || !std::path::Path::new(beneath).exists() {
            return;
        }
        let config = RenderingConfig::default();
        // On the chart's west edge (11.25 E), so half the screen is gap.
        let (_, lat) = MBTilesRenderer::new(chart, 8, 1600, 800, &config)
            .unwrap()
            .tiles_center()
            .unwrap();
        let lon = 11.25;
        let open = || {
            let mut renderer = MBTilesRenderer::new(chart, 8, 1600, 800, &config).unwrap();
            renderer.set_beneath(Some(beneath)).unwrap();
            renderer.look_at(lon, lat, (0, 0));
            renderer
        };
        let cold = median(
            (0..5)
                .map(|_| {
                    let mut renderer = open();
                    timed(|| renderer.render_frame()).1
                })
                .collect(),
        );
        let mut renderer = open();
        renderer.render_frame();
        let warm = median(
            (0..20)
                .map(|_| timed(|| renderer.render_frame()).1)
                .collect(),
        );
        println!("bench render 1600x800 {chart} z8 over {beneath}: cold {cold:?}, warm {warm:?}");
    }

    #[test]
    #[ignore]
    fn bench_prefetch_step_of_eight_tiles() {
        for (path, zoom) in bench_charts() {
            let config = RenderingConfig::default();
            let samples = (0..5)
                .map(|_| {
                    let mut renderer =
                        MBTilesRenderer::new(path, zoom, 1600, 800, &config).unwrap();
                    let (lon, lat) = renderer.tiles_center().unwrap();
                    renderer.look_at(lon, lat, (0, 0));
                    timed(|| renderer.prefetch_step(8)).1
                })
                .collect();
            println!("bench prefetch_step(8) cold {path}: {:?}", median(samples));
        }
    }

    #[test]
    #[ignore]
    fn bench_single_tile_decode() {
        for (path, zoom) in bench_charts() {
            let renderer =
                MBTilesRenderer::new(path, zoom, 1600, 800, &RenderingConfig::default()).unwrap();
            let (lon, lat) = renderer.tiles_center().unwrap();
            let (x, y) = renderer.project(lon, lat);
            let (tx, ty) = ((x / 256.0) as i64, (y / 256.0) as i64);
            let bytes = renderer
                .store
                .fetch_tile_bytes(renderer.zoom, tx, ty)
                .unwrap()
                .unwrap();
            let fetch = median(
                (0..20)
                    .map(|_| timed(|| renderer.store.fetch_tile_bytes(renderer.zoom, tx, ty)).1)
                    .collect(),
            );
            let decode = median(
                (0..20)
                    .map(|_| timed(|| image::load_from_memory(&bytes).unwrap().to_rgba8()).1)
                    .collect(),
            );
            println!(
                "bench tile {path} z{} ({} bytes): sqlite fetch {fetch:?}, png decode {decode:?}",
                renderer.zoom,
                bytes.len()
            );
        }
    }
}
