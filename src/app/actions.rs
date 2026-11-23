use std::path::Path;

use chrono::Local;

use super::confirm_dialog::ConfirmAction;
use super::{AppAction, Plotter, DEFAULT_TRACK_PATH};
use crate::Mode;

impl Plotter<'_> {
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
                self.confirm_dialog.request(ConfirmAction::ClearRoute);
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
                self.confirm_dialog.request(ConfirmAction::ClearTrack);
            }
            AppAction::ExportRouteGpx => {
                let now = Local::now();
                let path = Path::new("export")
                    .join(format!("Route-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.route.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record("INFO", format!("Route exportée: {}", path.display())),
                    Err(error) => {
                        crate::diagnostics::record("ERROR", format!("Erreur export route GPX: {error}"));
                    }
                }
            }
            AppAction::ExportTrackGpx => {
                let now = Local::now();
                let path = Path::new("export")
                    .join(format!("Track-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.track.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record("INFO", format!("Trace exportée: {}", path.display())),
                    Err(error) => {
                        crate::diagnostics::record("ERROR", format!("Erreur export trace GPX: {error}"));
                    }
                }
            }
            AppAction::ClearWaypoints => {
                self.confirm_dialog
                    .request(ConfirmAction::ClearWaypoints);
            }
            AppAction::ExportWaypointsGpx => {
                let now = Local::now();
                let path = Path::new("export")
                    .join(format!("Waypoints-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.waypoints.export_gpx(&path) {
                    Ok(()) => crate::diagnostics::record("INFO", format!("Waypoints exportés: {}", path.display())),
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
                            format!("Impossible d'ouvrir {}: {error}", &self.charts_path[index]),
                        );
                    }
                }
            }
            AppAction::Pan(dx, dy) => {
                if self.state.follow {
                    self.state.map_offset.0 += dx;
                    self.state.map_offset.1 += dy;
                } else {
                    self.state.position = Some(self.renderer.screen_to_gps(dx, dy));
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

        self.dirty = true;
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
                self.track
                    .save_default_track(Path::new(DEFAULT_TRACK_PATH));
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
        self.dirty = true;
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
