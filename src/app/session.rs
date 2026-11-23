//! Everything the navigator's actions act on, with no SDL handle in it:
//! the chart view, the saved state, the route, track, waypoints and
//! measurement, the boat, and the confirmation that may be pending. The
//! action layer (`actions.rs`) is implemented on this type, so it can be
//! driven by tests with a fixture chart and temporary paths. `Plotter` owns
//! one `Session` plus the SDL widgets and channels around it.

use std::path::PathBuf;

use super::confirm_dialog::Confirmation;
use crate::boat::BoatState;
use crate::mbtiles_renderer::MBTilesRenderer;
use crate::mesure::Mesure;
use crate::route::Route;
use crate::state::State;
use crate::track::Track;
use crate::waypoints::Waypoints;
use crate::Mode;

pub(crate) struct Session {
    pub renderer: MBTilesRenderer,
    pub state: State,
    pub mode: Mode,
    pub running: bool,
    pub boat: BoatState,
    pub route: Route,
    pub mesure: Mesure,
    pub track: Track,
    pub waypoints: Waypoints,
    /// Chart files offered in the Charts menu, in menu order.
    pub charts_path: Vec<String>,
    pub confirmation: Confirmation,
    /// Where GPX exports are written.
    pub export_dir: PathBuf,
}

impl Session {
    /// The position at the centre of the screen: the boat while following,
    /// otherwise the saved position, falling back to the chart's own centre
    /// the first time. Pushes it, the pan offset and the course-up rotation
    /// into the renderer so `screen_to_gps`, `gps_to_point`, prefetching and
    /// the next frame all use the same geometry.
    pub fn sync_view(&mut self) {
        let saved = *self
            .state
            .position
            .get_or_insert_with(|| self.renderer.initial_center());
        let (lon, lat) = if self.state.follow && valid_boat_position(&self.boat) {
            (self.boat.longitude, self.boat.latitude)
        } else {
            saved
        };
        let rotation = if self.state.course_up
            && self.boat.position_valid
            && self.boat.course_over_ground.is_finite()
        {
            self.boat.course_over_ground
        } else {
            0.0
        };
        self.renderer.set_rotation(rotation);
        self.renderer.look_at(lon, lat, self.state.map_offset);
    }
}

pub(crate) fn valid_boat_position(boat: &BoatState) -> bool {
    boat.position_valid
        && boat.longitude.is_finite()
        && boat.latitude.is_finite()
        && (-180.0..=180.0).contains(&boat.longitude)
        && (-90.0..=90.0).contains(&boat.latitude)
}

/// The Session fixture shared by the action and tool tests.
#[cfg(test)]
pub(in crate::app) mod test_support {
    use super::super::confirm_dialog::Confirmation;
    use super::Session;
    use crate::boat::BoatState;
    use crate::config::RenderingConfig;
    use crate::mbtiles_renderer::test_support::fixture_chart;
    use crate::mbtiles_renderer::MBTilesRenderer;
    use crate::{
        mesure::Mesure, route::Route, state::State, track::Track, waypoints::Waypoints, Mode,
    };
    use std::path::PathBuf;

    /// A Session over a fixture chart, with every file under a temp dir.
    /// The route has three points along the meridian; the Charts menu has
    /// the fixture chart first and a missing file second.
    pub(in crate::app) struct Fixture {
        pub dir: PathBuf,
        pub chart: PathBuf,
        pub session: Session,
    }

    pub(in crate::app) fn session(zooms: &[i32]) -> Fixture {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("plotter-session-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chart = fixture_chart(zooms);
        let chart_str = chart.to_str().unwrap().to_string();
        let state = State::new(dir.join("state.json"), chart_str.clone()).unwrap();
        let renderer = MBTilesRenderer::new(
            &chart_str,
            state.zoom,
            512,
            256,
            &RenderingConfig::default(),
            &chart_str,
        )
        .unwrap();
        let mut session = Session {
            renderer,
            state,
            mode: Mode::default(),
            running: true,
            boat: BoatState::default(),
            route: Route::from_points_at(
                vec![(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)],
                &dir.join("route.json"),
            ),
            mesure: Mesure::new().unwrap(),
            track: Track::new(&dir.join("track.json")),
            waypoints: Waypoints::empty_at(&dir.join("waypoints.json")),
            charts_path: vec![
                chart_str,
                dir.join("missing.mbtiles").to_string_lossy().into_owned(),
            ],
            confirmation: Confirmation::default(),
            export_dir: dir.join("export"),
        };
        session.sync_view();
        Fixture {
            dir,
            chart,
            session,
        }
    }

    impl Fixture {
        /// Drops the session (which holds the chart open) before deleting.
        pub(in crate::app) fn finish(self) {
            let Fixture {
                dir,
                chart,
                session,
            } = self;
            drop(session);
            std::fs::remove_file(chart).unwrap();
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
