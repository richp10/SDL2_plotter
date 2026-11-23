use crate::boat::BoatState;
use crate::Mode;
use crate::{
    ais, buttombar, config::AppConfig, mbtiles_renderer, mesure, route, screen, signalk, state,
    topbar, track, waypoints,
};
use mbtiles_renderer::MBTilesRenderer;
use sdl2::event::Event;
use sdl2::ttf::Sdl2TtfContext;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::watch;

mod actions;
mod confirm_dialog;
mod gesture;
mod input;
mod render;
mod session;
mod tools;

pub(crate) use crate::action::AppAction;

use confirm_dialog::{ConfirmAction, Confirmation};
use gesture::GestureRecognizer;
use session::{valid_boat_position, Session};
use tools::Target;

const STATE_FILE: &str = "default/state.json";
const DEFAULT_TRACK_PATH: &str = "default/track.json";
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
    topbar: topbar::Topbar<'ttf>,
    buttombar: buttombar::Buttombar<'ttf>,
    confirm_dialog: confirm_dialog::ConfirmDialog<'ttf>,
    pending_waypoint: Option<PendingWaypoint>,
    text_input: sdl2::keyboard::TextInputUtil,
    mouse: Mouse,
    state_rx: watch::Receiver<BoatState>,
    track_rx: tokio::sync::mpsc::UnboundedReceiver<(f64, f64)>,
    ais_rx: watch::Receiver<Vec<ais::AisTarget>>,
    ais_targets: Vec<ais::AisTarget>,
    last_ais_cleanup: Instant,
    streamer: Option<crate::stream::Streamer>,
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

        let charts_path = state::list_maps(&config.charts.directory);
        let charts_name: Vec<String> = charts_path
            .iter()
            .map(|path| {
                std::path::Path::new(path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path)
                    .to_string()
            })
            .collect();
        let state = state::State::new(STATE_FILE, config.charts.default.clone())
            .map_err(anyhow::Error::msg)?;
        let renderer = MBTilesRenderer::new(
            &state.map,
            state.zoom,
            width,
            height,
            &config.rendering,
            &config.charts.default,
        )?;

        let topbar = topbar::Topbar::new(ttf_context, width, charts_name.clone(), &config.fonts)
            .map_err(anyhow::Error::msg)?;
        let buttombar = buttombar::Buttombar::new(ttf_context, width, height, &config.fonts)
            .expect("failed to build buttombar bar");
        let confirm_dialog =
            confirm_dialog::ConfirmDialog::new(ttf_context, width, height, &config.fonts)
                .map_err(anyhow::Error::msg)?;
        let route = route::Route::new();
        let mesure = mesure::Mesure::new().map_err(anyhow::Error::msg)?;
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

        let mut session = Session {
            renderer,
            state,
            mode: Mode::default(),
            running: true,
            boat,
            route,
            mesure,
            track,
            waypoints,
            charts_path,
            confirmation: Confirmation::default(),
            export_dir: PathBuf::from("export"),
        };
        // The renderer knows where it is looking before the first event or frame.
        session.sync_view();

        Ok(Self {
            session,
            dirty: true,
            gestures: GestureRecognizer::new(LONG_CLICK, MOVE_TOLERANCE),
            grab: None,
            topbar,
            buttombar,
            confirm_dialog,
            pending_waypoint: None,
            text_input,
            mouse: Mouse::Hand,
            state_rx,
            track_rx,
            ais_rx,
            ais_targets: Vec::new(),
            last_ais_cleanup: Instant::now(),
            streamer,
        })
    }

    /// Runs an action on the session and schedules a redraw.
    fn execute_action(&mut self, action: AppAction) {
        self.session.execute_action(action);
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
        texture: &mut sdl2::render::Texture,
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

            ////////////////////// Track update ///////////////////////
            while let Ok(position) = self.track_rx.try_recv() {
                if self.session.track.add(position) {
                    track_updates += 1;
                    self.dirty = true;
                    if self.session.track.point_count() == 1 {
                        self.session.track.save_default_track();
                    }
                }
            }

            if self.session.track.checkpoint_due() {
                self.session.track.save_default_track();
            }

            //////////////////////   Position update  /////////////////////////
            if self.state_rx.has_changed()? {
                let boat = *self.state_rx.borrow_and_update();
                if self.session.boat.position_valid != boat.position_valid {
                    crate::diagnostics::record(
                        "INFO",
                        if boat.position_valid {
                            "Position du bateau reçue; marqueur activé"
                        } else {
                            "Position du bateau invalide; marqueur masqué"
                        },
                    );
                }
                self.session.boat = boat;

                if self.session.state.follow && valid_boat_position(&boat) {
                    self.session.state.position = Some((boat.longitude, boat.latitude));
                }
                self.dirty = true;
            }

            self.fire_long_press();

            //////////////// Ais Update //////////////////////////
            if self.session.state.display_ais {
                if self.ais_rx.has_changed().unwrap_or(false) {
                    self.ais_targets = self.ais_rx.borrow_and_update().clone();
                    self.dirty = true;
                }

                if !self.ais_targets.is_empty()
                    && self.last_ais_cleanup.elapsed() >= Duration::from_secs(1)
                {
                    let previous_count = self.ais_targets.len();
                    let now = Instant::now();
                    self.ais_targets.retain(|target| target.is_fresh(now));
                    if self.ais_targets.len() != previous_count {
                        self.dirty = true;
                    }
                    self.last_ais_cleanup = now;
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
        self.session.track.save_default_track();
        if let Err(error) = self.session.state.save(STATE_FILE) {
            crate::diagnostics::record("ERROR", format!("Erreur sauvegarde état: {error}"));
        }
        crate::diagnostics::record("INFO", "Boucle SDL terminée");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::valid_boat_position;
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
