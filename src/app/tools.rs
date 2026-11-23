//! The per-mode editing tools: what a click, a long press or a drag does to
//! the route, the waypoints or the measurement, given the current mode and
//! view. Implemented on `Session` so they are tested with a fixture chart and
//! temp files; `input.rs` only hands them gestures in screen pixels.

use sdl2::rect::Point;

use super::session::Session;
use crate::Mode;

/// Hit radius for points and handles, in pixels.
const POINT_RADIUS: i32 = 8;
/// Hit radius for the measurement's first mark, in pixels.
const MARK_RADIUS: i32 = 12;

/// What lies under a pixel in the current mode, and therefore what a drag
/// starting there grabs. Decided once at the press; the grab is then locked
/// for the whole drag, so dragging point A across point B keeps moving A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    RoutePoint(usize),
    /// The midpoint handle of leg `k` (between route points k and k+1).
    RouteHandle(usize),
    Waypoint(usize),
    MeasureMark,
    /// Nothing editable: a drag pans the chart.
    Chart,
}

/// A click that needs the shell's help: typing a waypoint name needs SDL
/// text input, so the tool asks for it instead of doing it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClickRequest {
    NameWaypoint((f64, f64)),
}

impl Session {
    /// What is under `pixel` in the current mode.
    pub fn target_at(&self, pixel: Point) -> Target {
        let view = self.renderer.view();
        match self.mode {
            Mode::EditRoute => {
                if let Some(i) = self.route.find_point_at(&view, pixel, POINT_RADIUS) {
                    Target::RoutePoint(i)
                } else if let Some(k) = self.route.find_midpoint_at(&view, pixel, POINT_RADIUS) {
                    Target::RouteHandle(k)
                } else {
                    Target::Chart
                }
            }
            Mode::EditWaypoint => match self.waypoints.find_point_at(&view, pixel, POINT_RADIUS) {
                Some(i) => Target::Waypoint(i),
                None => Target::Chart,
            },
            Mode::Measurement => {
                if self.mesure.find_mark1_at(&view, pixel, MARK_RADIUS) {
                    Target::MeasureMark
                } else {
                    Target::Chart
                }
            }
        }
    }

    /// A short click. Measurement places a mark; route editing adds a point
    /// (or inserts one on a handle) and saves; waypoint editing on empty
    /// chart asks the shell for a name. A pixel with no position is ignored.
    pub fn click(&mut self, pixel: Point) -> Option<ClickRequest> {
        let view = self.renderer.view();
        match self.mode {
            Mode::Measurement => {
                self.mesure.add_mark(&view, pixel);
                None
            }
            Mode::EditRoute => {
                match self.target_at(pixel) {
                    Target::RoutePoint(_) => {}
                    Target::RouteHandle(k) => {
                        let (lon, lat) = self.route.midpoints()[k];
                        self.route.insert(k, lon, lat);
                        self.save_route();
                    }
                    _ => {
                        if let Some((lon, lat)) = view.to_lonlat(pixel) {
                            self.route.add(lon, lat);
                            self.save_route();
                        }
                    }
                }
                None
            }
            Mode::EditWaypoint => match self.target_at(pixel) {
                Target::Waypoint(_) => None,
                _ => view.to_lonlat(pixel).map(ClickRequest::NameWaypoint),
            },
        }
    }

    /// A long press removes the route point or waypoint under it and saves.
    /// Returns true when something changed.
    pub fn long_press(&mut self, pixel: Point) -> bool {
        match self.target_at(pixel) {
            Target::RoutePoint(i) => {
                self.route.remove(i);
                self.save_route();
                true
            }
            Target::Waypoint(i) => {
                self.waypoints.remove(i);
                self.save_waypoints();
                true
            }
            _ => false,
        }
    }

    /// Moves the grabbed target to the position under `pixel`. The chart
    /// itself is dragged by the shell (it needs the texture), so `Chart` and
    /// handles are no-ops here. Returns true when something moved.
    pub fn drag_to(&mut self, target: Target, pixel: Point) -> bool {
        let view = self.renderer.view();
        match target {
            Target::RoutePoint(i) => {
                if let Some((lon, lat)) = view.to_lonlat(pixel) {
                    self.route.modify(i, lon, lat);
                }
                true
            }
            Target::Waypoint(i) => {
                if let Some(position) = view.to_lonlat(pixel) {
                    self.waypoints.modify(i, position);
                }
                true
            }
            Target::MeasureMark => {
                self.mesure.modify_mark1(&view, pixel);
                true
            }
            Target::RouteHandle(_) | Target::Chart => false,
        }
    }

    /// The drag is over: persist what it moved.
    pub fn drag_end(&mut self, target: Target) {
        match target {
            Target::RoutePoint(_) => self.save_route(),
            Target::Waypoint(_) => self.save_waypoints(),
            Target::MeasureMark | Target::RouteHandle(_) | Target::Chart => {}
        }
    }

    /// The shell collected a name for the waypoint a click asked about.
    pub fn name_waypoint(&mut self, position: (f64, f64), name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.waypoints.add(position, name.to_string());
        self.save_waypoints();
        crate::diagnostics::record("INFO", format!("Waypoint ajouté: {name}"));
    }

    fn save_route(&self) {
        if let Err(error) = self.route.save() {
            crate::diagnostics::record("ERROR", format!("Erreur sauvegarde route: {error}"));
        }
    }

