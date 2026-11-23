use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use image::{imageops, ImageBuffer, Rgba, RgbaImage};
use lru::LruCache;
use rusqlite::{Connection, OpenFlags};
use sdl2::rect::{Point, Rect};
use std::time::{Duration, Instant};
use crate::config::RenderingConfig;

const RAYON_TERRE	:f64   = 6378137.0;
const TILE_SIZE     :u32   = 256;
const ZOOM_ANIMATION_DURATION: Duration = Duration::from_millis(240);
const ROTATION_MARGIN: u32 = 4;
const MAP_BACKGROUND: Rgba<u8> = Rgba([245, 245, 245, 255]);

/// A decoded tile. `opaque` is computed once at decode time so that
/// compositing can copy the tile instead of alpha-blending it.
struct Tile {
    image: RgbaImage,
    opaque: bool,
}

/// Cached tile: `None` means "tile fetched but could not be decoded / did not exist".
/// Shared through an `Arc` so a cache hit does not copy the image.
type CachedTile = Option<Arc<Tile>>;

type TileKey = (i32, i64, i64);

/// Fetches and decodes tiles on a worker thread, so prefetching the tiles
/// around the view never stalls the render loop. The worker has its own
/// read-only connection to the chart.
struct TileLoader {
    /// `None` only while shutting down.
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
        let (requests, request_rx) = channel::<TileKey>();
        let (result_tx, results) = channel();
        let worker = std::thread::spawn(move || {
            for key in request_rx {
                let (z, x, y) = key;
                let tile = decode_tile(fetch_tile_bytes(&conn, z, x, y).unwrap_or(None));
                if result_tx.send((key, tile)).is_err() {
                    break; // the renderer that owned us is gone
                }
            }
        });
        Ok(Self {
            requests: Some(requests),
            results,
            in_flight: HashSet::new(),
            worker: Some(worker),
        })
    }

    fn is_pending(&self, key: &TileKey) -> bool {
        self.in_flight.contains(key)
    }

    fn request(&mut self, key: TileKey) {
        let Some(requests) = &self.requests else {
            return;
        };
        if !self.in_flight.contains(&key) && requests.send(key).is_ok() {
            self.in_flight.insert(key);
        }
    }

    /// Moves every finished tile into the cache. Returns how many arrived.
    fn drain_into(&mut self, cache: &mut LruCache<TileKey, CachedTile>) -> usize {
        let mut arrived = 0;
        while let Ok((key, tile)) = self.results.try_recv() {
            self.in_flight.remove(&key);
            if !cache.contains(&key) {
                cache.put(key, tile);
            }
            arrived += 1;
        }
        arrived
    }
}

impl Drop for TileLoader {
    /// Closes the request channel so the worker finishes what is queued and
    /// exits, then waits for it, so its connection to the chart file is
    /// closed before the renderer (and possibly the file) goes away.
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn fetch_tile_bytes(conn: &Connection, z: i32, x: i64, y: i64) -> rusqlite::Result<Option<Vec<u8>>> {
    let n = 1i64 << z; // 2^z
    let tms_y = n - 1 - y; // MBTiles utilise le schema TMS

    let mut stmt = conn.prepare_cached(
        "SELECT tile_data FROM tiles WHERE zoom_level=?1 AND tile_column=?2 AND tile_row=?3",
    )?;
    let result = stmt
        .query_row((z, x, tms_y), |row| row.get::<_, Vec<u8>>(0))
        .ok();
    Ok(result)
}

fn decode_tile(bytes: Option<Vec<u8>>) -> CachedTile {
    bytes
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|dyn_img| {
            let has_alpha = dyn_img.color().has_alpha();
            let image = dyn_img.to_rgba8();
            let opaque = !has_alpha || image.chunks_exact(4).all(|pixel| pixel[3] == 255);
            Arc::new(Tile { image, opaque })
        })
}

