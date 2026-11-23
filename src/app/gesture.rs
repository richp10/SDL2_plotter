//! Turns press, motion and release into gestures: a click, a long press, or
//! a drag with a locked origin. Pure and time-injected, so the 500 ms long
//! press and the move tolerance are tested without SDL. What a gesture
//! *means* (add a point, pan the chart) is decided elsewhere, by the mode's
//! tool on `Session`.

use std::time::{Duration, Instant};

use sdl2::rect::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Pressed and released without moving beyond the tolerance, before the
    /// long-press delay.
    Click(Point),
    /// Held still for the long-press delay. Fired once; the release after it
    /// is not a click.
    LongPress(Point),
    /// The pointer left the tolerance circle around the press. `origin` is
    /// where the press happened and is what gets grabbed.
    DragStart {
        origin: Point,
        at: Point,
    },
    DragMove {
        origin: Point,
        at: Point,
    },
    DragEnd {
        origin: Point,
        at: Point,
    },
}

struct Press {
    started: Instant,
    origin: Point,
    dragging: bool,
    long_fired: bool,
}

pub struct GestureRecognizer {
    long_press: Duration,
    tolerance: i32,
    press: Option<Press>,
}

impl GestureRecognizer {
    pub fn new(long_press: Duration, tolerance: i32) -> Self {
        GestureRecognizer {
            long_press,
            tolerance,
            press: None,
        }
    }

    pub fn press(&mut self, at: Point, now: Instant) {
        self.press = Some(Press {
            started: now,
            origin: at,
            dragging: false,
            long_fired: false,
        });
    }

    /// Pointer moved. Starts a drag once the tolerance is exceeded (unless a
    /// long press already fired), then reports every move of that drag.
    pub fn motion(&mut self, at: Point) -> Option<Gesture> {
        let press = self.press.as_mut()?;
        if press.dragging {
            return Some(Gesture::DragMove {
                origin: press.origin,
                at,
            });
        }
        if press.long_fired {
            return None;
        }
        let dx = at.x - press.origin.x;
        let dy = at.y - press.origin.y;
        if dx * dx + dy * dy > self.tolerance * self.tolerance {
            press.dragging = true;
            return Some(Gesture::DragStart {
                origin: press.origin,
                at,
            });
        }
        None
    }

    /// Button released: the end of a drag, a click, or nothing after a long
    /// press.
    pub fn release(&mut self, at: Point) -> Option<Gesture> {
        let press = self.press.take()?;
        if press.dragging {
            Some(Gesture::DragEnd {
                origin: press.origin,
                at,
            })
        } else if press.long_fired {
            None
        } else {
            Some(Gesture::Click(at))
        }
    }

    /// Called every loop iteration: fires the long press once the pointer
    /// has been held still long enough.
    pub fn tick(&mut self, now: Instant) -> Option<Gesture> {
        let press = self.press.as_mut()?;
        if press.dragging || press.long_fired || now.duration_since(press.started) < self.long_press
        {
            return None;
        }
        press.long_fired = true;
        Some(Gesture::LongPress(press.origin))
    }

    pub fn is_dragging(&self) -> bool {
        self.press.as_ref().is_some_and(|press| press.dragging)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_millis(500);

    fn recognizer() -> GestureRecognizer {
        GestureRecognizer::new(LONG, 5)
    }

    #[test]
    fn a_press_and_release_within_the_tolerance_is_a_click() {
        let mut g = recognizer();
        let t0 = Instant::now();
        g.press(Point::new(100, 100), t0);
        assert_eq!(
            g.motion(Point::new(103, 102)),
            None,
            "jitter inside the tolerance"
        );
        assert_eq!(
            g.release(Point::new(103, 102)),
            Some(Gesture::Click(Point::new(103, 102)))
        );
    }

    #[test]
    fn leaving_the_tolerance_starts_a_drag_from_the_press_origin() {
        let mut g = recognizer();
        g.press(Point::new(100, 100), Instant::now());
        assert_eq!(
            g.motion(Point::new(110, 100)),
            Some(Gesture::DragStart {
                origin: Point::new(100, 100),
                at: Point::new(110, 100)
            })
        );
        assert!(g.is_dragging());
        assert_eq!(
            g.motion(Point::new(120, 105)),
            Some(Gesture::DragMove {
                origin: Point::new(100, 100),
                at: Point::new(120, 105)
            })
        );
        assert_eq!(
            g.release(Point::new(121, 105)),
            Some(Gesture::DragEnd {
                origin: Point::new(100, 100),
                at: Point::new(121, 105)
            })
        );
        assert!(!g.is_dragging());
    }

    #[test]
    fn holding_still_fires_a_long_press_once_and_the_release_is_not_a_click() {
        let mut g = recognizer();
        let t0 = Instant::now();
        g.press(Point::new(50, 50), t0);
        assert_eq!(g.tick(t0 + Duration::from_millis(499)), None);
        assert_eq!(
            g.tick(t0 + Duration::from_millis(500)),
            Some(Gesture::LongPress(Point::new(50, 50)))
        );
        assert_eq!(g.tick(t0 + Duration::from_millis(900)), None, "fires once");
        assert_eq!(
            g.motion(Point::new(80, 80)),
            None,
            "no drag after a long press"
        );
        assert_eq!(g.release(Point::new(80, 80)), None);
    }

    #[test]
    fn a_drag_in_progress_suppresses_the_long_press() {
        let mut g = recognizer();
        let t0 = Instant::now();
        g.press(Point::new(0, 0), t0);
        g.motion(Point::new(20, 0));
        assert_eq!(g.tick(t0 + Duration::from_secs(2)), None);
    }

    #[test]
    fn motion_and_release_without_a_press_are_ignored() {
        let mut g = recognizer();
        assert_eq!(g.motion(Point::new(1, 1)), None);
        assert_eq!(g.release(Point::new(1, 1)), None);
        assert_eq!(g.tick(Instant::now()), None);
    }
}
