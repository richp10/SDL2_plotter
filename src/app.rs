use crate::boat::BoatState;
use crate::catalogue::Catalogue;
use crate::{
    buttombar,
    config::{AppConfig, InputSource},
    mbtiles_renderer, route, screen, signalk, state, topbar, track, waypoints,
};
use mbtiles_renderer::MBTilesRenderer;
use sdl2::event::Event;
use sdl2::render::{Texture, TextureCreator};
use sdl2::ttf::Sdl2TtfContext;
use sdl2::video::WindowContext;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod actions;
mod ais_info;
mod centre_button;
mod chart_manager;
mod confirm_dialog;
mod gesture;
mod input;
mod photo_button;
mod render;
mod session;
mod tools;

pub(crate) use crate::action::AppAction;

use confirm_dialog::ConfirmAction;
use gesture::GestureRecognizer;
use session::Session;
use tools::Target;

/// What the user decided about each chart file; see `catalogue`.
const LONG_CLICK: Duration = Duration::from_millis(500);
const MOVE_TOLERANCE: i32 = 5;
const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(10);

struct PendingWaypoint {
    position: (f64, f64),
    name: String,
}

#[derive(Debug)]
enum Mouse {
    Hand,
    Cross,
    Arrow,
    Move,
}

pub(crate) struct Plotter<'ttf> {
    /// Everything the actions act on; SDL-free and tested on its own.
    session: Session,
    dirty: bool,
    /// Press/motion/release become Click, LongPress and Drag gestures here.
    gestures: GestureRecognizer,
    /// What a drag in progress grabbed, decided once at its start.
    grab: Option<Target>,
    /// When the chart was last redrawn during the drag under way.
    drag_redrawn: Instant,
    /// The size the window changed to, applied before the next frame.
    resized: Option<(u32, u32)>,
    /// Where the state is saved when the loop ends.
    state_file: PathBuf,
    /// SDL reported that the render device or its targets were reset.
    device_reset: bool,
    /// Errors from each step of the main loop, logged without flooding the
    /// log. None of them ends the loop.
    render_errors: crate::diagnostics::Repeated,
    capture_errors: crate::diagnostics::Repeated,
    event_errors: crate::diagnostics::Repeated,
    resize_errors: crate::diagnostics::Repeated,
    topbar: topbar::Topbar<'ttf>,
    buttombar: buttombar::Buttombar<'ttf>,
    confirm_dialog: confirm_dialog::ConfirmDialog<'ttf>,
    chart_manager: chart_manager::ChartManager<'ttf>,
    /// The photo button and the thumbnails it shows.
    photo_button: photo_button::PhotoButtonView<'ttf>,
    /// The button that centres the chart on the boat.
    centre_button: centre_button::CentreButtonView,
    /// The details window of the AIS target tapped.
    ais_info: ais_info::AisInfoView<'ttf>,
    thumbnails: mbtiles_renderer::Thumbnails,
    pending_waypoint: Option<PendingWaypoint>,
    text_input: sdl2::keyboard::TextInputUtil,
    mouse: Mouse,
    /// The live-data source's channels.
    live: crate::feed::LiveReceivers,
    /// The source's boat or AIS channel closed and that was logged.
    boat_closed: bool,
    ais_closed: bool,
    streamer: Option<crate::stream::Streamer>,
}

/// Every file the plotter reads and writes, apart from the config and the
/// charts. `Files::default()` is the app's own layout under the working
/// directory; a test puts them all in a temp directory.
pub(crate) struct Files {
    pub state: PathBuf,
    pub track: PathBuf,
    pub route: PathBuf,
    pub waypoints: PathBuf,
    /// The catalogue's cache and the chart manager's decisions.
    pub chart_settings: PathBuf,
    /// Where GPX exports are written.
    pub export: PathBuf,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            state: "default/state.json".into(),
            track: "default/track.json".into(),
            route: "default/route.json".into(),
            waypoints: "default/waypoints.json".into(),
            chart_settings: "settings/charts.json".into(),
            export: "export".into(),
        }
    }
}

/// Counters for the loop statistics logged every `DIAGNOSTICS_INTERVAL`.
struct LoopStats {
    last_diagnostics: Instant,
    loop_count: u64,
    event_count: u64,
    track_updates: u64,
    rendered_frames: u64,
    last_input: Instant,
}

impl LoopStats {
    fn new() -> Self {
        Self {
            last_diagnostics: Instant::now(),
            loop_count: 0,
            event_count: 0,
            track_updates: 0,
            rendered_frames: 0,
            last_input: Instant::now(),
        }
    }
}

