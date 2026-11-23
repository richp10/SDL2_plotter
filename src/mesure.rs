use crate::gps;
use crate::mbtiles_renderer::{ChartView, MBTilesRenderer};
use crate::screen::Screen;
use sdl2::rect::Point;

pub struct Mesure {
    mark1: Option<(f64, f64)>,
    mark2: Option<(f64, f64)>,
    distance: f64,
    bearing: f64,
}

impl Mesure {
    pub fn new() -> Result<Self, String> {
        Ok(Mesure {
            mark1: None,
            mark2: None,
            distance: 0.0,
            bearing: 0.0,
        })
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

    pub fn show(&mut self, renderer: &MBTilesRenderer, screen: &mut Screen) {
        let s1 = self.get_pt1(renderer);
        let s2 = self.get_pt2(renderer);
        // les points
        for (x, y) in [s1, s2].into_iter().flatten() {
            let _ = screen.show_mark(x, y);
        }
        // le trait, seulement si les deux existent
        if let (Some((x1, y1)), Some((x2, y2))) = (s1, s2) {
            let _ = screen.show_line(x1, y1, x2, y2);
        }
        let text = if self.distance >= 1.0 {
            format!("{:.1} Mn {:.0}°", self.distance, self.bearing)
        } else {
            format!("{:.0} m {:.0}°", self.distance * 1852.0, self.bearing)
        };
        if let Some((x, y)) = s2 {
            let _ = screen.show_text(&text, x, y);
        }
    }

    fn get_pt1(&self, renderer: &MBTilesRenderer) -> Option<(i32, i32)> {
        let pt = renderer.gps_to_point(self.mark1?);
        Some((pt.x, pt.y))
    }

    fn get_pt2(&self, renderer: &MBTilesRenderer) -> Option<(i32, i32)> {
        let pt = renderer.gps_to_point(self.mark2?);
        Some((pt.x, pt.y))
    }
}
