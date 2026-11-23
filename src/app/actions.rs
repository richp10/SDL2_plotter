use chrono::Local;

use super::confirm_dialog::ConfirmAction;
use super::session::Session;
use super::AppAction;
use crate::Mode;

impl Session {
    pub(super) fn execute_action(&mut self, action: AppAction) {
        crate::diagnostics::record("DEBUG", format!("Action: {}", action.name()));

        match action {
            AppAction::SetMode(mode) => self.mode = mode,
            AppAction::CycleMode => {
                self.mode = match self.mode {
                    Mode::EditRoute => Mode::EditWaypoint,
                    Mode::EditWaypoint => Mode::Measurement,
                    Mode::Measurement => Mode::EditRoute,
                };
            }
            AppAction::ToggleFollow => {
                self.state.follow = !self.state.follow;
                self.center_map_on_visible_center();
            }
            AppAction::FollowOn => {
                self.state.follow = true;
                self.center_map_on_visible_center();
            }
            AppAction::FollowOff => {
                self.state.follow = false;
                self.center_map_on_visible_center();
            }
            AppAction::ToggleGoto => {
                self.state.goto = !self.state.goto;
                self.state.nav_id = 0;
            }
            AppAction::ClearRoute => {
                self.confirmation.request(ConfirmAction::ClearRoute);
            }
            AppAction::PreviousWaypoint => {
                self.state.nav_id = self.state.nav_id.saturating_sub(1);
            }
            AppAction::NextWaypoint => {
                let last = self.route.get_nb_wp().saturating_sub(1);
                self.state.nav_id = self.state.nav_id.saturating_add(1).min(last);
            }
            AppAction::Quit => self.running = false,
            AppAction::ClearTrack => {
                self.confirmation.request(ConfirmAction::ClearTrack);
            }
            AppAction::ExportRouteGpx => {
                let now = Local::now();
                let path = self
                    .export_dir
                    .join(format!("Route-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.route.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record(
                        "INFO",
                        format!("Route exportée: {}", path.display()),
                    ),
                    Err(error) => {
                        crate::diagnostics::record(
                            "ERROR",
                            format!("Erreur export route GPX: {error}"),
                        );
                    }
                }
            }
            AppAction::ExportTrackGpx => {
                let now = Local::now();
                let path = self
                    .export_dir
                    .join(format!("Track-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.track.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record(
                        "INFO",
                        format!("Trace exportée: {}", path.display()),
                    ),
                    Err(error) => {
                        crate::diagnostics::record(
                            "ERROR",
                            format!("Erreur export trace GPX: {error}"),
                        );
                    }
                }
            }
            AppAction::ClearWaypoints => {
                self.confirmation.request(ConfirmAction::ClearWaypoints);
            }
            AppAction::ExportWaypointsGpx => {
                let now = Local::now();
                let path = self
                    .export_dir
                    .join(format!("Waypoints-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.waypoints.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record(
                        "INFO",
                        format!("Waypoints exportés: {}", path.display()),
                    ),
                    Err(error) => {
                        crate::diagnostics::record(
                            "ERROR",
                            format!("Erreur export waypoints GPX: {error}"),
                        );
                    }
                }
            }
            AppAction::ChangeChart(index) => {
                match self.renderer.change_map(&self.charts_path[index]) {
                    Ok(()) => {
                        self.state.map = self.charts_path[index].clone();
                        self.state.map_offset = (0, 0);
                    }
                    Err(error) => {
                        crate::diagnostics::record(
                            "ERROR",
                            format!("Impossible d'ouvrir {}: {error}", self.charts_path[index]),
                        );
                    }
                }
            }
            AppAction::Pan(dx, dy) => {
                if self.state.follow {
                    self.state.map_offset.0 += dx;
                    self.state.map_offset.1 += dy;
                } else if let Some(center) = self.renderer.view().panned_center(dx, dy) {
                    self.state.position = Some(center);
                    self.state.map_offset = (0, 0);
                }
            }
            AppAction::ZoomIn => {
                self.state.zoom = self.renderer.zoom_increase();
            }
            AppAction::ZoomOut => {
                self.state.zoom = self.renderer.zoom_decrease();
            }
            AppAction::HideWaypoints => self.state.display_waypoint = false,
            AppAction::ShowWaypoints => self.state.display_waypoint = true,
            AppAction::HideRoute => self.state.display_route = false,
            AppAction::ShowRoute => self.state.display_route = true,
            AppAction::HideTrack => self.state.display_track = false,
            AppAction::ShowTrack => self.state.display_track = true,
            AppAction::HideAis => self.state.display_ais = false,
            AppAction::ShowAis => self.state.display_ais = true,
            AppAction::SetCourseUp(enabled) => self.state.course_up = enabled,
        }

        self.sync_view();
    }

