use crate::boat::BoatState;
use crate::{
    ais, buttombar, mbtiles_renderer, mesure, route, screen, signalk, state, topbar, track,
    waypoints, config::AppConfig,
};
use crate::Mode;
use mbtiles_renderer::MBTilesRenderer;
use sdl2::event::Event;
use sdl2::ttf::Sdl2TtfContext;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::sync::watch;

mod actions;
mod confirm_dialog;
mod input;
mod render;

const STATE_FILE: &str = "default/state.json";
const DEFAULT_TRACK_PATH: &str = "default/track.json";
const LONG_CLICK: Duration = Duration::from_millis(500);
const MOVE_TOLERANCE: i32 = 5;
const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(10);

fn valid_boat_position(boat: &BoatState) -> bool {
    boat.position_valid
        && boat.longitude.is_finite()
        && boat.latitude.is_finite()
        && (-180.0..=180.0).contains(&boat.longitude)
        && (-90.0..=90.0).contains(&boat.latitude)
}

enum AppAction {
    SetMode(Mode),
    CycleMode,
    ToggleFollow,
	FollowOn,
	FollowOff,
    ToggleGoto,
    ClearRoute,
    PreviousWaypoint,
    NextWaypoint,
    Quit,
    ClearTrack,
    ExportRouteGpx,
    ExportTrackGpx,
    ClearWaypoints,
    ExportWaypointsGpx,
    ChangeChart(usize),
    Pan(i32, i32),
    ZoomIn,
    ZoomOut,
    ShowWaypoints,
    HideWaypoints,
    ShowRoute,
    HideRoute,
    ShowTrack,
    HideTrack,
    ShowAis,
    HideAis,
    SetCourseUp(bool),
}

impl AppAction {
    fn name(&self) -> &'static str {
        match self {
            Self::SetMode(_) => "SetMode",
            Self::CycleMode => "CycleMode",
            Self::ToggleFollow => "ToggleFollow",
			Self::FollowOn => "On",
			Self::FollowOff => "Off",
            Self::ToggleGoto => "ToggleGoto",
            Self::ClearRoute => "ClearRoute",
            Self::PreviousWaypoint => "PreviousWaypoint",
            Self::NextWaypoint => "NextWaypoint",
            Self::Quit => "Quit",
            Self::ClearTrack => "ClearTrack",
            Self::ExportRouteGpx => "ExportRouteGpx",
            Self::ExportTrackGpx => "ExportTrackGpx",
            Self::ClearWaypoints => "ClearWaypoints",
            Self::ExportWaypointsGpx => "ExportWaypointsGpx",
            Self::ChangeChart(_) => "ChangeChart",
            Self::Pan(_, _) => "Pan",
            Self::ZoomIn => "ZoomIn",
            Self::ZoomOut => "ZoomOut",
            Self::ShowWaypoints => "ShowWaypoints",
            Self::HideWaypoints => "HideWaypoints",
            Self::ShowRoute => "ShowRoute",
            Self::HideRoute => "HideRoute",
            Self::ShowTrack => "ShowTrack",
            Self::HideTrack => "HideTrack",
            Self::ShowAis => "ShowAis",
            Self::HideAis => "HideAis",
            Self::SetCourseUp(_) => "SetCourseUp",
        }
    }
}

struct Press {
    start: Instant,
    x: i32,
    y: i32,
    moved: bool,
    long_fired: bool,
}

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
    #[allow(dead_code)]
    Wait,
}

