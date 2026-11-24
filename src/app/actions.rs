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
                // Choosing a file chooses its layer; `sync_view` then shows
                // the file of that layer that best fits the view.
                self.anchor_on_visible_center();
                match self.catalogue.visible().nth(index) {
                    Some(file) => self.state.layer = Some(file.layer.clone()),
                    None => crate::diagnostics::record(
                        "WARN",
                        format!("Carte {index} absente du catalogue"),
                    ),
                }
            }
            AppAction::Pan(dx, dy) => {
                if self.following_boat() {
                    // The boat stays the anchor; the drag is an offset from it.
                    self.state.map_offset.0 += dx;
                    self.state.map_offset.1 += dy;
                } else if let Some(center) = self.renderer.view().panned_center(dx, dy) {
                    // Nothing to be offset from: the saved position moves.
                    self.state.position = Some(center);
                    self.state.map_offset = (0, 0);
                }
            }
            AppAction::ZoomIn => {
                // As deep as the most detailed file here goes, no deeper.
                if self.state.zoom < self.zoom_limit() {
                    self.anchor_on_visible_center();
                    self.zoom_to(self.state.zoom + 1);
                }
            }
            AppAction::ZoomOut => {
                // Below every file's first level the default chart shows.
                if self.state.zoom > 0 {
                    self.anchor_on_visible_center();
                    self.zoom_to(self.state.zoom - 1);
                }
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
                self.track.clear(std::time::Instant::now());
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

    /// True while the view is anchored on a live boat fix.
    fn following_boat(&self) -> bool {
        self.state.follow && super::session::valid_boat_position(&self.boat)
    }

    /// Unless the boat is the anchor, folds the pan offset into the saved
    /// position so that whatever changes the zoom keeps the visible centre.
    /// A pixel offset is a different distance at each zoom; a position is not.
    fn anchor_on_visible_center(&mut self) {
        if !self.following_boat() {
            self.center_map_on_visible_center();
        }
    }

    fn zoom_to(&mut self, zoom: i32) {
        self.renderer.zoom_to(zoom);
        self.state.zoom = zoom;
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
    use super::super::session::test_support::{session, session_with_charts};
    use super::*;

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

    fn valid_boat() -> crate::boat::BoatState {
        crate::boat::BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: true,
            speed_over_ground: 4.0,
            course_over_ground: 0.0,
        }
    }

    fn visible_centre(session: &Session) -> (f64, f64) {
        session
            .renderer
            .view()
            .to_lonlat(sdl2::rect::Point::new(256, 128))
            .unwrap()
    }

    fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
        // Offsets are whole pixels; allow one pixel of rounding at zoom 2.
        assert!(
            (actual.0 - expected.0).abs() < 0.4 && (actual.1 - expected.1).abs() < 0.4,
            "got {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn panning_while_following_without_a_fix_moves_the_position_like_not_following() {
        let mut fx = session(&[2]);
        fx.session.state.follow = true;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.sync_view();
        fx.session.execute_action(AppAction::Pan(100, 0));
        assert_eq!(
            fx.session.state.map_offset,
            (0, 0),
            "no boat to be offset from"
        );
        let (lon, _) = fx.session.state.position.unwrap();
        assert!(lon < 0.0, "the saved position moved: {lon}");
        fx.finish();
    }

    #[test]
    fn changing_chart_keeps_the_visible_centre_when_the_view_was_dragged() {
        let mut fx = session(&[2]);
        fx.session.state.follow = false;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.state.map_offset = (90, 30);
        fx.session.sync_view();
        let before = visible_centre(&fx.session);
        assert!(before.0 < -5.0, "the drag moved the view: {before:?}");

        fx.session.execute_action(AppAction::ChangeChart(0));

        assert_close(visible_centre(&fx.session), before);
        assert_eq!(fx.session.state.map_offset, (0, 0));
        fx.finish();
    }

    #[test]
    fn zooming_keeps_the_visible_centre_when_the_view_was_dragged() {
        let mut fx = session(&[2, 3]);
        fx.session.state.follow = false;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.state.map_offset = (90, 30);
        fx.session.sync_view();
        let before = visible_centre(&fx.session);

        fx.session.execute_action(AppAction::ZoomOut);
        std::thread::sleep(std::time::Duration::from_millis(300));
        fx.session.renderer.advance_zoom_animation();
        fx.session.sync_view();

        assert_close(visible_centre(&fx.session), before);
        assert_eq!(fx.session.state.map_offset, (0, 0));
        fx.finish();
    }

    #[test]
    fn panning_while_following_drags_the_chart_by_an_offset() {
        let mut fx = session(&[2]);
        fx.session.state.follow = true;
        fx.session.boat = valid_boat();
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
        fx.session.boat = valid_boat();
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
        assert_eq!(fx.session.state.zoom, 3, "top of the only chart: no change");
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
    fn choosing_a_chart_chooses_its_layer_and_an_unknown_entry_changes_nothing() {
        let mut fx =
            session_with_charts(&[("A_Navionics.mbtiles", &[2]), ("B_BingSat.mbtiles", &[2])]);
        fx.session.execute_action(AppAction::ChangeChart(1));
        assert_eq!(fx.session.state.layer.as_deref(), Some("Bing satellite"));
        assert!(fx.session.state.map.ends_with("B_BingSat.mbtiles"));
        assert!(fx.session.renderer.path().ends_with("B_BingSat.mbtiles"));

        fx.session.execute_action(AppAction::ChangeChart(7));
        assert_eq!(fx.session.state.layer.as_deref(), Some("Bing satellite"));
        fx.finish();
    }

    #[test]
    fn zooming_in_switches_to_the_more_detailed_file_of_the_layer_and_back() {
        let mut fx = session_with_charts(&[
            ("Ocean_Navionics.mbtiles", &[1, 2, 3]),
            ("Coast_Navionics.mbtiles", &[4, 5]),
        ]);
        fx.session.state.zoom = 3;
        fx.session.renderer.zoom_to(3);
        fx.session.sync_view();
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));

        assert_eq!(
            fx.session.renderer.beneath_path(),
            None,
            "Coast starts deeper"
        );
        fx.session.execute_action(AppAction::ZoomIn);
        assert_eq!(fx.session.state.zoom, 4);
        assert!(
            fx.session
                .renderer
                .beneath_path()
                .is_some_and(|p| p.ends_with("Ocean_Navionics.mbtiles")),
            "the ocean chart fills in beyond the coast chart"
        );
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Coast_Navionics.mbtiles"));
        assert!(fx.session.state.map.ends_with("Coast_Navionics.mbtiles"));

        fx.session.execute_action(AppAction::ZoomIn);
        fx.session.execute_action(AppAction::ZoomIn);
        assert_eq!(fx.session.state.zoom, 5, "no deeper than the deepest file");

        for _ in 0..2 {
            fx.session.execute_action(AppAction::ZoomOut);
        }
        assert_eq!(fx.session.state.zoom, 3);
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));
        fx.finish();
    }

    #[test]
    fn below_every_files_first_level_the_default_chart_shows_and_zooming_in_returns() {
        let mut fx = session_with_charts(&[
            ("World.mbtiles", &[0, 1]),
            ("Ocean_Navionics.mbtiles", &[3, 4]),
        ]);
        fx.session.state.layer = Some("Navionics".into());
        fx.session.state.zoom = 3;
        fx.session.renderer.zoom_to(3);
        fx.session.sync_view();
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));
        fx.session.execute_action(AppAction::ZoomOut);
        assert!(
            fx.session.renderer.path().ends_with("World.mbtiles"),
            "World is a chart in the default layer, the fallback"
        );
        fx.session.execute_action(AppAction::ZoomIn);
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));
        fx.finish();
    }

    #[test]
    fn a_chart_that_fails_to_open_is_logged_once_and_the_current_one_stays() {
        let mut fx = session_with_charts(&[
            ("Ocean_Navionics.mbtiles", &[1, 2]),
            ("Coast_Navionics.mbtiles", &[3]),
        ]);
        fx.session.state.zoom = 2;
        fx.session.renderer.zoom_to(2);
        fx.session.sync_view();
        std::fs::remove_file(fx.dir.join("charts/Coast_Navionics.mbtiles")).unwrap();

        fx.session.execute_action(AppAction::ZoomIn);
        assert_eq!(fx.session.state.zoom, 3);
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));
        fx.session.sync_view();
        assert!(fx
            .session
            .renderer
            .path()
            .ends_with("Ocean_Navionics.mbtiles"));
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
