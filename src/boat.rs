use crate::overlay::Shape;

const MINIMUM_SPEED: f64 = 1.0;

#[derive(Clone, Copy, Debug)]
pub struct BoatState {
    pub longitude: f64,
    pub latitude: f64,
    pub position_valid: bool,
    pub speed_over_ground: f64,
    pub course_over_ground: f64,
}

impl BoatState {
    pub fn default() -> Self {
        BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: false,
            speed_over_ground: 0.0,
            course_over_ground: 0.0,
        }
    }

    /// The boat marker with its course vector when under way; nothing while
    /// the fix is invalid. Where it falls on screen is the painter's business.
    pub fn shape(&self) -> Option<Shape> {
        self.position_valid.then(|| Shape::Boat {
            at: (self.longitude, self.latitude),
            course: (self.speed_over_ground > MINIMUM_SPEED).then_some(self.course_over_ground),
        })
    }

    pub fn position(self) -> (f64, f64) {
        (self.longitude, self.latitude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boat_shows_its_course_only_when_under_way_and_nothing_without_a_fix() {
        let mut boat = BoatState {
            longitude: -4.5,
            latitude: 48.4,
            position_valid: true,
            speed_over_ground: 0.2,
            course_over_ground: 1.0,
        };
        assert_eq!(
            boat.shape(),
            Some(Shape::Boat {
                at: (-4.5, 48.4),
                course: None
            })
        );
        boat.speed_over_ground = 5.0;
        assert!(matches!(boat.shape(), Some(Shape::Boat { course: Some(c), .. }) if c == 1.0));
        boat.position_valid = false;
        assert_eq!(boat.shape(), None);
    }
}
