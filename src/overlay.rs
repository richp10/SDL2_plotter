//! What the data modules want drawn over the chart, and how it becomes pixel
//! commands. A data module emits `Shape`s in chart coordinates and knows
//! nothing about pixels or SDL; `plan` projects them through the `ChartView`,
//! culls what is off screen and decides the style kind; `Screen::paint`
//! executes the resulting `Draw` list. Culling and styling therefore happen
//! in one place, and the plan is tested without a window.

use sdl2::rect::Point;

use crate::mbtiles_renderer::ChartView;

/// Pixels beyond the screen edge within which a marker is still drawn, so a
/// symbol straddling the edge is not cut in half by the cull.
const MARKER_MARGIN: i32 = 32;

/// Something to draw, in chart coordinates `(lon, lat)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Route legs with a dot at every point.
    Route(Vec<(f64, f64)>),
    /// Midpoint handles shown while editing the route.
    RouteHandles(Vec<(f64, f64)>),
    /// The boat's track, in normalised Mercator coordinates (0..1) as the
    /// track stores them.
    Track(Vec<(f64, f64)>),
    /// Own boat; `bow`, `course` and `heading` are true, in radians. `bow`
    /// is where the hull points (`None`: no direction known, a round
    /// marker). `fix` is false while the position is only the last one
    /// known.
    Boat {
        at: (f64, f64),
        bow: Option<f64>,
        course: Option<f64>,
        heading: Option<f64>,
        fix: bool,
    },
    /// An AIS target with its label; `course` and `heading` are true, in
    /// radians.
    Ais {
        at: (f64, f64),
        course: Option<f64>,
        heading: Option<f64>,
        label: String,
        style: AisStyle,
    },
    /// An AIS target inside the collision limits; `course` is true, in
    /// radians. `uncertain`: close by, with no motion data.
    Collision {
        at: (f64, f64),
        course: Option<f64>,
        label: String,
        uncertain: bool,
    },
    Waypoint {
        at: (f64, f64),
        label: String,
    },
    /// A measurement mark.
    Mark((f64, f64)),
    /// The measurement line between the two marks.
    MeasureLine((f64, f64), (f64, f64)),
    /// A closed outline through positions: a chart's coverage.
    Outline(Vec<(f64, f64)>),
    /// Free text anchored at a position, offset in pixels.
    Label {
        at: (f64, f64),
        text: String,
        offset: (i32, i32),
    },
}

/// A drawing command in screen pixels, already culled. Courses are screen
/// angles (course-up rotation applied).
#[derive(Debug, Clone, PartialEq)]
pub enum Draw {
    Route(Vec<Point>),
    Handles(Vec<Point>),
    Track(Vec<Point>),
    Boat {
        at: Point,
        bow: Option<f64>,
        course: Option<f64>,
        heading: Option<f64>,
        fix: bool,
    },
    Ais {
        at: Point,
        course: Option<f64>,
        heading: Option<f64>,
        style: AisStyle,
    },
    Collision {
        at: Point,
        course: Option<f64>,
        uncertain: bool,
    },
    Waypoint(Point),
    Mark(Point),
    Line(Point, Point),
    Outline(Vec<Point>),
    Text {
        at: Point,
        text: String,
    },
    /// Small text beside a marker (an AIS target's name).
    Label {
        at: Point,
        text: String,
    },
}