/// The streaming texture the chart is drawn into, for a window of
/// `width` x `height` (room for any rotation).
pub(crate) fn chart_texture(
    texture_creator: &TextureCreator<WindowContext>,
    width: u32,
    height: u32,
) -> Result<Texture, String> {
    let (texture_width, texture_height) = MBTilesRenderer::texture_dimensions(width, height);
    let mut texture = texture_creator
        .create_texture_streaming(
            sdl2::pixels::PixelFormatEnum::RGBA32,
            texture_width,
            texture_height,
        )
        .map_err(|error| error.to_string())?;
    texture.set_scale_mode(sdl2::render::ScaleMode::Linear);
    Ok(texture)
}

impl<'ttf> Plotter<'ttf> {
    pub(crate) fn new(
        sdl_context: &sdl2::Sdl,
        ttf_context: &'ttf Sdl2TtfContext,
        width: u32,
        height: u32,
        config: &AppConfig,
        streamer: Option<crate::stream::Streamer>,
        files: Files,
    ) -> anyhow::Result<Self> {
        let text_input = sdl_context
            .video()
            .map_err(anyhow::Error::msg)?
            .text_input();
        let (feed, live) = crate::feed::channels();
        crate::diagnostics::record(
            "INFO",
            format!("Source des données bateau: {:?}", config.input.source),
        );
        // A bad input setting costs the live data, never the start.
        match (&config.input.source, config.input_error()) {
            (_, Some(error)) => {
                tokio::spawn(crate::feed::idle(feed, error));
            }
            (InputSource::Signalk, None) => {
                tokio::spawn(signalk::run(config.signalk.clone(), feed));
            }
            (InputSource::Nmea0183, None) => {
                tokio::spawn(crate::nmea0183::run(config.nmea0183.clone(), feed));
            }
            // input_error reports these; this arm only keeps the match whole.
            (source @ (InputSource::Nmea2k | InputSource::Unknown(_)), None) => {
                tokio::spawn(crate::feed::idle(
                    feed,
                    format!("Source non prise en charge: {source:?}"),
                ));
            }
        }

        let catalogue = Catalogue::load(Path::new(&config.charts.directory), &files.chart_settings);
        for line in catalogue.describe() {
            crate::diagnostics::record("INFO", line);
        }
        let state = state::State::new(&files.state, config.charts.default.clone())
            .map_err(anyhow::Error::msg)?;
        let renderer =
            MBTilesRenderer::new(&state.map, state.zoom, width, height, &config.rendering)?;

        let topbar =
            topbar::Topbar::new(ttf_context, width, &config.fonts).map_err(anyhow::Error::msg)?;
        let buttombar = buttombar::Buttombar::new(ttf_context, width, height, &config.fonts)
            .expect("failed to build buttombar bar");
        let confirm_dialog =
            confirm_dialog::ConfirmDialog::new(ttf_context, width, height, &config.fonts)
                .map_err(anyhow::Error::msg)?;
        let chart_manager =
            chart_manager::ChartManager::new(ttf_context, width, height, &config.fonts)
                .map_err(anyhow::Error::msg)?;
        let photo_button = photo_button::PhotoButtonView::new(ttf_context, height, &config.fonts)
            .map_err(anyhow::Error::msg)?;
        let ais_info = ais_info::AisInfoView::new(ttf_context, height, &config.fonts)
            .map_err(anyhow::Error::msg)?;
        let route = route::Route::load(&files.route);
        let track = track::Track::with_checkpoint_interval(
            &files.track,
            Duration::from_secs(config.track.checkpoint_interval_seconds),
        );
        crate::diagnostics::record(
            "INFO",
            format!(
                "Trace initialisée: {} points dans {}",
                track.point_count(),
                files.track.display()
            ),
        );
        let waypoints = waypoints::Waypoints::load(&files.waypoints);

        let session = Session::new(
            renderer,
            state,
            BoatState::default(),
            route,
            track,
            waypoints,
            catalogue,
            config.charts.default.clone(),
            files.export.clone(),
        );

        Ok(Self {
            session,
            dirty: true,
            gestures: GestureRecognizer::new(LONG_CLICK, MOVE_TOLERANCE),
            grab: None,
            drag_redrawn: Instant::now(),
            resized: None,
            state_file: files.state,
            device_reset: false,
            render_errors: crate::diagnostics::Repeated::default(),
            capture_errors: crate::diagnostics::Repeated::default(),
            event_errors: crate::diagnostics::Repeated::default(),
            resize_errors: crate::diagnostics::Repeated::default(),
            topbar,
            buttombar,
            confirm_dialog,
            chart_manager,
            photo_button,
            centre_button: centre_button::CentreButtonView::new(width, height),
            ais_info,
            thumbnails: mbtiles_renderer::Thumbnails::new(),
            pending_waypoint: None,
            text_input,
            mouse: Mouse::Hand,
            live,
            boat_closed: false,
            ais_closed: false,
            streamer,
        })
    }

