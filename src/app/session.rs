//! Everything the navigator's actions act on, with no SDL handle in it:
//! the chart view, the saved state, the route, track, waypoints and
//! measurement, the boat, and the confirmation that may be pending. The
//! action layer (`actions.rs`) is implemented on this type, so it can be
//! driven by tests with a fixture chart and temporary paths. `Plotter` owns
//! one `Session` plus the SDL widgets and channels around it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::confirm_dialog::Confirmation;
use crate::ais::AisTarget;
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
    /// The AIS targets currently shown, as last published by the Signal K
    /// task and thinned by `expire_ais`.
    pub ais_targets: Vec<AisTarget>,
    last_ais_expiry: Instant,
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
    /// A session over the given parts, with no AIS targets yet.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        renderer: MBTilesRenderer,
        state: State,
        boat: BoatState,
        route: Route,
        track: Track,
        waypoints: Waypoints,
        charts_path: Vec<String>,
        export_dir: PathBuf,
    ) -> Self {
        let mut session = Session {
            renderer,
            state,
            mode: Mode::default(),
            running: true,
            boat,
            ais_targets: Vec::new(),
            last_ais_expiry: Instant::now(),
            route,
            mesure: Mesure::new(),
            track,
            waypoints,
            charts_path,
            confirmation: Confirmation::default(),
            export_dir,
        };
        // The renderer knows where it is looking before the first event or frame.
        session.sync_view();
        session
    }

    /// A new fix from the Signal K task. While following, the view moves
    /// with a valid fix; an invalid fix is kept (the marker hides) but never
    /// followed. Logs when the fix becomes valid or invalid.
    pub fn on_boat(&mut self, boat: BoatState) {
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
    }

    /// The AIS list as the Signal K task last published it.
    pub fn set_ais(&mut self, targets: Vec<AisTarget>) {
        self.ais_targets = targets;
    }

    /// Drops targets silent for longer than `ais::TARGET_TIMEOUT`, checking
    /// at most once a second. The Signal K task applies the same rule when a
    /// delta arrives; this pass covers a connection that has gone quiet.
    /// Returns true when the list changed.
    pub fn expire_ais(&mut self, now: Instant) -> bool {
        if self.ais_targets.is_empty()
            || now.duration_since(self.last_ais_expiry) < Duration::from_secs(1)
        {
            return false;
        }
        self.last_ais_expiry = now;
        crate::ais::expire(&mut self.ais_targets, now)
    }

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
    use super::Session;
    use crate::boat::BoatState;
    use crate::config::RenderingConfig;
    use crate::mbtiles_renderer::test_support::fixture_chart;
    use crate::mbtiles_renderer::MBTilesRenderer;
    use crate::{route::Route, state::State, track::Track, waypoints::Waypoints};
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
        let session = Session::new(
            renderer,
            state,
            BoatState::default(),
            Route::from_points_at(
                vec![(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)],
                &dir.join("route.json"),
            ),
            Track::new(&dir.join("track.json")),
            Waypoints::empty_at(&dir.join("waypoints.json")),
            vec![
                chart_str,
                dir.join("missing.mbtiles").to_string_lossy().into_owned(),
            ],
            dir.join("export"),
        );
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

#[cfg(test)]
mod tests {
    use super::test_support::session;
    use crate::ais::{AisTarget, TARGET_TIMEOUT};
    use crate::boat::BoatState;
    use std::time::{Duration, Instant};

    fn boat_at(lon: f64, lat: f64) -> BoatState {
        BoatState {
            longitude: lon,
            latitude: lat,
            position_valid: true,
            speed_over_ground: 5.0,
            course_over_ground: 1.0,
        }
    }

    #[test]
    fn a_boat_update_moves_the_view_only_while_following() {
        let mut fx = session(&[2]);
        fx.session.state.follow = false;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.on_boat(boat_at(10.0, 20.0));
        assert_eq!(fx.session.boat.longitude, 10.0);
        assert_eq!(fx.session.state.position, Some((0.0, 0.0)), "not following");

        fx.session.state.follow = true;
        fx.session.on_boat(boat_at(11.0, 21.0));
        assert_eq!(fx.session.state.position, Some((11.0, 21.0)));

        let mut invalid = boat_at(12.0, 22.0);
        invalid.position_valid = false;
        fx.session.on_boat(invalid);
        assert_eq!(
            fx.session.state.position,
            Some((11.0, 21.0)),
            "invalid fix is not followed"
        );
        assert!(
            !fx.session.boat.position_valid,
            "but the boat state is kept"
        );
        fx.finish();
    }

    #[test]
    fn stale_ais_targets_expire_after_the_timeout_and_at_most_once_a_second() {
        let mut fx = session(&[2]);
        fx.session.set_ais(vec![
            AisTarget::new("vessels.a".to_string()),
            AisTarget::new("vessels.b".to_string()),
        ]);
        let now = Instant::now();
        assert!(!fx.session.expire_ais(now), "fresh targets stay");
        assert_eq!(fx.session.ais_targets.len(), 2);

        let later = now + TARGET_TIMEOUT + Duration::from_secs(1);
        assert!(
            !fx.session.expire_ais(now + Duration::from_millis(500)),
            "checked less than a second ago"
        );
        assert!(fx.session.expire_ais(later), "both expired");
        assert!(fx.session.ais_targets.is_empty());
        assert!(
            !fx.session.expire_ais(later + Duration::from_secs(2)),
            "nothing left to expire"
        );
        fx.finish();
    }
}