/// Projects and culls `shapes` for `view`. Polylines are kept whole (the
/// painter clips their segments); markers and their labels are dropped
/// together when the marker is more than `MARKER_MARGIN` pixels off screen.
pub fn plan(view: &ChartView, shapes: &[Shape]) -> Vec<Draw> {
    let mut draws = Vec::new();
    let visible = |p: Point| view.contains(p, MARKER_MARGIN);
    for shape in shapes {
        match shape {
            Shape::Route(points) => {
                if !points.is_empty() {
                    draws.push(Draw::Route(
                        points.iter().map(|&p| view.to_pixel(p)).collect(),
                    ));
                }
            }
            Shape::RouteHandles(points) => {
                let handles: Vec<Point> = points
                    .iter()
                    .map(|&p| view.to_pixel(p))
                    .filter(|&p| visible(p))
                    .collect();
                if !handles.is_empty() {
                    draws.push(Draw::Handles(handles));
                }
            }
            Shape::Track(norm) => {
                if norm.len() >= 2 {
                    draws.push(Draw::Track(
                        norm.iter()
                            .map(|&(nx, ny)| view.norm_to_pixel(nx, ny))
                            .collect(),
                    ));
                }
            }
            Shape::Boat {
                at,
                bow,
                course,
                heading,
                fix,
            } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Boat {
                        at,
                        bow: bow
                            .filter(|b| b.is_finite())
                            .map(|b| view.course_to_screen(b)),
                        course: course.map(|c| view.course_to_screen(c)),
                        heading: heading
                            .filter(|h| h.is_finite())
                            .map(|h| view.course_to_screen(h)),
                        fix: *fix,
                    });
                }
            }
            Shape::Ais {
                at,
                course,
                heading,
                label,
                style,
            } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    let screen = |angle: &Option<f64>| {
                        angle
                            .filter(|a| a.is_finite())
                            .map(|a| view.course_to_screen(a))
                    };
                    draws.push(Draw::Ais {
                        at,
                        course: screen(course),
                        heading: screen(heading),
                        style: *style,
                    });
                    // Clear of the hull (twice `ais_outline_size` from the
                    // centre by default).
                    draws.push(Draw::Label {
                        at: Point::new(at.x + 22, at.y - 8),
                        text: label.clone(),
                    });
                }
            }
            Shape::Collision {
                at,
                course,
                label,
                uncertain,
            } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Collision {
                        at,
                        course: course
                            .filter(|c| c.is_finite())
                            .map(|c| view.course_to_screen(c)),
                        uncertain: *uncertain,
                    });
                    draws.push(Draw::Text {
                        at: Point::new(at.x + 10, at.y - 10),
                        text: label.clone(),
                    });
                }
            }
            Shape::Waypoint { at, label } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Waypoint(at));
                    draws.push(Draw::Text {
                        at: Point::new(at.x + 10, at.y - 12),
                        text: label.clone(),
                    });
                }
            }
            Shape::Mark(at) => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Mark(at));
                }
            }
            Shape::Outline(points) => {
                if points.len() >= 2 {
                    draws.push(Draw::Outline(
                        points.iter().map(|&p| view.to_pixel(p)).collect(),
                    ));
                }
            }
            Shape::MeasureLine(a, b) => {
                draws.push(Draw::Line(view.to_pixel(*a), view.to_pixel(*b)));
            }
            Shape::Label { at, text, offset } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Text {
                        at: Point::new(at.x + offset.0, at.y + offset.1),
                        text: text.clone(),
                    });
                }
            }
        }
    }
    draws
}

/// The colour group of an AIS target, from its ship type: what it is and,
/// roughly, how large.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    Cargo,
    Tanker,
    Passenger,
    HighSpeed,
    /// Pilot, search and rescue, tug, port tender, law enforcement.
    Service,
    Fishing,
    /// Sailing and pleasure craft.
    Leisure,
    /// Any other type, or none known yet.
    #[default]
    Other,
}

impl Category {
    pub fn of(ship_type: Option<u8>) -> Self {
        match ship_type {
            Some(70..=79) => Self::Cargo,
            Some(80..=89) => Self::Tanker,
            Some(60..=69) => Self::Passenger,
            Some(40..=49) => Self::HighSpeed,
            Some(50..=59) => Self::Service,
            Some(30) => Self::Fishing,
            Some(36 | 37) => Self::Leisure,
            _ => Self::Other,
        }
    }
}

/// The day shape a vessel shows by COLREGs for its navigational status,
/// drawn beside the target as OpenCPN does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// At anchor or moored.
    Ball,
    /// Not under command.
    TwoBalls,
    /// Aground.
    ThreeBalls,
    /// Restricted in her ability to manoeuvre.
    BallDiamondBall,
    /// Constrained by her draught.
    Cylinder,
    /// Engaged in fishing: two cones, points together.
    Fishing,
}

impl Mark {
    /// The mark for an AIS navigational status; none under way.
    pub fn of(nav_status: Option<u8>) -> Option<Self> {
        Some(match nav_status? {
            1 | 5 => Self::Ball,
            2 => Self::TwoBalls,
            3 => Self::BallDiamondBall,
            4 => Self::Cylinder,
            6 => Self::ThreeBalls,
            7 => Self::Fishing,
            _ => return None,
        })
    }
}

