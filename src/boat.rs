use std::time::{Duration, Instant};

use crate::overlay::Shape;

/// Below this speed over ground the COG vector is not drawn: 1.0 m/s,
/// about 1.9 kn. Under it, GPS course is mostly noise.
const MINIMUM_SPEED: f64 = 1.0;

/// No position for this long: the fix is lost.
pub const FIX_TIMEOUT: Duration = Duration::from_secs(10);
/// No heading for this long: the heading is lost.
pub const HEADING_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
pub struct BoatState {
    pub longitude: f64,
    pub latitude: f64,
    pub position_valid: bool,
    pub speed_over_ground: f64,
    pub course_over_ground: f64,
    /// True heading in radians, when a source gives one.
    pub heading: Option<f64>,
    /// When the source last received a valid position. `None` until the
    /// first fix.
    pub fixed_at: Option<Instant>,
    /// When the source last received a heading.
    pub heading_at: Option<Instant>,
}

impl BoatState {
    pub fn default() -> Self {
        BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: false,
            speed_over_ground: 0.0,
            course_over_ground: 0.0,
            heading: None,
            fixed_at: None,
            heading_at: None,
        }
    }

    /// The boat marker at the last position that had a fix, with its course
    /// vector when under way and its heading line, both only while the fix
    /// is valid. Nothing before the first fix. Where it falls on screen is
    /// the painter's business.
    pub fn shape(&self) -> Option<Shape> {
        self.fixed_at?;
        let fix = self.position_valid;
        Some(Shape::Boat {
            at: (self.longitude, self.latitude),
            course: (fix && self.speed_over_ground > MINIMUM_SPEED)
                .then_some(self.course_over_ground),
            heading: self.heading.filter(|_| fix),
            fix,
        })
    }

    /// This state as it is at `now`: the fix is lost when no position came
    /// for `FIX_TIMEOUT`, the heading when none came for `HEADING_TIMEOUT`.
    /// The last position stays, for the grey marker.
    pub fn expired(self, now: Instant) -> BoatState {
        let fresh = |at: Option<Instant>, timeout: Duration| {
            at.is_some_and(|at| now.saturating_duration_since(at) < timeout)
        };
        BoatState {
            position_valid: self.position_valid && fresh(self.fixed_at, FIX_TIMEOUT),
            heading: self
                .heading
                .filter(|_| fresh(self.heading_at, HEADING_TIMEOUT)),
            ..self
        }
    }

    pub fn position(self) -> (f64, f64) {
        (self.longitude, self.latitude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn fixed(now: Instant) -> BoatState {
        BoatState {
            longitude: -4.5,
            latitude: 48.4,
            position_valid: true,
            speed_over_ground: 5.0,
            course_over_ground: 1.0,
            heading: Some(0.5),
            fixed_at: Some(now),
            heading_at: Some(now),
        }
    }

    #[test]
    fn a_fix_shows_with_course_and_heading() {
        let boat = fixed(Instant::now());
        assert_eq!(
            boat.shape(),
            Some(Shape::Boat {
                at: (-4.5, 48.4),
                course: Some(1.0),
                heading: Some(0.5),
                fix: true
            })
        );
    }

    #[test]
    fn the_course_shows_only_when_under_way() {
        let mut boat = fixed(Instant::now());
        boat.speed_over_ground = 0.2;
        assert!(matches!(
            boat.shape(),
            Some(Shape::Boat { course: None, .. })
        ));
    }

    #[test]
    fn a_lost_fix_shows_at_the_last_position_with_no_vectors() {
        let mut boat = fixed(Instant::now());
        boat.position_valid = false;
        assert_eq!(
            boat.shape(),
            Some(Shape::Boat {
                at: (-4.5, 48.4),
                course: None,
                heading: None,
                fix: false
            })
        );
    }

    #[test]
    fn no_marker_before_the_first_fix() {
        assert_eq!(BoatState::default().shape(), None);
    }

    #[test]
    fn the_fix_and_the_heading_go_stale_after_ten_seconds() {
        let now = Instant::now();
        let boat = fixed(now);
        let just_in = boat.expired(now + Duration::from_millis(9_900));
        assert!(just_in.position_valid);
        assert_eq!(just_in.heading, Some(0.5));
        let stale = boat.expired(now + Duration::from_secs(10));
        assert!(!stale.position_valid);
        assert_eq!(stale.heading, None);
        assert_eq!(stale.position(), (-4.5, 48.4), "the last position is kept");
    }

    #[test]
    fn the_heading_goes_stale_on_its_own() {
        let now = Instant::now();
        let mut boat = fixed(now + Duration::from_secs(8));
        boat.heading_at = Some(now);
        let later = boat.expired(now + Duration::from_secs(11));
        assert!(later.position_valid);
        assert_eq!(later.heading, None);
    }
}
