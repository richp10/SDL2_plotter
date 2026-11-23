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
    /// Own boat; `course` is a true course in radians, shown when moving.
    Boat {
        at: (f64, f64),
        course: Option<f64>,
    },
    /// An AIS target with its label; `course` is a true course in radians.
    Ais {
        at: (f64, f64),
        course: Option<f64>,
        label: String,
    },
    Waypoint {
        at: (f64, f64),
        label: String,
    },
    /// A measurement mark.
    Mark((f64, f64)),
    /// The measurement line between the two marks.
    MeasureLine((f64, f64), (f64, f64)),
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
    Boat { at: Point, course: Option<f64> },
    Ais { at: Point, course: Option<f64> },
    Waypoint(Point),
    Mark(Point),
    Line(Point, Point),
    Text { at: Point, text: String },
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
            Shape::Boat { at, course } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Boat {
                        at,
                        course: course.map(|c| view.course_to_screen(c)),
                    });
                }
            }
            Shape::Ais { at, course, label } => {
                let at = view.to_pixel(*at);
                if visible(at) {
                    draws.push(Draw::Ais {
                        at,
                        course: course
                            .filter(|c| c.is_finite())
                            .map(|c| view.course_to_screen(c)),
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
                    course: None,
                },
                Shape::Boat {
                    at: (90.0, 0.0),
                    course: None,
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
                course: None
            }
        );
        assert!(matches!(draws[1], Draw::Mark(_)));
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
                    label: "Ship".into(),
                },
            ],
        );
        let texts: Vec<&str> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Here", "Ship"]);
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
                course: Some(std::f64::consts::FRAC_PI_2),
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