/// How an AIS target is drawn, apart from where: its colour group, its
/// day-shape mark and its class (a class B hull has a flat stern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AisStyle {
    pub category: Category,
    pub mark: Option<Mark>,
    pub class_b: bool,
}

/// The outline of a hull around `at`: the bow first, then the other
/// corners. `angle` is a screen angle (clockwise from up) and
/// `half_length` the distance from the centre to the bow, in pixels. A
/// flat stern (class B, and the own boat) has five corners; a class A
/// stern has a notch in the middle.
pub fn ais_hull(at: Point, angle: f64, half_length: f64, flat_stern: bool) -> Vec<Point> {
    // Bow, shoulders and stern corners, with the bow towards -y.
    let mut hull = vec![(0.0, -1.0), (0.45, -0.3), (0.4, 0.9)];
    if !flat_stern {
        hull.push((0.0, 0.6));
    }
    hull.extend([(-0.4, 0.9), (-0.45, -0.3)]);
    let (sin, cos) = angle.sin_cos();
    hull.into_iter()
        .map(|(x, y)| {
            let (x, y) = (x * half_length, y * half_length);
            Point::new(
                at.x + (x * cos - y * sin).round() as i32,
                at.y + (x * sin + y * cos).round() as i32,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ChartView {
        // 512x256 on the equator at zoom 4: about 11 px per degree.
        ChartView::centred_on((0.0, 0.0), 4.0, 0.0, 512, 256)
    }

    #[test]
    fn markers_on_screen_are_projected_and_markers_far_off_screen_are_culled() {
        let draws = plan(
            &view(),
            &[
                Shape::Boat {
                    at: (0.0, 0.0),
                    bow: None,
                    course: None,
                    heading: None,
                    fix: true,
                },
                Shape::Boat {
                    at: (90.0, 0.0),
                    bow: None,
                    course: None,
                    heading: None,
                    fix: true,
                },
                Shape::Mark((1.0, 1.0)),
                Shape::Mark((-120.0, 0.0)),
            ],
        );
        assert_eq!(draws.len(), 2, "{draws:?}");
        assert_eq!(
            draws[0],
            Draw::Boat {
                at: Point::new(256, 128),
                bow: None,
                course: None,
                heading: None,
                fix: true
            }
        );
        assert!(matches!(draws[1], Draw::Mark(_)));
    }

    #[test]
    fn the_ais_heading_is_rotated_into_a_screen_angle() {
        let rotated = ChartView::centred_on((0.0, 0.0), 4.0, 1.0, 512, 256);
        let draws = plan(
            &rotated,
            &[Shape::Ais {
                at: (0.0, 0.0),
                course: None,
                heading: Some(2.0),
                label: "Ship".into(),
                style: AisStyle::default(),
            }],
        );
        let Draw::Ais {
            heading: Some(heading),
            ..
        } = &draws[0]
        else {
            panic!("{draws:?}");
        };
        assert!((heading - rotated.course_to_screen(2.0)).abs() < 1e-12);
    }

    #[test]
    fn a_class_a_hull_has_a_notched_stern_and_a_class_b_hull_a_flat_one() {
        let centre = Point::new(100, 100);
        let a = ais_hull(centre, 0.0, 10.0, false);
        let b = ais_hull(centre, 0.0, 10.0, true);
        assert_eq!((a.len(), b.len()), (6, 5));
        let deepest_stern = |hull: &[Point]| hull.iter().map(|p| p.y).max().unwrap();
        assert_eq!(deepest_stern(&a), deepest_stern(&b), "same length");
        assert!(
            a.iter()
                .any(|p| p.x == 100 && p.y > 100 && p.y < deepest_stern(&a)),
            "a notch in the middle of the class A stern: {a:?}"
        );
    }

    #[test]
    fn the_ais_hull_points_its_bow_along_the_direction() {
        let centre = Point::new(100, 100);
        let north = ais_hull(centre, 0.0, 10.0, true);
        assert_eq!(north[0], Point::new(100, 90), "bow up");
        assert!(north.iter().skip(1).all(|p| p.y > 90));
        let east = ais_hull(centre, std::f64::consts::FRAC_PI_2, 10.0, true);
        assert_eq!(east[0], Point::new(110, 100), "bow right");
        assert!(east.iter().skip(1).all(|p| p.x < 110));
    }

    #[test]
    fn a_boat_without_a_fix_is_drawn_with_no_vectors() {
        let draws = plan(
            &view(),
            &[Shape::Boat {
                at: (0.0, 0.0),
                bow: None,
                course: None,
                heading: None,
                fix: false,
            }],
        );
        assert_eq!(
            draws,
            vec![Draw::Boat {
                at: Point::new(256, 128),
                bow: None,
                course: None,
                heading: None,
                fix: false
            }]
        );
    }

    #[test]
    fn the_heading_is_rotated_like_the_course() {
        let rotated = ChartView::centred_on((0.0, 0.0), 4.0, 1.0, 512, 256);
        let draws = plan(
            &rotated,
            &[Shape::Boat {
                at: (0.0, 0.0),
                bow: Some(std::f64::consts::FRAC_PI_2),
                course: None,
                heading: Some(std::f64::consts::FRAC_PI_2),
                fix: true,
            }],
        );
        let Draw::Boat {
            heading: Some(heading),
            ..
        } = &draws[0]
        else {
            panic!("{draws:?}");
        };
        assert!((heading - rotated.course_to_screen(std::f64::consts::FRAC_PI_2)).abs() < 1e-12);
    }

    #[test]
    fn a_label_is_dropped_together_with_its_marker() {
        let draws = plan(
            &view(),
            &[
                Shape::Waypoint {
                    at: (0.0, 0.0),
                    label: "Here".into(),
                },
                Shape::Waypoint {
                    at: (100.0, 0.0),
                    label: "Gone".into(),
                },
                Shape::Ais {
                    at: (1.0, 0.0),
                    course: Some(f64::NAN),
                    heading: None,
                    label: "Ship".into(),
                    style: AisStyle::default(),
                },
            ],
        );
        let texts: Vec<&str> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Text { text, .. } | Draw::Label { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Here", "Ship"]);
        assert!(
            draws
                .iter()
                .any(|d| matches!(d, Draw::Label { text, .. } if text == "Ship")),
            "an AIS name is a small label"
        );
        assert!(
            matches!(draws[2], Draw::Ais { course: None, .. }),
            "a non-finite course is dropped: {draws:?}"
        );
    }

    #[test]
    fn courses_are_rotated_into_screen_angles() {
        let rotated = ChartView::centred_on((0.0, 0.0), 4.0, std::f64::consts::FRAC_PI_2, 512, 256);
        let draws = plan(
            &rotated,
            &[Shape::Boat {
                at: (0.0, 0.0),
                bow: None,
                course: Some(std::f64::consts::FRAC_PI_2),
                heading: None,
                fix: true,
            }],
        );
        let Draw::Boat {
            course: Some(course),
            ..
        } = &draws[0]
        else {
            panic!("{draws:?}");
        };
        assert!(course.abs() < 1e-12, "east on a course-up view points up");
    }

    #[test]
    fn polylines_are_kept_whole_even_when_partly_off_screen() {
        let draws = plan(
            &view(),
            &[
                Shape::Route(vec![(0.0, 0.0), (120.0, 0.0)]),
                Shape::Track(vec![(0.5, 0.5), (0.9, 0.5)]),
                Shape::MeasureLine((0.0, 0.0), (150.0, 10.0)),
            ],
        );
        assert_eq!(draws.len(), 3, "{draws:?}");
        assert!(matches!(&draws[0], Draw::Route(points) if points.len() == 2));
        assert!(matches!(&draws[1], Draw::Track(points) if points[0] == Point::new(256, 128)));
        assert!(matches!(draws[2], Draw::Line(..)));
    }

    #[test]
    fn an_outline_is_projected_whole() {
        let draws = plan(
            &view(),
            &[Shape::Outline(vec![
                (-1.0, 1.0),
                (1.0, 1.0),
                (1.0, -1.0),
                (90.0, -1.0),
            ])],
        );
        let Draw::Outline(points) = &draws[0] else {
            panic!("{draws:?}");
        };
        assert_eq!(points.len(), 4, "the off-screen corner too");
    }

    #[test]
    fn empty_shapes_produce_nothing() {
        let draws = plan(
            &view(),
            &[
                Shape::Route(vec![]),
                Shape::Track(vec![(0.5, 0.5)]),
                Shape::RouteHandles(vec![(100.0, 0.0)]),
            ],
        );
        assert!(draws.is_empty(), "{draws:?}");
    }
}
