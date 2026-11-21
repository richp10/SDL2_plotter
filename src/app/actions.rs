use std::path::Path;

use chrono::Local;

use super::{AppAction, Plotter, DEFAULT_TRACK_DIR};
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
            AppAction::ToggleGoto => {
                self.state.goto = !self.state.goto;
                self.state.nav_id = 0;
            }
            AppAction::ClearRoute => {
                self.state.goto = false;
                self.route.clear();
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
                self.track.reset();
            }
            AppAction::SaveTrack => {
                let now = Local::now();
                let path = Path::new(DEFAULT_TRACK_DIR)
                    .join(format!("Track-{}.gpx", now.format("%Y-%m-%d_%H-%M-%S")));
                match self.track.save(&path) {
                    Ok(()) => println!("Trace sauvegardée : {}", path.display()),
                    Err(error) => eprintln!("Erreur sauvegarde : {error}"),
                }
            }
            AppAction::ClearWaypoints => self.waypoints.reset(),
            AppAction::SaveWaypoints => {
                if let Err(error) = self.waypoints.save() {
                    eprintln!("Erreur sauvegarde des waypoints : {error}");
                }
            }
            AppAction::ChangeChart(index) => {
                match self.renderer.change_map(&self.charts_path[index]) {
                    Ok(()) => {
                        self.state.map = self.charts_path[index].clone();
                        self.state.map_offset = (0, 0);
                    }
                    Err(error) => {
                        eprintln!("Impossible d'ouvrir {} : {error}", &self.charts_path[index])
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
                self.state.zoom = self.renderer.zoom_increase(&self.state.map);
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
