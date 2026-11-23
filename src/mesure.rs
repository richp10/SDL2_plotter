use crate::gps;
use crate::mbtiles_renderer::ChartView;
use crate::overlay::Shape;
use sdl2::rect::Point;

pub struct Mesure {
    mark1: Option<(f64, f64)>,
    mark2: Option<(f64, f64)>,
    distance: f64,
    bearing: f64,
}

impl Mesure {
    pub fn new() -> Self {
        Mesure {
            mark1: None,
            mark2: None,
            distance: 0.0,
            bearing: 0.0,
        }
    }

    /// Places the first mark, or places/moves the second one, at the chart
    /// position under `pixel`. A pixel with no position (beyond the poles)
    /// is ignored.
    pub fn add_mark(&mut self, view: &ChartView, pixel: Point) {
        let Some(position) = view.to_lonlat(pixel) else {
            return;
        };
        match self.mark1 {
            // Étape 1 : rien de posé -> premier point
            None => {
                self.mark1 = Some(position);
                self.mark2 = None;
                (self.distance, self.bearing) = (0.0, 0.0);
            }
            // Étapes suivantes : poser ou déplacer le point d'arrivée
            Some(mark1) => {
                self.mark2 = Some(position);
                (self.distance, self.bearing) = gps::distance_bearing_nm(mark1, position);
            }
        }
    }

    /// Distance (nautical miles) and bearing (degrees) once both marks are
    /// placed.
    #[cfg(test)]
    pub fn result(&self) -> Option<(f64, f64)> {
        self.mark2.map(|_| (self.distance, self.bearing))
    }

    pub fn clear(&mut self) {
        self.mark1 = None;
        self.mark2 = None;
        (self.distance, self.bearing) = (0.0, 0.0);
    }

    /// True when `pixel` is within `tolerance` pixels of the first mark.
    pub fn find_mark1_at(&self, view: &ChartView, pixel: Point, tolerance: i32) -> bool {
        view.nearest(self.mark1, pixel, tolerance).is_some()
    }

    /// Moves the first mark to the chart position under `pixel`.
    pub fn modify_mark1(&mut self, view: &ChartView, pixel: Point) {
        let Some(mark1) = view.to_lonlat(pixel) else {
            return;
        };
        self.mark1 = Some(mark1);
        (self.distance, self.bearing) = match self.mark2 {
            Some(mark2) => gps::distance_bearing_nm(mark1, mark2),
            None => (0.0, 0.0),
        };
    }

    /// The marks, the line between them and the readout at the second mark.
    pub fn shapes(&self) -> Vec<Shape> {
        let mut shapes = Vec::new();
        let Some(mark1) = self.mark1 else {
            return shapes;
        };
        shapes.push(Shape::Mark(mark1));
        if let Some(mark2) = self.mark2 {
            shapes.push(Shape::Mark(mark2));
            shapes.push(Shape::MeasureLine(mark1, mark2));
            let text = if self.distance >= 1.0 {
                format!("{:.1} Mn {:.0}°", self.distance, self.bearing)
            } else {
                format!("{:.0} m {:.0}°", self.distance * 1852.0, self.bearing)
            };
            shapes.push(Shape::Label {
                at: mark2,
                text,
                offset: (0, 0),
            });
        }
        shapes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_marks_give_two_marks_a_line_and_a_readout_in_nautical_miles() {
        let view = ChartView::centred_on((0.0, 0.0), 6.0, 0.0, 512, 256);
        let mut mesure = Mesure::new();
        assert!(mesure.shapes().is_empty());
        mesure.add_mark(&view, view.to_pixel((0.0, 0.0)));
        assert_eq!(mesure.shapes(), vec![Shape::Mark((0.0, 0.0))]);

        mesure.add_mark(&view, view.to_pixel((0.0, 1.0)));
        let shapes = mesure.shapes();
        assert_eq!(shapes.len(), 4, "{shapes:?}");
        assert!(matches!(shapes[2], Shape::MeasureLine(..)));
        let Shape::Label { text, .. } = &shapes[3] else {
            panic!("{shapes:?}");
        };
        assert!(
            text.ends_with("Mn 0°") || text.ends_with("Mn 360°"),
            "{text}"
        );
    }
}