    /// The window was resized or maximised: everything that holds its size
    /// follows, and the chart texture is made again for the new size.
    fn resize(
        &mut self,
        screen: &mut screen::Screen<'_>,
        texture_creator: &TextureCreator<WindowContext>,
        texture: &mut Texture,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        *texture = chart_texture(texture_creator, width, height).map_err(anyhow::Error::msg)?;
        screen.resize(width, height);
        self.session.renderer.resize(width, height);
        self.session.sync_view();
        self.topbar.resize(width);
        self.buttombar.resize(width, height);
        self.confirm_dialog.resize(width, height);
        self.chart_manager.resize(width, height);
        self.photo_button.resize(height);
        self.centre_button.resize(width, height);
        self.ais_info.resize(height);
        self.dirty = true;
        crate::diagnostics::record("INFO", format!("Fenêtre redimensionnée: {width}x{height}"));
        Ok(())
    }

    /// One iteration of the main loop, apart from the sleep: events, live
    /// data, the frame, the cursor and tile prefetch. No step in it ends
    /// the loop; see `main_loop`.
    fn iteration(
        &mut self,
        stats: &mut LoopStats,
        event_pump: &mut sdl2::EventPump,
        screen: &mut screen::Screen<'_>,
        texture_creator: &TextureCreator<WindowContext>,
        texture: &mut Texture,
    ) {
        if stats.last_diagnostics.elapsed() >= DIAGNOSTICS_INTERVAL {
            crate::diagnostics::record(
					"INFO",
					format!(
						"État boucle: iterations={}, événements={}, points ajoutés={}, images rendues={}, points trace={}, sauvegarde en attente={}, dernier événement utilisateur il y a {:?}",
						stats.loop_count,
						stats.event_count,
						stats.track_updates,
						stats.rendered_frames,
						self.session.track.point_count(),
						self.session.track.save_pending(),
						stats.last_input.elapsed()
					),
				);
            stats.last_diagnostics = Instant::now();
        }

        /////////////////////   Event /////////////////////////
        // Input converts clicks with the geometry the renderer holds;
        // make sure it reflects the current state, not the last frame.
        self.session.sync_view();
        for event in event_pump.poll_iter() {
            stats.event_count += 1;
            if matches!(
                &event,
                Event::KeyDown { .. }
                    | Event::TextInput { .. }
                    | Event::MouseButtonDown { .. }
                    | Event::MouseButtonUp { .. }
                    | Event::MouseMotion { .. }
                    | Event::MouseWheel { .. }
            ) {
                stats.last_input = Instant::now();
            }
            let event_started = Instant::now();
            // A failed event costs that event, never the app.
            let handled = self.match_event(event, screen, texture);
            self.event_errors
                .report(handled, "ERROR", "Erreur de traitement SDL");
            let elapsed = event_started.elapsed();
            if elapsed >= Duration::from_millis(500) {
                crate::diagnostics::record(
                    "WARN",
                    format!("Traitement d'un événement SDL lent: {elapsed:?}"),
                );
            }
        }

        if std::mem::take(&mut self.device_reset) {
            // The renderer lost its textures: make the chart texture
            // again, as a resize to the same size does. Without a size,
            // try again next iteration.
            let size = screen.canvas.output_size();
            match self
                .resize_errors
                .report(size, "ERROR", "Taille de la fenêtre inconnue")
            {
                Some(size) => {
                    crate::diagnostics::record(
                        "WARN",
                        "Périphérique de rendu réinitialisé; texture refaite",
                    );
                    self.resized.get_or_insert(size);
                }
                None => self.device_reset = true,
            }
        }
        if let Some((width, height)) = self.resized.take() {
            // A texture that cannot be made yet keeps the old one and
            // the old size; the resize is tried again next iteration.
            let resized = self.resize(screen, texture_creator, texture, width, height);
            if self
                .resize_errors
                .report(resized, "ERROR", "Redimensionnement impossible, réessayé")
                .is_none()
            {
                self.resized.get_or_insert((width, height));
            }
        }

        // Live data in: drain each channel, hand it to the session, and
        // redraw when it says so. The rules live on Track and Session.
        let now = Instant::now();
        while let Ok(position) = self.live.track.try_recv() {
            if self.session.track.record(position, now) {
                stats.track_updates += 1;
                self.dirty = true;
            }
        }
        self.session.track.tick(now);

        if let Some(boat) =
            crate::feed::take_change(&mut self.live.boat, &mut self.boat_closed, "bateau")
        {
            self.session.on_boat(boat, now);
            self.dirty = true;
        }
        if self.session.expire_boat(now) {
            self.dirty = true;
        }

        self.fire_long_press();

        if self.session.state.display_ais {
            if let Some(targets) =
                crate::feed::take_change(&mut self.live.ais, &mut self.ais_closed, "AIS")
            {
                self.session.set_ais(targets);
                self.dirty = true;
            }
            if self.session.expire_ais(now) {
                self.dirty = true;
            }
        }
        if self.session.renderer.advance_zoom_animation() {
            self.dirty = true;
        }
        //////////////////////   Display /////////////////////////
        if self.dirty && !self.dragging_chart() {
            let display_started = Instant::now();
            // A frame that fails is logged and tried again next
            // iteration: the chart must keep drawing.
            let drawn = self.display(screen, texture);
            if self
                .render_errors
                .report(drawn, "ERROR", "Erreur de rendu, image réessayée")
                .is_some()
            {
                stats.rendered_frames += 1;
                self.dirty = false;
            }
            let elapsed = display_started.elapsed();
            if elapsed >= Duration::from_millis(500) {
                crate::diagnostics::record("WARN", format!("Rendu lent: {elapsed:?}"));
            }
        }

        /////////////////  Mouse //////////////////////////////////
        self.mouse(screen);

        /////////////// preload tiles on the edge ///////////////////////////
        let prefetch_started = Instant::now();
        self.session.renderer.prefetch_step(8);
        let elapsed = prefetch_started.elapsed();
        if elapsed >= Duration::from_millis(500) {
            crate::diagnostics::record(
                "WARN",
                format!("Préchargement des tuiles lent: {elapsed:?}"),
            );
        }
    }