    pub(super) fn apply_confirmation(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::ClearRoute => {
                self.state.goto = false;
                if let Err(error) = self.route.clear() {
                    crate::diagnostics::record(
                        "ERROR",
                        format!("Erreur sauvegarde route: {error}"),
                    );
                }
            }
            ConfirmAction::ClearTrack => {
                self.track.reset();
                self.track.save_default_track();
            }
            ConfirmAction::ClearWaypoints => {
                self.waypoints.reset();
                if let Err(error) = self.waypoints.save() {
                    crate::diagnostics::record(
                        "ERROR",
                        format!("Erreur sauvegarde des waypoints: {error}"),
                    );
                }
            }
        }
        self.sync_view();
    }

    pub(super) fn center_map_on_visible_center(&mut self) {
        if let Some(position) = self.state.position {
            self.state.position = Some(
                self.renderer
                    .center_with_offset(position, self.state.map_offset),
            );
            self.state.map_offset = (0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::confirm_dialog::Confirmation;
    use super::*;
    use crate::boat::BoatState;
    use crate::config::RenderingConfig;
    use crate::mbtiles_renderer::test_support::fixture_chart;
    use crate::mbtiles_renderer::MBTilesRenderer;
    use crate::{mesure::Mesure, route::Route, state::State, track::Track, waypoints::Waypoints};
    use std::path::PathBuf;

    /// A Session over a fixture chart, with every file under a temp dir.
    /// The route has three points along the meridian; the Charts menu has
    /// the fixture chart first and a missing file second.
    struct Fixture {
        dir: PathBuf,
        chart: PathBuf,
        session: Session,
    }

    fn session(zooms: &[i32]) -> Fixture {
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
        fn finish(self) {
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

    #[test]
    fn show_and_hide_actions_flip_the_overlay_flags() {
        let mut fx = session(&[2]);
        fx.session.execute_action(AppAction::HideRoute);
        assert!(!fx.session.state.display_route);
        fx.session.execute_action(AppAction::ShowRoute);
        assert!(fx.session.state.display_route);
        fx.session.execute_action(AppAction::HideAis);
        assert!(!fx.session.state.display_ais);
        fx.session.execute_action(AppAction::HideWaypoints);
        assert!(!fx.session.state.display_waypoint);
        fx.session.execute_action(AppAction::HideTrack);
        assert!(!fx.session.state.display_track);
        fx.session.execute_action(AppAction::SetCourseUp(true));
        assert!(fx.session.state.course_up);
        fx.finish();
    }

    #[test]
    fn next_waypoint_stops_at_the_last_point_and_previous_at_the_first() {
        let mut fx = session(&[2]);
        for _ in 0..5 {
            fx.session.execute_action(AppAction::NextWaypoint);
        }
        assert_eq!(fx.session.state.nav_id, 2, "three points: ids 0..=2");
        for _ in 0..5 {
            fx.session.execute_action(AppAction::PreviousWaypoint);
        }
        assert_eq!(fx.session.state.nav_id, 0);
        fx.finish();
    }

    #[test]
    fn toggling_goto_restarts_navigation_at_the_first_waypoint() {
        let mut fx = session(&[2]);
        fx.session.execute_action(AppAction::NextWaypoint);
        fx.session.execute_action(AppAction::ToggleGoto);
        assert!(fx.session.state.goto);
        assert_eq!(fx.session.state.nav_id, 0);
        fx.finish();
    }

    #[test]
    fn panning_while_following_drags_the_chart_by_an_offset() {
        let mut fx = session(&[2]);
        fx.session.state.follow = true;
        fx.session.execute_action(AppAction::Pan(30, -10));
        fx.session.execute_action(AppAction::Pan(5, 5));
        assert_eq!(fx.session.state.map_offset, (35, -5));
        fx.finish();
    }

    #[test]
    fn panning_while_not_following_moves_the_saved_position() {
        let mut fx = session(&[2]);
        fx.session.state.follow = false;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.sync_view();
        // dx is centre minus pointer: +100 is a point 100 px left of centre.
        fx.session.execute_action(AppAction::Pan(100, 0));
        let (lon, lat) = fx.session.state.position.unwrap();
        assert!(lon < 0.0, "lon {lon}");
        assert!(lat.abs() < 1e-9, "lat {lat}");
        assert_eq!(fx.session.state.map_offset, (0, 0));
        fx.finish();
    }

    #[test]
    fn turning_follow_off_folds_the_pan_offset_into_the_position() {
        let mut fx = session(&[2]);
        fx.session.state.follow = true;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.execute_action(AppAction::Pan(100, 0));
        fx.session.execute_action(AppAction::FollowOff);
        assert!(!fx.session.state.follow);
        assert_eq!(fx.session.state.map_offset, (0, 0));
        let (lon, _) = fx.session.state.position.unwrap();
        assert!(
            lon < 0.0,
            "the dragged view is now the saved position, lon {lon}"
        );
        fx.finish();
    }

    #[test]
    fn zoom_actions_record_the_chart_zoom_in_the_state() {
        let mut fx = session(&[2, 3]);
        fx.session.execute_action(AppAction::ZoomOut);
        assert_eq!(fx.session.state.zoom, 2);
        fx.session.execute_action(AppAction::ZoomIn);
        assert_eq!(fx.session.state.zoom, 3);
        fx.session.execute_action(AppAction::ZoomIn);
        assert_eq!(fx.session.state.zoom, 3, "top of the chart: no change");
        fx.finish();
    }

    #[test]
    fn clearing_the_route_waits_for_confirmation_and_then_saves_an_empty_route() {
        let mut fx = session(&[2]);
        fx.session.execute_action(AppAction::ClearRoute);
        assert!(fx.session.confirmation.is_open());
        assert_eq!(fx.session.route.get_nb_wp(), 3, "nothing cleared yet");

        assert_eq!(fx.session.confirmation.resolve(false), None);
        assert_eq!(fx.session.route.get_nb_wp(), 3, "cancelled");

        fx.session.state.goto = true;
        fx.session.execute_action(AppAction::ClearRoute);
        let action = fx.session.confirmation.resolve(true).unwrap();
        assert_eq!(action, ConfirmAction::ClearRoute);
        fx.session.apply_confirmation(action);
        assert_eq!(fx.session.route.get_nb_wp(), 0);
        assert!(!fx.session.state.goto, "cannot navigate an empty route");
        let saved = crate::geo_data::read(&fx.dir.join("route.json")).unwrap();
        assert!(saved.is_empty(), "the empty route reached disk");
        fx.finish();
    }

    #[test]
    fn changing_chart_records_it_and_an_unreadable_chart_leaves_the_state_alone() {
        let mut fx = session(&[2]);
        let chart = fx.session.charts_path[0].clone();
        fx.session.state.map_offset = (5, 5);
        fx.session.execute_action(AppAction::ChangeChart(0));
        assert_eq!(fx.session.state.map, chart);
        assert_eq!(fx.session.state.map_offset, (0, 0));

        fx.session.state.map_offset = (7, 7);
        fx.session.execute_action(AppAction::ChangeChart(1));
        assert_eq!(
            fx.session.state.map, chart,
            "the missing chart is not recorded"
        );
        assert_eq!(fx.session.state.map_offset, (7, 7));
        fx.finish();
    }

    #[test]
    fn exporting_waypoints_writes_a_gpx_file_into_the_export_directory() {
        let mut fx = session(&[2]);
        fx.session.waypoints.add((1.0, 2.0), "Mooring".to_string());
        fx.session.execute_action(AppAction::ExportWaypointsGpx);
        let exports: Vec<_> = std::fs::read_dir(&fx.session.export_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(exports.len(), 1, "{exports:?}");
        let contents = std::fs::read_to_string(&exports[0]).unwrap();
        assert!(contents.contains("<wpt") && contents.contains("Mooring"));
        fx.finish();
    }

    #[test]
    fn quit_stops_the_session_and_cycle_mode_visits_every_mode() {
        let mut fx = session(&[2]);
        assert_eq!(fx.session.mode, Mode::Measurement);
        fx.session.execute_action(AppAction::CycleMode);
        assert_eq!(fx.session.mode, Mode::EditRoute);
        fx.session.execute_action(AppAction::CycleMode);
        assert_eq!(fx.session.mode, Mode::EditWaypoint);
        fx.session.execute_action(AppAction::CycleMode);
        assert_eq!(fx.session.mode, Mode::Measurement);
        fx.session.execute_action(AppAction::Quit);
        assert!(!fx.session.running);
        fx.finish();
    }
}
