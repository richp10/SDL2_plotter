use crate::boat::BoatState;
use crate::catalogue::Catalogue;
use crate::{
    ais, buttombar, config::AppConfig, mbtiles_renderer, route, screen, signalk, state, topbar,
    track, waypoints,
};
use mbtiles_renderer::MBTilesRenderer;
use sdl2::event::Event;
use sdl2::render::{Texture, TextureCreator};
use sdl2::ttf::Sdl2TtfContext;
use sdl2::video::WindowContext;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::watch;

mod actions;
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

const STATE_FILE: &str = "default/state.json";
const DEFAULT_TRACK_PATH: &str = "default/track.json";
/// What the user decided about each chart file; see `catalogue`.
const CHART_SETTINGS: &str = "settings/charts.json";
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
    topbar: topbar::Topbar<'ttf>,
    buttombar: buttombar::Buttombar<'ttf>,
    confirm_dialog: confirm_dialog::ConfirmDialog<'ttf>,
    chart_manager: chart_manager::ChartManager<'ttf>,
    /// The photo button and the thumbnails it shows.
    photo_button: photo_button::PhotoButtonView<'ttf>,
    thumbnails: mbtiles_renderer::Thumbnails,
    pending_waypoint: Option<PendingWaypoint>,
    text_input: sdl2::keyboard::TextInputUtil,
    mouse: Mouse,
    state_rx: watch::Receiver<BoatState>,
    track_rx: tokio::sync::mpsc::UnboundedReceiver<(f64, f64)>,
    ais_rx: watch::Receiver<Vec<ais::AisTarget>>,
    streamer: Option<crate::stream::Streamer>,
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
    ) -> anyhow::Result<Self> {
        let text_input = sdl_context
            .video()
            .map_err(anyhow::Error::msg)?
            .text_input();
        let boat = BoatState::default();
        let (state_tx, state_rx) = watch::channel(boat);
        let (track_tx, track_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ais_tx, ais_rx) = watch::channel(Vec::new());
        tokio::spawn(signalk::signalk_task(
            state_tx,
            track_tx,
            ais_tx,
            config.signalk.clone(),
        ));

        let catalogue = Catalogue::load(
            Path::new(&config.charts.directory),
            Path::new(CHART_SETTINGS),
        );
        for line in catalogue.describe() {
            crate::diagnostics::record("INFO", line);
        }
        let state = state::State::new(STATE_FILE, config.charts.default.clone())
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
        let route = route::Route::new();
        let track = track::Track::with_checkpoint_interval(
            Path::new(DEFAULT_TRACK_PATH),
            Duration::from_secs(config.track.checkpoint_interval_seconds),
        );
        crate::diagnostics::record(
            "INFO",
            format!(
                "Trace initialisée: {} points dans {}",
                track.point_count(),
                DEFAULT_TRACK_PATH
            ),
        );
        let waypoints = waypoints::Waypoints::new();

        let session = Session::new(
            renderer,
            state,
            boat,
            route,
            track,
            waypoints,
            catalogue,
            config.charts.default.clone(),
            PathBuf::from("export"),
        );

        Ok(Self {
            session,
            dirty: true,
            gestures: GestureRecognizer::new(LONG_CLICK, MOVE_TOLERANCE),
            grab: None,
            drag_redrawn: Instant::now(),
            resized: None,
            topbar,
            buttombar,
            confirm_dialog,
            chart_manager,
            photo_button,
            thumbnails: mbtiles_renderer::Thumbnails::new(),
            pending_waypoint: None,
            text_input,
            mouse: Mouse::Hand,
            state_rx,
            track_rx,
            ais_rx,
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
        self.dirty = true;
        crate::diagnostics::record("INFO", format!("Fenêtre redimensionnée: {width}x{height}"));
        Ok(())
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
        let mut last_diagnostics = Instant::now();
        let mut loop_count: u64 = 0;
        let mut event_count: u64 = 0;
        let mut track_updates: u64 = 0;
        let mut rendered_frames: u64 = 0;
        let mut last_input = Instant::now();
        crate::diagnostics::record("INFO", "Boucle SDL démarrée");
        while self.session.running {
            let loop_started = Instant::now();
            if last_diagnostics.elapsed() >= DIAGNOSTICS_INTERVAL {
                crate::diagnostics::record(
					"INFO",
					format!(
						"État boucle: iterations={loop_count}, événements={event_count}, points ajoutés={track_updates}, images rendues={rendered_frames}, points trace={}, sauvegarde en attente={}, dernier événement utilisateur il y a {:?}",
						self.session.track.point_count(),
						self.session.track.save_pending(),
						last_input.elapsed()
					),
				);
                last_diagnostics = Instant::now();
            }

            /////////////////////   Event /////////////////////////
            // Input converts clicks with the geometry the renderer holds;
            // make sure it reflects the current state, not the last frame.
            self.session.sync_view();
            for event in event_pump.poll_iter() {
                event_count += 1;
                if matches!(
                    &event,
                    Event::KeyDown { .. }
                        | Event::TextInput { .. }
                        | Event::MouseButtonDown { .. }
                        | Event::MouseButtonUp { .. }
                        | Event::MouseMotion { .. }
                        | Event::MouseWheel { .. }
                ) {
                    last_input = Instant::now();
                }
                let event_started = Instant::now();
                self.match_event(event, screen, texture).map_err(|error| {
                    crate::diagnostics::record(
                        "ERROR",
                        format!("Erreur de traitement SDL: {error}"),
                    );
                    anyhow::Error::msg(error)
                })?;
                let elapsed = event_started.elapsed();
                if elapsed >= Duration::from_millis(500) {
                    crate::diagnostics::record(
                        "WARN",
                        format!("Traitement d'un événement SDL lent: {elapsed:?}"),
                    );
                }
            }

            if let Some((width, height)) = self.resized.take() {
                self.resize(screen, texture_creator, texture, width, height)?;
            }

            // Live data in: drain each channel, hand it to the session, and
            // redraw when it says so. The rules live on Track and Session.
            let now = Instant::now();
            while let Ok(position) = self.track_rx.try_recv() {
                if self.session.track.record(position, now) {
                    track_updates += 1;
                    self.dirty = true;
                }
            }
            self.session.track.tick(now);

            if self.state_rx.has_changed()? {
                let boat = *self.state_rx.borrow_and_update();
                self.session.on_boat(boat);
                self.dirty = true;
            }

            self.fire_long_press();

            if self.session.state.display_ais {
                if self.ais_rx.has_changed().unwrap_or(false) {
                    self.session
                        .set_ais(self.ais_rx.borrow_and_update().clone());
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
                self.display(screen, texture).map_err(anyhow::Error::msg)?;
                rendered_frames += 1;
                let elapsed = display_started.elapsed();
                if elapsed >= Duration::from_millis(500) {
                    crate::diagnostics::record("WARN", format!("Rendu lent: {elapsed:?}"));
                }
                self.dirty = false;
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

            /////// Sleep to limit CPU usage /////////////////////////
            std::thread::sleep(Duration::from_millis(16));
            loop_count += 1;
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
        if let Err(error) = self.session.state.save(STATE_FILE) {
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