    /// After a panic in an iteration: drop the gesture in progress (its
    /// state may be half changed) and draw the next frame in full.
    fn after_panic(&mut self) {
        self.grab = None;
        self.dirty = true;
    }

    /// Runs an action on the session and schedules a redraw. Opening the
    /// chart manager is the shell's: the window holds SDL state.
    fn execute_action(&mut self, action: AppAction) {
        if action == AppAction::OpenChartManager {
            self.chart_manager.open();
            self.session.highlight = None;
            self.topbar.close();
        } else {
            self.session.execute_action(action);
        }
        self.dirty = true;
    }

    /// Closes the chart manager.
    fn close_chart_manager(&mut self) {
        if self.chart_manager.typing.is_some() {
            self.text_input.stop();
        }
        self.chart_manager.close();
        self.dirty = true;
    }

    /// Applies a confirmed destructive action and schedules a redraw.
    fn apply_confirmation(&mut self, action: ConfirmAction) {
        self.session.apply_confirmation(action);
        self.dirty = true;
    }

    pub(crate) fn main_loop(
        &mut self,
        event_pump: &mut sdl2::EventPump,
        screen: &mut screen::Screen<'_>,
        texture_creator: &TextureCreator<WindowContext>,
        texture: &mut Texture,
    ) -> anyhow::Result<()> {
        let mut stats = LoopStats::new();
        crate::diagnostics::record("INFO", "Boucle SDL démarrée");
        while self.session.running {
            let loop_started = Instant::now();
            // A panic costs this iteration, never the app: the hook has
            // logged it, the next frame is drawn in full, and an input
            // gesture half done is dropped.
            let survived = crate::diagnostics::survive(|| {
                self.iteration(&mut stats, event_pump, screen, texture_creator, texture)
            });
            if survived.is_none() {
                self.after_panic();
            }

            /////// Sleep to limit CPU usage /////////////////////////
            // Outside the guard, so a panic every frame is not a busy loop.
            std::thread::sleep(Duration::from_millis(16));
            stats.loop_count += 1;
            let elapsed = loop_started.elapsed();
            if elapsed >= Duration::from_secs(1) {
                crate::diagnostics::record(
                    "WARN",
                    format!("Itération principale lente: {elapsed:?}"),
                );
            }
        } // end while self.session.running

        crate::diagnostics::record("INFO", "Arrêt demandé, sauvegarde finale de la trace");
        self.session.track.close();
        if let Err(error) = self.session.state.save(&self.state_file) {
            crate::diagnostics::record("ERROR", format!("Erreur sauvegarde état: {error}"));
        }
        crate::diagnostics::record("INFO", "Boucle SDL terminée");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::session::valid_boat_position;
    use crate::boat::BoatState;

    /// The real `Plotter` on SDL's dummy video driver (no display needed),
    /// every file in a temp directory. One test, because SDL is set up
    /// once per process.
    #[tokio::test]
    async fn the_main_loop_survives_a_failing_resize_and_retries_it() {
        use super::session::test_support::{session, Fixture};
        sdl2::hint::set("SDL_VIDEODRIVER", "dummy");
        let Fixture {
            dir,
            chart,
            session,
        } = session(&[2]);
        drop(session);

        let mut config = crate::config::AppConfig::default();
        config.input.source = crate::config::InputSource::Nmea2k;
        config.charts.directory = dir.join("charts").to_string_lossy().into_owned();
        config.charts.default = chart.to_string_lossy().into_owned();
        config.window.width = 640;
        config.window.height = 480;
        let files = super::Files {
            state: dir.join("state.json"),
            track: dir.join("track.json"),
            route: dir.join("route.json"),
            waypoints: dir.join("waypoints.json"),
            chart_settings: dir.join("charts.json"),
            export: dir.join("export"),
        };

        let mut sdl = crate::screen::MoteurSdl::init().unwrap();
        let mut screen = crate::screen::Screen::new(
            &sdl.ttf_context,
            &sdl.context,
            &config.window,
            &config.fonts,
            &config.appearance,
        )
        .unwrap();
        let texture_creator = screen.canvas.texture_creator();
        let mut texture = super::chart_texture(&texture_creator, 640, 480).unwrap();
        let mut plotter = super::Plotter::new(
            &sdl.context,
            &sdl.ttf_context,
            640,
            480,
            &config,
            None,
            files,
        )
        .unwrap();
        let mut stats = super::LoopStats::new();
        let mut step = |plotter: &mut super::Plotter, screen: &mut crate::screen::Screen| {
            plotter.iteration(
                &mut stats,
                &mut sdl.event_pump,
                screen,
                &texture_creator,
                &mut texture,
            )
        };

        step(&mut plotter, &mut screen);
        assert!(plotter.session.running);
        assert!(!plotter.dirty, "the first frame was drawn");

        // No texture can be this big: the resize fails.
        let huge = (200_000, 200_000);
        plotter.resized = Some(huge);
        step(&mut plotter, &mut screen);
        assert!(plotter.session.running, "the loop goes on");
        assert_eq!(plotter.resized, Some(huge), "tried again next iteration");
        assert_eq!(screen.screen_width, 640, "the old size stays");

        // A size that works is applied.
        plotter.resized = Some((800, 600));
        step(&mut plotter, &mut screen);
        assert_eq!(plotter.resized, None);
        assert_eq!((screen.screen_width, screen.screen_height), (800, 600));

        // The AIS details window draws with a frame.
        let centre = plotter.session.state.position.unwrap_or((0.0, 0.0));
        let mut ship = crate::ais::AisTarget::new("vessels.urn:mrn:imo:mmsi:232001001".into());
        ship.position = Some(centre);
        ship.name = Some("SEA SPIRIT".into());
        ship.ship_type = Some(70);
        ship.nav_status = Some(7);
        plotter.session.set_ais(vec![ship]);
        plotter.session.selected_ais = Some("vessels.urn:mrn:imo:mmsi:232001001".into());
        plotter.dirty = true;
        step(&mut plotter, &mut screen);
        assert!(!plotter.dirty, "the frame with the window was drawn");
        assert!(plotter.session.selected_ais_target().is_some());

        drop(plotter);
        drop(screen);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepts_received_positions_within_gps_bounds() {
        let mut boat = BoatState::default();
        boat.position_valid = true;
        assert!(valid_boat_position(&boat));

        boat.longitude = 180.0;
        boat.latitude = 90.0;
        assert!(valid_boat_position(&boat));

        boat.longitude = -180.0;
        boat.latitude = -90.0;
        assert!(valid_boat_position(&boat));
    }

    #[test]
    fn rejects_missing_non_finite_or_out_of_range_boat_positions() {
        let mut boat = BoatState::default();
        assert!(!valid_boat_position(&boat));

        boat.position_valid = true;
        boat.longitude = f64::NAN;
        assert!(!valid_boat_position(&boat));

        boat.longitude = 0.0;
        boat.latitude = f64::INFINITY;
        assert!(!valid_boat_position(&boat));

        boat.latitude = 0.0;
        boat.longitude = 180.1;
        assert!(!valid_boat_position(&boat));

        boat.longitude = 0.0;
        boat.latitude = -90.1;
        assert!(!valid_boat_position(&boat));
    }
}