pub struct MBTilesRenderer {
    #[allow(dead_code)]
    path: String,
    tile_size: u32,
    width: u32,
    height: u32,
    render_width: u32,
    render_height: u32,
    zoom: i32,
    display_zoom: f64,
    zoom_animation: Option<(f64, Instant)>,
    rotation: f64,
    min_zoom: i32,
    max_zoom: i32,
    metadata: HashMap<String, String>,
    #[allow(dead_code)]
    tile_format: String,
    conn: Connection,
    cache: LruCache<TileKey, CachedTile>,
    loader: TileLoader,
    rendering_config: RenderingConfig,
    default_map: String,
    pub resolution : f64,
    /// Centre (en coordonnées "pixel monde") utilisé lors du dernier rendu.
    /// Sert de référence pour convertir d'autres coordonnées GPS (waypoints,
    /// etc.) en coordonnées écran correspondant à la vue actuellement affichée.
    last_center: (f64, f64),
    available_zooms: Vec<i32>,
}

impl MBTilesRenderer {
    pub fn new(
        mbtiles_path: &str,
        zoom: i32,
        width: u32,
        height: u32,
        config: &RenderingConfig,
        default_map: &str,
    ) -> rusqlite::Result<Self> 
        {
        let (render_width, render_height) = Self::texture_dimensions(width, height);
        // Ouverture en lecture seule, equivalent de "file:...?mode=ro"
        let conn = Connection::open_with_flags(
            mbtiles_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;

        let mut renderer = Self {
            path: mbtiles_path.to_string(),
            tile_size : TILE_SIZE,
            width,
            height,
            render_width,
            render_height,
            zoom, // valeur temporaire, ecrasee juste apres par _clamp_zoom
            display_zoom: zoom as f64,
            zoom_animation: None,
            rotation: 0.0,
            min_zoom: 0,
            max_zoom: 18,
            metadata: HashMap::new(),
            tile_format: "png".to_string(),
            conn,
            cache: LruCache::new(NonZeroUsize::new(config.tile_cache_size.max(1)).unwrap()),
            loader: TileLoader::start(mbtiles_path)?,
            rendering_config: config.clone(),
            default_map: default_map.to_string(),
            resolution: 1.0,
            last_center: (0.0, 0.0),
            available_zooms: Vec::new(),
        };

        renderer.load_metadata()?;
        renderer.zoom = renderer.nearest_zoom( zoom)
                                .unwrap_or_else(|| renderer.clamp_zoom(zoom));
        renderer.display_zoom = renderer.zoom as f64;
        println!("nearest zoom: {}",renderer.zoom);

        Ok(renderer)
        }

    pub fn texture_dimensions(width: u32, height: u32) -> (u32, u32) {
        let side = ((width as f64).hypot(height as f64)).ceil() as u32 + ROTATION_MARGIN;
        let scaled_side = side * 2 + ROTATION_MARGIN;
        (scaled_side, scaled_side)
    }

    pub fn rotation_degrees(&self) -> f64 {
        -self.rotation.to_degrees()
    }

    pub fn render_rect(&self) -> Rect {
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

    pub fn texture_rect(&self) -> Rect {
        let display_rect = self.render_rect();
        let zoom_scale = 2.0_f64.powf(self.display_zoom - self.source_zoom() as f64);
        let (width, height) = source_dimensions(display_rect.width(), display_rect.height(), zoom_scale);
        Rect::new(
            (self.render_width - width) as i32 / 2,
            (self.render_height - height) as i32 / 2,
            width,
            height,
        )
    }

    pub fn display_dimensions(&self) -> (u32, u32) {
        let rect = self.render_rect();
        (rect.width(), rect.height())
    }

    /// Collects tiles the loader has finished and asks it for up to `budget`
    /// missing tiles around the view. Never decodes on the calling thread.
    /// Returns true while tiles are still missing or being loaded.
    pub fn prefetch_step(&mut self, budget: usize) -> bool {
        self.loader.drain_into(&mut self.cache);
        let source_zoom = self.source_zoom();
        let n_tiles: i64 = 1i64 << source_zoom;
        let tile_size = self.tile_size as f64;
        let zoom_scale = 2.0_f64.powf(self.display_zoom - source_zoom as f64);
        let source_center_x = self.last_center.0 / zoom_scale;
        let source_center_y = self.last_center.1 / zoom_scale;
        let (view_width, view_height) = rotated_bounds(self.width, self.height, self.rotation);
        let origin_x = source_center_x - view_width as f64 / zoom_scale / 2.0;
        let origin_y = source_center_y - view_height as f64 / zoom_scale / 2.0;

        let first_tx = (origin_x / tile_size).floor() as i64 - self.rendering_config.prefetch_margin;
        let first_ty = (origin_y / tile_size).floor() as i64 - self.rendering_config.prefetch_margin;
        let last_tx = ((origin_x + view_width as f64 / zoom_scale) / tile_size).floor() as i64 + self.rendering_config.prefetch_margin;
        let last_ty = ((origin_y + view_height as f64 / zoom_scale) / tile_size).floor() as i64 + self.rendering_config.prefetch_margin;

        let mut requested = 0;
        for tx in first_tx..=last_tx
            {
            for ty in first_ty..=last_ty
                {
                if ty < 0 || ty >= n_tiles
                    {
                    continue;
                    }
                let key = (source_zoom, tx.rem_euclid(n_tiles), ty);
                if self.cache.contains(&key) || self.loader.is_pending(&key)
                    {
                    continue;
                    }
                if requested >= budget
                    {
                    return true; // il en reste
                    }
                self.loader.request(key);
                requested += 1;
                }
            }
        !self.loader.in_flight.is_empty()
        }

    fn nearest_zoom(&self, target: i32) -> Option<i32> 
        {
        self.available_zooms
            .iter()
            .copied()
            .min_by_key(|&z| (z - target).abs())
        }

    fn source_zoom(&self) -> i32 {
        source_zoom_for(self.display_zoom, &self.available_zooms, self.zoom)
    }

    fn set_zoom(&mut self, zoom: i32) {
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

    pub fn advance_zoom_animation(&mut self) -> bool {
        let Some((start_zoom, started_at)) = self.zoom_animation else {
            return false;
        };

        let progress = (started_at.elapsed().as_secs_f64()
            / ZOOM_ANIMATION_DURATION.as_secs_f64())
        .min(1.0);
        let eased = smooth_zoom_progress(progress);
        let next_zoom = start_zoom + (self.zoom as f64 - start_zoom) * eased;
        let scale = 2.0_f64.powf(next_zoom - self.display_zoom);
        self.last_center.0 *= scale;
        self.last_center.1 *= scale;
        self.display_zoom = next_zoom;

        if progress >= 1.0 {
            self.display_zoom = self.zoom as f64;
            self.zoom_animation = None;
        }
        true
    }

    pub fn set_rotation(&mut self, radians: f64) {
        self.rotation = if radians.is_finite() {
            radians.rem_euclid(std::f64::consts::TAU)
        } else {
            0.0
        };
    }

    pub fn course_to_screen(&self, course: f64) -> f64 {
        course_to_screen(course, self.rotation)
    }
        
    /// Moves to the next available zoom level. At the chart's maximum zoom
    /// nothing changes: the tile cache and view are kept as they are.
    pub fn zoom_increase(&mut self) -> i32
        {
        if let Some(i) = self.available_zooms.iter().position(|&z| z == self.zoom)
            {
            if let Some(&next) = self.available_zooms.get(i + 1)
                {
                self.set_zoom(next);
                }
            }
        self.zoom
        }
    
    pub fn zoom_decrease(&mut self) -> i32
        {
        if let Some(i) = self.available_zooms.iter().position(|&z| z == self.zoom) 
            {
            if let Some(&prev) = i.checked_sub(1).and_then(|j| self.available_zooms.get(j)) 
                {
                self.set_zoom(prev);
                }
            else{
                println!("zomm mini");
                let fallback_map = self.default_map.clone();
                let _ = self.change_map(&fallback_map);
                }
            }
        println!("zomm : {}",self.zoom);
        self.zoom
        }
        
    pub fn change_map(&mut self, mbtiles_path: &str) -> rusqlite::Result<()> {
        // Position GPS actuellement affichée au centre de l'écran
        let (lon, lat) = self.pixel_to_lonlat(self.last_center.0, self.last_center.1);

        // On construit le nouveau renderer d'abord : si ça échoue, self n'est pas touché.
        // Le cache est neuf (donc vide), les métadonnées et les zooms sont rechargés,
        // et le zoom courant est ramené dans [min_zoom, max_zoom] de la nouvelle carte.
        let mut new_renderer = Self::new(
            mbtiles_path,
            self.zoom,
            self.width,
            self.height,
            &self.rendering_config,
            &self.default_map,
        )?;

        // On garde un last_center cohérent avec le (possible) nouveau zoom
        new_renderer.last_center = new_renderer.project(lon, lat);

        *self = new_renderer;
        Ok(())
    }
    
    /// Position to show when no position has been saved yet. Prefers the
    /// centre of the tiles actually present, then the chart metadata, and
    /// finally (0, 0) so that a sparse or empty chart never stops startup.
    pub fn initial_center(&self) -> (f64, f64) {
        if let Some(center) = self.tiles_center() {
            return center;
        }
        match center_from_metadata(&self.metadata) {
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

/// Centre calculé à partir des tuiles réellement présentes au zoom min.
/// Gère les cartes qui chevauchent l'antiméridien (Pacifique).
    pub fn tiles_center(&self) -> Option<(f64, f64)> {
        let z = self.min_zoom;
        let n = (1i64 << z) as f64;
        let ni = 1i64 << z;

        // Colonnes occupées (triées)
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT tile_column FROM tiles WHERE zoom_level=?1 ORDER BY tile_column")
            .ok()?;
        let cols: Vec<i64> = stmt
            .query_map([z], |r| r.get::<_, i64>(0))
            .ok()?
            .flatten()
            .collect();
        if cols.is_empty() {
            return None;
        }

        // Plus grand trou circulaire entre deux colonnes consécutives
        let mut best_gap = -1i64;
        let mut start = cols[0]; // première colonne après le trou
        for i in 0..cols.len() {
            let cur = cols[i];
            let next = if i + 1 < cols.len() { cols[i + 1] } else { cols[0] + ni };
            let gap = next - cur - 1;
            if gap > best_gap {
                best_gap = gap;
                start = next % ni;
            }
        }

        // Étendue couverte (en tuiles) = tout sauf le trou
        let span = (ni - best_gap) as f64;
        let center_col = (start as f64 + span / 2.0) % n;
        let mut lon = center_col / n * 360.0 - 180.0;
        if lon >= 180.0 {
            lon -= 360.0;
        }

        // Latitude : lignes min/max (TMS -> XYZ)
        let (rmin, rmax): (i64, i64) = self
            .conn
            .query_row(
                "SELECT MIN(tile_row), MAX(tile_row) FROM tiles WHERE zoom_level=?1",
                [z],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        let y_top = (ni - 1 - rmax) as f64;       // bord nord
        let y_bottom = (ni - 1 - rmin) as f64 + 1.0; // bord sud
        let y_mid = (y_top + y_bottom) / 2.0;

        let merc = std::f64::consts::PI * (1.0 - 2.0 * y_mid / n);
        let lat = merc.sinh().atan().to_degrees();
        Some((lon, lat))
    } 


    pub fn get_raw_rgba(
        &mut self,
        lon: f64,
        lat: f64,
        offset: (i32, i32),
    ) -> (Vec<u8>, Rect) {
        let (mut center_x, mut center_y) = self.lonlat_to_pixel(lon, lat);
        let (offset_x, offset_y) = screen_to_world_delta(
            offset.0 as f64,
            offset.1 as f64,
            self.rotation,
        );
        center_x -= offset_x;
        center_y -= offset_y;
        self.last_center = (center_x, center_y); // mémorisé pour gps_to_screen()
        let (image, rect) = self.render_view(center_x, center_y);
        (image.into_raw(), rect)
    }

    pub fn center_with_offset(&self, center: (f64, f64), offset: (i32, i32)) -> (f64, f64) {
        let (center_x, center_y) = self.project(center.0, center.1);
        let (offset_x, offset_y) =
            screen_to_world_delta(offset.0 as f64, offset.1 as f64, self.rotation);
        self.pixel_to_lonlat(
            center_x - offset_x,
            center_y - offset_y,
        )
    }

    pub fn gps_to_point(&self, pos: (f64,f64)) -> Point
        {
        let (world_x, world_y) = self.project(pos.0, pos.1);
        let (x, y) =
            self.world_to_screen(world_x, world_y, self.last_center.0, self.last_center.1);
        Point::new(x as i32, y as i32)
        }
    
    pub fn screen_to_gps(&self, dx:i32, dy:i32) -> (f64,f64) {
        let (offset_x, offset_y) =
            screen_to_world_delta(dx as f64, dy as f64, self.rotation);
        let (x, y) = (self.last_center.0, self.last_center.1);
        let (new_lon, new_lat) = self.pixel_to_lonlat(x - offset_x, y - offset_y);
        (new_lon,new_lat)
        }

    fn world_to_screen(&self, world_x: f64, world_y: f64, center_x: f64, center_y: f64) -> (i64, i64) {
        let (x, y) = world_to_screen_delta(
            world_x - center_x,
            world_y - center_y,
            self.rotation,
        );
        (
            (x + self.width as f64 / 2.0).round() as i64,
            (y + self.height as f64 / 2.0).round() as i64,
        )
    }

    fn lonlat_to_pixel(&mut self, lon: f64, lat: f64) -> (f64, f64) {
        let (x, y) = self.project(lon, lat);

        let n = 2.0_f64.powf(self.display_zoom);
        let tile_size = self.tile_size as f64;
        let lat_rad = lat.to_radians();
        self.resolution = (lat_rad.cos() * 2.0 * std::f64::consts::PI * RAYON_TERRE)
                            / (tile_size * n);

        (x, y)
    }

    fn pixel_to_lonlat(&self, x: f64, y: f64) -> (f64, f64) {
        let scale = TILE_SIZE as f64 * 2f64.powf(self.display_zoom);

        let lon = x / scale * 360.0 - 180.0;

        let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * y / scale;
        let lat = n.sinh().atan().to_degrees();

        (lon, lat)
    }

    fn load_metadata(&mut self) -> rusqlite::Result<()> {
        // Lecture de la table metadata (peut ne pas exister -> on ignore l'erreur)
        let mut meta = HashMap::new();
        if let Ok(mut stmt) = self.conn.prepare("SELECT name, value FROM metadata") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for r in rows.flatten() {
                    meta.insert(r.0, r.1);
                }
            }
        }

        self.tile_format = meta.get("format").cloned().unwrap_or_else(|| "png".to_string());

        let parsed_min = meta.get("minzoom").and_then(|v| v.parse::<i32>().ok());
        let parsed_max = meta.get("maxzoom").and_then(|v| v.parse::<i32>().ok());

        match (parsed_min, parsed_max) {
            (Some(min), Some(max)) => {
                self.min_zoom = min;
                self.max_zoom = max;
            }
            _ => {
                let (min_row, max_row): (Option<i64>, Option<i64>) = self.conn.query_row(
                    "SELECT MIN(zoom_level), MAX(zoom_level) FROM tiles",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                self.min_zoom = min_row.unwrap_or(0) as i32;
                self.max_zoom = max_row.unwrap_or(18) as i32;
            }
        }

        self.metadata = meta;
        let mut zooms: Vec<i32> = Vec::new();
            if let Ok(mut stmt) = self
                .conn
                .prepare("SELECT DISTINCT zoom_level FROM tiles ORDER BY zoom_level")
            {
                if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i32>(0)) {
                    zooms = rows.flatten().collect();
                }
            }
            self.available_zooms = zooms;
        Ok(())
    }

    fn fetch_tile_bytes(&self, z: i32, x: i64, y: i64) -> rusqlite::Result<Option<Vec<u8>>> {
        fetch_tile_bytes(&self.conn, z, x, y)
    }

    /// The tile from the cache, or decoded now on this thread when a frame
    /// needs it and the loader has not delivered it yet.
    fn get_tile_image(&mut self, z: i32, x: i64, y: i64) -> CachedTile {
        let key = (z, x, y);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }

        let tile = decode_tile(self.fetch_tile_bytes(z, x, y).unwrap_or(None));
        self.cache.put(key, tile.clone());
        tile
    }

    fn clamp_zoom(&self, zoom: i32) -> i32 {
        zoom.clamp(self.min_zoom, self.max_zoom)
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
        let tile_size = self.tile_size as f64;

        let origin_x = center_x - width as f64 / 2.0;
        let origin_y = center_y - height as f64 / 2.0;

        let mut canvas: RgbaImage =
            ImageBuffer::from_pixel(width, height, MAP_BACKGROUND);

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

                if let Some(tile) = self.get_tile_image(zoom, wrapped_x, ty) {
                    let px = (tx as f64 * tile_size - origin_x).round() as i64;
                    let py = (ty as f64 * tile_size - origin_y).round() as i64;
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

    fn project(&self, lon: f64, lat: f64) -> (f64, f64) 
        {
        let (x, y) = project_norm(lon, lat);
        let scale = self.tile_size as f64 * 2.0_f64.powf(self.display_zoom);
        (x * scale, y * scale)
        }
    
    pub fn norm_to_point(&self, nx: f64, ny: f64) -> Point 
        {
        let scale = self.tile_size as f64 * 2.0_f64.powf(self.display_zoom);
        let (x, y) = self.world_to_screen(
            nx * scale,
            ny * scale,
            self.last_center.0,
            self.last_center.1,
        );
        Point::new(
            x as i32,
            y as i32,
            )
        }
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

fn rotated_bounds(width: u32, height: u32, angle: f64) -> (u32, u32) {
    let (sin, cos) = angle.sin_cos();
    let ceil_bound = |value: f64| (value - 1e-9).ceil().max(1.0) as u32;
    (
        ceil_bound(width as f64 * cos.abs() + height as f64 * sin.abs()),
        ceil_bound(width as f64 * sin.abs() + height as f64 * cos.abs()),
    )
}

fn source_zoom_for(display_zoom: f64, available_zooms: &[i32], fallback: i32) -> i32 {
    let lower_zoom = available_zooms
        .iter()
        .copied()
        .filter(|&zoom| zoom as f64 <= display_zoom)
        .max();
    let nearest_zoom = available_zooms.iter().copied().min_by(|a, b| {
        (*a as f64 - display_zoom)
            .abs()
            .total_cmp(&(*b as f64 - display_zoom).abs())
    });

    match (nearest_zoom, lower_zoom) {
        (Some(nearest), Some(lower)) if nearest as f64 > display_zoom + 1.0 => lower,
        (Some(nearest), _) => nearest,
        (None, _) => fallback,
    }
}

fn source_dimensions(width: u32, height: u32, zoom_scale: f64) -> (u32, u32) {
    if (zoom_scale - 1.0).abs() < f64::EPSILON {
        return (width, height);
    }
    (
        (width as f64 / zoom_scale).ceil() as u32 + 2,
        (height as f64 / zoom_scale).ceil() as u32 + 2,
    )
}

fn smooth_zoom_progress(progress: f64) -> f64 {
    progress * progress * (3.0 - 2.0 * progress)
}

fn course_to_screen(course: f64, rotation: f64) -> f64 {
    (course - rotation).rem_euclid(std::f64::consts::TAU)
}

fn world_to_screen_delta(dx: f64, dy: f64, angle: f64) -> (f64, f64) {
    let (sin, cos) = angle.sin_cos();
    (dx * cos + dy * sin, -dx * sin + dy * cos)
}

fn screen_to_world_delta(dx: f64, dy: f64, angle: f64) -> (f64, f64) {
    let (sin, cos) = angle.sin_cos();
    (dx * cos - dy * sin, dx * sin + dy * cos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_up_rotates_eastward_course_to_screen_top() {
        let (x, y) = world_to_screen_delta(1.0, 0.0, std::f64::consts::FRAC_PI_2);
        assert!(x.abs() < 1e-12);
        assert!((y + 1.0).abs() < 1e-12);
    }

    #[test]
    fn screen_delta_rotation_round_trips() {
        let original = (125.0, -42.0);
        let angle = 1.37;
        let screen = world_to_screen_delta(original.0, original.1, angle);
        let world = screen_to_world_delta(screen.0, screen.1, angle);

        assert!((world.0 - original.0).abs() < 1e-10);
        assert!((world.1 - original.1).abs() < 1e-10);
    }

    #[test]
    fn rotated_view_bounds_cover_swapped_quarter_turn() {
        let (width, height) = rotated_bounds(1600, 800, std::f64::consts::FRAC_PI_2);
        assert_eq!((width, height), (800, 1600));
    }

    #[test]
    fn texture_dimensions_cover_full_screen_after_rotation() {
        let (width, height) = MBTilesRenderer::texture_dimensions(1600, 800);
        assert_eq!(width, height);
        assert!(width >= 2 * 1600.0_f64.hypot(800.0) as u32);
    }

    #[test]
    fn texture_source_dimensions_follow_zoom_scale() {
        assert_eq!(source_dimensions(1600, 800, 1.0), (1600, 800));
        assert_eq!(source_dimensions(1600, 800, 2.0), (802, 402));
        assert_eq!(source_dimensions(1600, 800, 0.5), (3202, 1602));
    }

    #[test]
    fn course_arrow_points_up_when_course_up_is_enabled() {
        let screen_course = course_to_screen(
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        );
        assert!(screen_course.abs() < 1e-12);
    }

    #[test]
    fn zoom_transition_uses_smoothstep_endpoints() {
        assert_eq!(smooth_zoom_progress(0.0), 0.0);
        assert_eq!(smooth_zoom_progress(0.5), 0.5);
        assert_eq!(smooth_zoom_progress(1.0), 1.0);
    }

    #[test]
    fn source_zoom_avoids_extreme_downscaling_for_sparse_charts() {
        assert_eq!(source_zoom_for(12.0, &[4, 18], 12), 4);
    }

    /// Writes a small raster MBTiles file holding every tile of the given
    /// zoom levels, each a solid-colour 256x256 PNG.
    fn fixture_chart(zooms: &[i32]) -> std::path::PathBuf {
        fixture_chart_with(zooms, Rgba([0, 120, 200, 255]))
    }

    fn fixture_chart_with(zooms: &[i32], colour: Rgba<u8>) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "plotter-fixture-{}-{unique}.mbtiles",
            std::process::id()
        ));
        let mut png = Vec::new();
        RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, colour)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageOutputFormat::Png)
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
                    conn.execute(
                        "INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)",
                        (z, x, y, &png),
                    )
                    .unwrap();
                }
            }
        }
        path
    }

    /// Renders a 512x256 view centred on (0, 0) and returns the centre pixel.
    fn centre_pixel_of(path: &std::path::Path) -> Rgba<u8> {
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 2, 512, 256, &RenderingConfig::default(), path_str)
                .unwrap();
        let (pixels, rect) = renderer.get_raw_rgba(0.0, 0.0, (0, 0));
        let (width, height) = (rect.width() as usize, rect.height() as usize);
        let index = ((height / 2) * width + width / 2) * 4;
        Rgba([pixels[index], pixels[index + 1], pixels[index + 2], pixels[index + 3]])
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

    #[test]
    fn initial_center_falls_back_when_the_chart_has_no_tiles() {
        let path = fixture_chart(&[]);
        let path_str = path.to_str().unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO metadata VALUES ('bounds', '-10,50,0,60')",
            [],
        )
        .unwrap();
        drop(conn);

        let renderer =
            MBTilesRenderer::new(path_str, 8, 512, 256, &RenderingConfig::default(), path_str)
                .unwrap();
        assert_eq!(renderer.tiles_center(), None);
        assert_eq!(renderer.initial_center(), (-5.0, 55.0));
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    // ----- benchmarks: cargo test --release bench_ -- --ignored --nocapture -----

    /// The real charts to benchmark against that are present (they are
    /// gitignored): path and the zoom to render at.
    fn bench_charts() -> Vec<(&'static str, i32)> {
        [("map/OpenSeaMap-test.mbtiles", 8), ("assets/OSM.mbtiles", 6)]
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
        let probe = MBTilesRenderer::new(path, zoom, 1600, 800, &config, path).unwrap();
        let (lon, lat) = probe.tiles_center().unwrap();

        let cold = median(
            (0..5)
                .map(|_| {
                    let mut renderer =
                        MBTilesRenderer::new(path, zoom, 1600, 800, &config, path).unwrap();
                    timed(|| renderer.get_raw_rgba(lon, lat, (0, 0))).1
                })
                .collect(),
        );
        let mut renderer = MBTilesRenderer::new(path, zoom, 1600, 800, &config, path).unwrap();
        renderer.get_raw_rgba(lon, lat, (0, 0));
        let warm = median(
            (0..20)
                .map(|_| timed(|| renderer.get_raw_rgba(lon, lat, (0, 0))).1)
                .collect(),
        );
        println!(
            "bench render 1600x800 {path} z{}: cold {cold:?}, warm {warm:?}, {} tiles cached",
            renderer.zoom,
            renderer.cache.len()
        );
        }
    }

    #[test]
    #[ignore]
    fn bench_prefetch_step_of_eight_tiles() {
        for (path, zoom) in bench_charts() {
        let config = RenderingConfig::default();
        let samples = (0..5)
            .map(|_| {
                let mut renderer =
                    MBTilesRenderer::new(path, zoom, 1600, 800, &config, path).unwrap();
                let (lon, lat) = renderer.tiles_center().unwrap();
                renderer.last_center = renderer.project(lon, lat);
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
            MBTilesRenderer::new(path, zoom, 1600, 800, &RenderingConfig::default(), path)
                .unwrap();
        let (lon, lat) = renderer.tiles_center().unwrap();
        let (x, y) = renderer.project(lon, lat);
        let (tx, ty) = ((x / 256.0) as i64, (y / 256.0) as i64);
        let bytes = renderer.fetch_tile_bytes(renderer.zoom, tx, ty).unwrap().unwrap();
        let fetch = median(
            (0..20)
                .map(|_| timed(|| renderer.fetch_tile_bytes(renderer.zoom, tx, ty)).1)
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

    #[test]
    fn prefetch_loads_tiles_in_the_background_without_blocking_the_caller() {
        let path = fixture_chart(&[3]); // 64 tiles, all in view plus margin at 512x256
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 3, 512, 256, &RenderingConfig::default(), path_str)
                .unwrap();
        renderer.last_center = renderer.project(0.0, 0.0);
        assert_eq!(renderer.cache.len(), 0);

        let started = Instant::now();
        renderer.prefetch_step(64);
        let first_call = started.elapsed();

        let deadline = Instant::now() + Duration::from_secs(15);
        while renderer.cache.len() < 20 {
            assert!(Instant::now() < deadline, "only {} tiles arrived", renderer.cache.len());
            std::thread::sleep(Duration::from_millis(10));
            renderer.prefetch_step(64);
        }
        // Decoding twenty or more PNG tiles synchronously takes far longer
        // than this, even in release; the caller must only enqueue work.
        assert!(first_call < Duration::from_millis(50), "prefetch_step took {first_call:?}");
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn zooming_in_at_the_maximum_zoom_keeps_the_tile_cache() {
        let path = fixture_chart(&[2, 3]);
        let path_str = path.to_str().unwrap();
        let mut renderer =
            MBTilesRenderer::new(path_str, 3, 512, 256, &RenderingConfig::default(), path_str)
                .unwrap();
        assert_eq!(renderer.zoom, 3);
        renderer.last_center = renderer.project(0.0, 0.0);
        while renderer.prefetch_step(64) {}
        let cached_before = renderer.cache.len();
        assert!(cached_before > 0);

        assert_eq!(renderer.zoom_increase(), 3);

        assert_eq!(renderer.cache.len(), cached_before);
        assert!(renderer.zoom_animation.is_none());
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }
}

pub fn project_norm(lon: f64, lat: f64) -> (f64, f64) 
    {
    let lat_rad = lat.to_radians();
    let x = (lon + 180.0) / 360.0;
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0;
    (x, y)
    }