pub(crate) struct Plotter<'ttf> {
    renderer: MBTilesRenderer,
    state: state::State,
    mode: Mode,
    running: bool,
    dirty: bool,
    is_draging: bool,
    xdrag: i32,
    ydrag: i32,
    drag_start: (i32, i32),
    boat: BoatState,
    topbar: topbar::Topbar<'ttf>,
    buttombar: buttombar::Buttombar<'ttf>,
    confirm_dialog: confirm_dialog::ConfirmDialog<'ttf>,
    route: route::Route,
    mesure: mesure::Mesure,
    track: track::Track,
    waypoints: waypoints::Waypoints,
    pending_waypoint: Option<PendingWaypoint>,
    text_input: sdl2::keyboard::TextInputUtil,
    charts_path: Vec<String>,
    charts_name: Vec<String>,
    press: Option<Press>,
    selected: Option<usize>,
    dragging_measure_start: bool,
    mouse: Mouse,
    state_rx: watch::Receiver<BoatState>,
    track_rx: tokio::sync::mpsc::UnboundedReceiver<(f64, f64)>,
    ais_rx: watch::Receiver<Vec<ais::AisTarget>>,
    #[allow(dead_code)]
    status_rx: watch::Receiver<String>,
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
        let (status_tx, status_rx) = watch::channel(String::new());
        tokio::spawn(signalk::signalk_task(
            state_tx,
            track_tx,
            ais_tx,
            status_tx,
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
        let route = route::Route::new().map_err(anyhow::Error::msg)?;
        let mesure = mesure::Mesure::new().map_err(anyhow::Error::msg)?;
        let track = track::Track::with_checkpoint_interval(
            Path::new(DEFAULT_TRACK_PATH),
            Duration::from_secs(config.track.checkpoint_interval_seconds),
        )?;
        crate::diagnostics::record(
            "INFO",
            format!(
                "Trace initialisée: {} points dans {}",
                track.point_count(),
                DEFAULT_TRACK_PATH
            ),
        );
        let waypoints = waypoints::Waypoints::new()?;

        Ok(Self {
            renderer,
            state,
            mode: Mode::default(),
            running: true,
            dirty: true,
            is_draging: false,
            xdrag: 0,
            ydrag: 0,
            drag_start: (0, 0),
            boat,
            topbar,
            buttombar,
            confirm_dialog,
            route,
            mesure,
            track,
            waypoints,
            pending_waypoint: None,
            text_input,
            charts_path,
            charts_name,
            press: None,
            selected: None,
            dragging_measure_start: false,
            mouse: Mouse::Hand,
            state_rx,
            track_rx,
            ais_rx,
            status_rx,
            ais_targets: Vec::new(),
            last_ais_cleanup: Instant::now(),
            streamer,
        })
    }

    pub(crate) fn main_loop(
        &mut self,
        event_pump: &mut sdl2::EventPump,
        map: &mut screen::Map<'_>,
        texture: &mut sdl2::render::Texture,
    ) -> anyhow::Result<()> {
        let mut last_diagnostics = Instant::now();
        let mut loop_count: u64 = 0;
        let mut event_count: u64 = 0;
        let mut track_updates: u64 = 0;
        let mut rendered_frames: u64 = 0;
        let mut last_input = Instant::now();
        crate::diagnostics::record("INFO", "Boucle SDL démarrée");
        while self.running {
            let loop_started = Instant::now();
            if last_diagnostics.elapsed() >= DIAGNOSTICS_INTERVAL {
                crate::diagnostics::record(
					"INFO",
					format!(
						"État boucle: iterations={loop_count}, événements={event_count}, points ajoutés={track_updates}, images rendues={rendered_frames}, points trace={}, sauvegarde en attente={}, dernier événement utilisateur il y a {:?}",
						self.track.point_count(),
						self.track.save_pending(),
						last_input.elapsed()
					),
				);
                last_diagnostics = Instant::now();
            }

            /////////////////////   Event /////////////////////////
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
                self.match_event(event, map, texture).map_err(|error| {
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
                if self.track.add(position) {
                    track_updates += 1;
                    self.dirty = true;
                    if self.track.point_count() == 1 {
                        self.track
                            .save_default_track(Path::new(DEFAULT_TRACK_PATH));
                    }
                }
            }

            if self.track.checkpoint_due() {
                self.track
                    .save_default_track(Path::new(DEFAULT_TRACK_PATH));
            }

            //////////////////////   Position update  /////////////////////////
            if self.state_rx.has_changed()? {
                let boat = *self.state_rx.borrow_and_update();
                if self.boat.position_valid != boat.position_valid {
                    crate::diagnostics::record(
                        "INFO",
                        if boat.position_valid {
                            "Position du bateau reçue; marqueur activé"
                        } else {
                            "Position du bateau invalide; marqueur masqué"
                        },
                    );
                }
                self.boat = boat;

                if self.state.follow && valid_boat_position(&boat) {
                    self.state.position = Some((boat.longitude, boat.latitude));
                }
                self.dirty = true;
            }

            self.long_click();

            //////////////// Ais Update //////////////////////////
            if self.state.display_ais {
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
            if self.renderer.advance_zoom_animation() {
                self.dirty = true;
            }
            //////////////////////   Display /////////////////////////
            if self.dirty && !self.is_draging {
                let display_started = Instant::now();
                self.display(map, texture)
                    .map_err(anyhow::Error::msg)?;
                rendered_frames += 1;
                let elapsed = display_started.elapsed();
                if elapsed >= Duration::from_millis(500) {
                    crate::diagnostics::record("WARN", format!("Rendu lent: {elapsed:?}"));
                }
                self.dirty = false;
            }

            /////////////////  Mouse //////////////////////////////////
            self.mouse(map);

            /////////////// preload tiles on the edge ///////////////////////////
            let prefetch_started = Instant::now();
            self.renderer.prefetch_step(8);
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
        } // end while self.running

        crate::diagnostics::record("INFO", "Arrêt demandé, sauvegarde finale de la trace");
        self.track
            .save_default_track(Path::new(DEFAULT_TRACK_PATH));
        if let Err(error) = self.state.save(STATE_FILE) {
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