    fn save_waypoints(&self) {
        if let Err(error) = self.waypoints.save() {
            crate::diagnostics::record(
                "ERROR",
                format!("Erreur sauvegarde des waypoints: {error}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::test_support::session;
    use super::*;
    use crate::route::Route;

    /// Three route points 10 degrees apart on the equator: about 28 px apart
    /// at zoom 2, so hit tests can tell them apart.
    fn route_session() -> super::super::session::test_support::Fixture {
        let mut fx = session(&[2]);
        fx.session.route = Route::from_points_at(
            vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)],
            &fx.dir.join("route.json"),
        );
        fx.session.state.position = Some((10.0, 0.0));
        fx.session.sync_view();
        fx
    }

    fn pixel_of(fx: &super::super::session::test_support::Fixture, position: (f64, f64)) -> Point {
        fx.session.renderer.view().to_pixel(position)
    }

    #[test]
    fn target_at_reports_what_is_under_the_pointer_in_each_mode() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditRoute;
        let p1 = pixel_of(&fx, (10.0, 0.0));
        assert_eq!(fx.session.target_at(p1), Target::RoutePoint(1));
        let handle = pixel_of(&fx, (5.0, 0.0));
        assert_eq!(fx.session.target_at(handle), Target::RouteHandle(0));
        assert_eq!(fx.session.target_at(Point::new(10, 200)), Target::Chart);

        fx.session.mode = Mode::EditWaypoint;
        fx.session.waypoints.add((10.0, 0.0), "A".to_string());
        assert_eq!(fx.session.target_at(p1), Target::Waypoint(0));

        fx.session.mode = Mode::Measurement;
        assert_eq!(fx.session.target_at(p1), Target::Chart, "no mark yet");
        fx.session.click(p1);
        assert_eq!(fx.session.target_at(p1), Target::MeasureMark);
        fx.finish();
    }

    #[test]
    fn a_click_on_the_chart_in_route_mode_adds_a_point_and_saves() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditRoute;
        assert_eq!(fx.session.click(Point::new(400, 128)), None);
        assert_eq!(fx.session.route.get_nb_wp(), 4);
        assert_eq!(
            crate::geo_data::read(&fx.dir.join("route.json"))
                .unwrap()
                .len(),
            4
        );
        fx.finish();
    }

    #[test]
    fn a_click_on_a_midpoint_handle_inserts_a_point_there() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditRoute;
        fx.session.click(pixel_of(&fx, (5.0, 0.0)));
        let saved = crate::geo_data::read(&fx.dir.join("route.json")).unwrap();
        assert_eq!(saved.len(), 4);
        assert!((saved[1].0 - 5.0).abs() < 1e-9, "{saved:?}");
        fx.finish();
    }

    #[test]
    fn a_long_press_on_a_route_point_removes_it_and_saves() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditRoute;
        assert!(fx.session.long_press(pixel_of(&fx, (10.0, 0.0))));
        assert_eq!(fx.session.route.get_nb_wp(), 2);
        assert_eq!(
            crate::geo_data::read(&fx.dir.join("route.json"))
                .unwrap()
                .len(),
            2
        );
        assert!(!fx.session.long_press(Point::new(10, 200)), "nothing there");
        fx.finish();
    }

    #[test]
    fn dragging_a_route_point_across_another_keeps_moving_the_grabbed_one() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditRoute;
        let grab = fx.session.target_at(pixel_of(&fx, (0.0, 0.0)));
        assert_eq!(grab, Target::RoutePoint(0));

        // Drag point 0 right across point 1 and a little beyond it.
        let mut over_p1 = pixel_of(&fx, (10.0, 0.0));
        over_p1.x += 2;
        assert!(fx.session.drag_to(grab, over_p1));
        fx.session.drag_end(grab);

        let saved = crate::geo_data::read(&fx.dir.join("route.json")).unwrap();
        assert_eq!(saved.len(), 3);
        assert!(saved[0].0 > 9.0, "point 0 moved near point 1: {saved:?}");
        assert!(
            (saved[1].0 - 10.0).abs() < 1e-9,
            "point 1 untouched: {saved:?}"
        );
        fx.finish();
    }

    #[test]
    fn two_clicks_in_measurement_mode_measure_the_distance() {
        let mut fx = route_session();
        fx.session.mode = Mode::Measurement;
        // At zoom 2 a pixel is about 0.18 degrees, so measure ten degrees of
        // latitude (600 nautical miles) and allow a pixel of rounding each end.
        fx.session.click(pixel_of(&fx, (10.0, 0.0)));
        fx.session.click(pixel_of(&fx, (10.0, 10.0)));
        let (distance, bearing) = fx.session.mesure.result().unwrap();
        assert!(
            (distance - 600.0).abs() < 25.0,
            "ten degrees of latitude: {distance}"
        );
        assert!(
            bearing.abs() < 3.0 || (bearing - 360.0).abs() < 3.0,
            "north: {bearing}"
        );
        fx.finish();
    }

    #[test]
    fn a_click_in_waypoint_mode_asks_for_a_name_and_naming_adds_the_waypoint() {
        let mut fx = route_session();
        fx.session.mode = Mode::EditWaypoint;
        let request = fx.session.click(Point::new(300, 100));
        let Some(ClickRequest::NameWaypoint(position)) = request else {
            panic!("expected a name request, got {request:?}");
        };
        fx.session.name_waypoint(position, "  Mooring ");
        fx.session.name_waypoint(position, "   ");
        assert_eq!(fx.session.waypoints.len(), 1, "blank names are refused");
        assert!(fx.dir.join("waypoints.json").exists());
        fx.finish();
    }
}
