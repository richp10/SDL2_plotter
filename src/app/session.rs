//! Everything the navigator's actions act on, with no SDL handle in it:
//! the chart view, the saved state, the route, track, waypoints and
//! measurement, the boat, and the confirmation that may be pending. The
//! action layer (`actions.rs`) is implemented on this type, so it can be
//! driven by tests with a fixture chart and temporary paths. `Plotter` owns
//! one `Session` plus the SDL widgets and channels around it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::confirm_dialog::Confirmation;
use crate::ais::AisTarget;
use crate::boat::BoatState;
use crate::catalogue::{Catalogue, ChartFile, Choice, Request, Role};
use crate::mbtiles_renderer::MBTilesRenderer;
use crate::mesure::Mesure;
use crate::route::Route;
use crate::state::State;
use crate::track::Track;
use crate::waypoints::Waypoints;
use crate::Mode;

pub(crate) struct Session {
    pub renderer: MBTilesRenderer,
    pub state: State,
    pub mode: Mode,
    pub running: bool,
    pub boat: BoatState,
    /// The AIS targets currently shown, as last published by the Signal K
    /// task and thinned by `expire_ais`.
    pub ais_targets: Vec<AisTarget>,
    last_ais_expiry: Instant,
    pub route: Route,
    pub mesure: Mesure,
    pub track: Track,
    pub waypoints: Waypoints,
    /// Every chart file and what the navigator decided about it.
    pub catalogue: Catalogue,
    /// Shown when nothing in the catalogue fits the view (`charts.default`).
    default_chart: String,
    /// Files that failed to open, not retried every frame.
    unavailable: HashSet<String>,
    /// The last choice, as logged, so a change is logged once.
    last_choice: String,
    /// The photo layer the current round of photos started from: their
    /// order stays put while `NextPhoto` steps through them.
    pub(super) photo_round: Option<String>,
    /// Coverage outlined on the chart by the manager's "Where".
    pub highlight: Option<crate::catalogue::Bounds>,
    pub confirmation: Confirmation,
    /// Where GPX exports are written.
    pub export_dir: PathBuf,
}

impl Session {
    /// A session over the given parts, with no AIS targets yet.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        renderer: MBTilesRenderer,
        state: State,
        boat: BoatState,
        route: Route,
        track: Track,
        waypoints: Waypoints,
        catalogue: Catalogue,
        default_chart: String,
        export_dir: PathBuf,
    ) -> Self {
        let mut session = Session {
            renderer,
            state,
            mode: Mode::default(),
            running: true,
            boat,
            ais_targets: Vec::new(),
            last_ais_expiry: Instant::now(),
            route,
            mesure: Mesure::new(),
            track,
            waypoints,
            catalogue,
            default_chart,
            unavailable: HashSet::new(),
            last_choice: String::new(),
            photo_round: None,
            highlight: None,
            confirmation: Confirmation::default(),
            export_dir,
        };
        // A state saved before layers existed: the layer of the chart it
        // showed. The renderer may have settled on another zoom.
        if session.state.layer.is_none() {
            session.state.layer = session
                .catalogue
                .visible()
                .find(|f| f.path.to_string_lossy() == session.state.map)
                .map(|f| f.layer.clone());
            if let Some(layer) = session.state.layer.clone() {
                match session.catalogue.role_of(&layer) {
                    Role::Chart => session.state.chart_layer = Some(layer),
                    Role::Imagery => session.state.imagery_layer = Some(layer),
                }
            }
        }
        session.state.zoom = session.renderer.zoom();
        // The renderer knows where it is looking before the first event or frame.
        session.sync_view();
        session
    }

    /// A new fix from the Signal K task. While following, the view moves
    /// with a valid fix; an invalid fix is kept (the marker hides) but never
    /// followed. Logs when the fix becomes valid or invalid.
    pub fn on_boat(&mut self, boat: BoatState) {
        if self.boat.position_valid != boat.position_valid {
            crate::diagnostics::record(
                "INFO",
                if boat.position_valid {
                    "Position du bateau reçue; marqueur activé"
                } else {
                    "Position du bateau invalide; marqueur masqué"
                },
            );
        }
        self.boat = boat;
        if self.state.follow && valid_boat_position(&boat) {
            self.state.position = Some((boat.longitude, boat.latitude));
        }
    }

    /// The AIS list as the Signal K task last published it.
    pub fn set_ais(&mut self, targets: Vec<AisTarget>) {
        self.ais_targets = targets;
    }

    /// Drops targets silent for longer than `ais::TARGET_TIMEOUT`, checking
    /// at most once a second. The Signal K task applies the same rule when a
    /// delta arrives; this pass covers a connection that has gone quiet.
    /// Returns true when the list changed.
    pub fn expire_ais(&mut self, now: Instant) -> bool {
        if self.ais_targets.is_empty()
            || now.duration_since(self.last_ais_expiry) < Duration::from_secs(1)
        {
            return false;
        }
        self.last_ais_expiry = now;
        crate::ais::expire(&mut self.ais_targets, now)
    }

    /// The position at the centre of the screen: the boat while following,
    /// otherwise the saved position, falling back to the chart's own centre
    /// the first time. Pushes it, the pan offset and the course-up rotation
    /// into the renderer so `screen_to_gps`, `gps_to_point`, prefetching and
    /// the next frame all use the same geometry.
    /// Pushes the view derived from the state into the renderer, then shows
    /// the chart file the catalogue chooses for it.
    pub fn sync_view(&mut self) {
        self.push_view();
        self.show_chosen_chart();
    }

    /// What `Catalogue::choose` says about the current view.
    pub fn chart_choice(&self) -> Choice<'_> {
        let (centre, zoom) = self.view_centre_and_zoom();
        self.catalogue.choose(Request {
            layer: self.state.layer.as_deref(),
            lock: self.state.lock.as_deref(),
            centre,
            zoom,
        })
    }

    /// The centre of the screen and the zoom being drawn, not the one
    /// reached: during the animation a file deeper than the drawing would be
    /// drawn levels above its first level (a wheel burst onto the Azores
    /// cost a 57 s frame).
    fn view_centre_and_zoom(&self) -> ((f64, f64), i32) {
        let centre = self
            .renderer
            .view()
            .panned_center(0, 0)
            .or(self.state.position)
            .unwrap_or_default();
        (centre, self.renderer.display_zoom().round() as i32)
    }

    /// The photo sources available at the view, in the order `NextPhoto`
    /// steps through them: the round's first, else the last one used.
    fn photo_sources(&self) -> Vec<&ChartFile> {
        let (centre, zoom) = self.view_centre_and_zoom();
        let first = self
            .photo_round
            .as_deref()
            .or(self.state.imagery_layer.as_deref());
        self.catalogue.photo_sources(centre, zoom, first)
    }

    /// The chosen layer is a photo layer.
    pub(super) fn on_photo_layer(&self) -> bool {
        self.state
            .layer
            .as_deref()
            .is_some_and(|layer| self.catalogue.role_of(layer) == Role::Imagery)
    }

    /// The file shown is a photo.
    pub(super) fn showing_photo(&self) -> bool {
        self.chart_choice()
            .file
            .is_some_and(|f| self.catalogue.role_of(&f.layer) == Role::Imagery)
    }

    /// Where the shown photo is among the sources here, 0-based.
    fn photo_index(&self, sources: &[&ChartFile]) -> Option<usize> {
        let shown = self.chart_choice().file?;
        if self.catalogue.role_of(&shown.layer) != Role::Imagery {
            return None;
        }
        sources.iter().position(|f| f.layer == shown.layer)
    }

    /// The photo layer one `NextPhoto` goes to; `None` means the chart
    /// (from a photo) or nothing at all (from a chart).
    pub(super) fn next_photo_layer(&self) -> Option<String> {
        let sources = self.photo_sources();
        let next = match self.photo_index(&sources) {
            Some(k) => k + 1,
            None if self.showing_photo() => sources.len(),
            None => 0,
        };
        sources.get(next).map(|f| f.layer.clone())
    }

    /// The photo button, while a photo source is available here or a photo
    /// is shown: what a tap goes to, and which source is on.
    #[cfg(test)]
    pub fn photo_button(&self) -> Option<PhotoButton> {
        let sources = self.photo_sources();
        let showing = self.showing_photo();
        if sources.is_empty() && !showing {
            return None;
        }
        let index = self.photo_index(&sources);
        let next = match index {
            Some(k) => sources.get(k + 1),
            None if showing => None,
            None => sources.first(),
        };
        let (preview, preview_is_photo) = match next {
            Some(file) => (Some(file.path.clone()), true),
            None => {
                let (centre, zoom) = self.view_centre_and_zoom();
                let chart = self.catalogue.choose(Request {
                    layer: self.state.chart_layer.as_deref(),
                    lock: None,
                    centre,
                    zoom,
                });
                (chart.file.map(|f| f.path.clone()), false)
            }
        };
        Some(PhotoButton {
            preview,
            preview_is_photo,
            position: index.map(|k| (k + 1, sources.len())),
        })
    }

    /// What the bottom bar says about the chart shown, and whether it is a
    /// photo (the badge shows, in the stream too).
    pub fn chart_status(&self) -> ChartStatus {
        let choice = self.chart_choice();
        let mut text = match choice.file {
            Some(file) => format!("{}: {}", file.layer, file.name),
            None => "Overview chart".to_string(),
        };
        if let Some(levels) = choice.overscale {
            text.push_str(&format!(" · OVERSCALE ×{}", 1u32 << levels.clamp(0, 20)));
        }
        if let (Some(from), Some(file)) = (choice.fallback_from, choice.file) {
            text.push_str(&format!(" · No {from} here, showing {}", file.layer));
        }
        if let Some(file) = choice.more_detail {
            text.push_str(&format!(" · More detail: {}", file.name));
        }
        if self.state.lock.is_some() && choice.file.is_some() {
            text.push_str(if choice.outside_lock {
                " · LOCKED · Outside locked chart"
            } else {
                " · LOCKED"
            });
        }
        ChartStatus {
            text,
            imagery: choice
                .file
                .is_some_and(|f| self.catalogue.role_of(&f.layer) == Role::Imagery),
        }
    }

    /// The deepest zoom worth zooming in to at the view centre.
    pub fn zoom_limit(&self) -> i32 {
        self.chart_choice()
            .zoom_limit
            .unwrap_or(0)
            .max(self.renderer.top_zoom())
    }

    fn show_chosen_chart(&mut self) {
        // A photo layer with nothing here: back to the chart for good, so
        // the photo never comes back without a tap.
        let photo_left = self.on_photo_layer() && {
            let choice = self.chart_choice();
            choice.fallback_from.is_some() && choice.fallback_from == self.state.layer.as_deref()
        };
        if photo_left {
            crate::diagnostics::record("INFO", "Photo hors couverture, retour à la carte");
            self.choose_layer(self.state.chart_layer.clone());
        }
        let choice = self.chart_choice();
        let target = choice.file.map_or_else(
            || self.default_chart.clone(),
            |file| file.path.to_string_lossy().into_owned(),
        );
        let beneath = choice
            .beneath
            .map(|file| file.path.to_string_lossy().into_owned());
        let summary = describe_choice(&choice, &target);
        if summary != self.last_choice {
            crate::diagnostics::record("DEBUG", format!("Carte choisie: {summary}"));
            self.last_choice = summary;
        }
        if target != self.renderer.path() && !self.unavailable.contains(&target) {
            // Same zoom, same view: the file changes under it, offset included.
            match self.renderer.change_map(&target) {
                Ok(()) => {
                    crate::diagnostics::record("INFO", format!("Carte affichée: {target}"));
                    self.state.map = target;
                }
                Err(error) => self.mark_unavailable(target, error),
            }
        }
        let beneath = beneath.filter(|path| !self.unavailable.contains(path));
        if beneath.as_deref() != self.renderer.beneath_path() {
            if let Err(error) = self.renderer.set_beneath(beneath.as_deref()) {
                self.mark_unavailable(beneath.unwrap_or_default(), error);
            }
        }
    }

    fn mark_unavailable(&mut self, path: String, error: rusqlite::Error) {
        crate::diagnostics::record("ERROR", format!("Impossible d'ouvrir {path}: {error}"));
        self.unavailable.insert(path);
    }

    fn push_view(&mut self) {
        let saved = *self
            .state
            .position
            .get_or_insert_with(|| self.renderer.initial_center());
        let (lon, lat) = if self.state.follow && valid_boat_position(&self.boat) {
            (self.boat.longitude, self.boat.latitude)
        } else {
            saved
        };
        let rotation = if self.state.course_up
            && self.boat.position_valid
            && self.boat.course_over_ground.is_finite()
        {
            self.boat.course_over_ground
        } else {
            0.0
        };
        self.renderer.set_rotation(rotation);
        self.renderer.look_at(lon, lat, self.state.map_offset);
    }
}

/// The photo button: the file whose tile previews where a tap goes (the
/// next photo source, or the chart after the last), and while a photo is
/// shown, which source of how many it is (1-based).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoButton {
    pub preview: Option<PathBuf>,
    pub preview_is_photo: bool,
    pub position: Option<(usize, usize)>,
}

/// The chart part of the bottom bar.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartStatus {
    pub text: String,
    /// The file shown is imagery, not a chart.
    pub imagery: bool,
}

/// One line for the log: the file and every indication that goes with it.
fn describe_choice(choice: &Choice, target: &str) -> String {
    let mut line = target.to_string();
    if let Some(levels) = choice.overscale {
        line.push_str(&format!(", surzoom ×{}", 1u32 << levels.clamp(0, 20)));
    }
    if let Some(file) = choice.more_detail {
        line.push_str(&format!(", plus détaillée: {}", file.name));
    }
    if let Some(layer) = choice.fallback_from {
        line.push_str(&format!(", rien dans la couche {layer} ici"));
    }
    if let Some(file) = choice.beneath {
        line.push_str(&format!(", dessous: {}", file.name));
    }
    if choice.outside_lock {
        line.push_str(", hors de la carte verrouillée");
    }
    line
}

pub(crate) fn valid_boat_position(boat: &BoatState) -> bool {
    boat.position_valid
        && boat.longitude.is_finite()
        && boat.latitude.is_finite()
        && (-180.0..=180.0).contains(&boat.longitude)
        && (-90.0..=90.0).contains(&boat.latitude)
}

/// The Session fixture shared by the action and tool tests.
#[cfg(test)]
pub(in crate::app) mod test_support {
    use super::Session;
    use crate::boat::BoatState;
    use crate::catalogue::Catalogue;
    use crate::config::RenderingConfig;
    use crate::mbtiles_renderer::test_support::{fixture_chart, unique_name};
    use crate::mbtiles_renderer::MBTilesRenderer;
    use crate::{route::Route, state::State, track::Track, waypoints::Waypoints};
    use std::path::PathBuf;

    /// A Session over one fixture chart, with every file under a temp dir.
    /// The route has three points along the meridian.
    pub(in crate::app) struct Fixture {
        pub dir: PathBuf,
        pub chart: PathBuf,
        pub session: Session,
    }

    pub(in crate::app) fn session(zooms: &[i32]) -> Fixture {
        session_with_charts(&[("fixture.mbtiles", zooms)])
    }

    /// A Session over fixture charts (whole-world tiles at the given zooms)
    /// named as given in `dir/charts`, catalogued, the first one open and
    /// also the default chart.
    pub(in crate::app) fn session_with_charts(charts: &[(&str, &[i32])]) -> Fixture {
        let bounded: Vec<(&str, &[i32], &str)> = charts
            .iter()
            .map(|&(name, zooms)| (name, zooms, ""))
            .collect();
        session_with_bounded_charts(&bounded)
    }

    /// As `session_with_charts`, each file declaring the given `bounds`
    /// metadata ("west,south,east,north"; empty for none), which the
    /// catalogue takes as its coverage.
    pub(in crate::app) fn session_with_bounded_charts(charts: &[(&str, &[i32], &str)]) -> Fixture {
        let dir = std::env::temp_dir().join(format!("plotter-session-{}", unique_name()));
        let chart_dir = dir.join("charts");
        std::fs::create_dir_all(&chart_dir).unwrap();
        for (name, zooms, bounds) in charts {
            let path = chart_dir.join(name);
            std::fs::rename(fixture_chart(zooms), &path).unwrap();
            if !bounds.is_empty() {
                rusqlite::Connection::open(&path)
                    .unwrap()
                    .execute("INSERT INTO metadata VALUES ('bounds', ?1)", [bounds])
                    .unwrap();
            }
        }
        let chart = chart_dir.join(charts[0].0);
        let chart_str = chart.to_str().unwrap().to_string();
        let catalogue = Catalogue::load(&chart_dir, &dir.join("charts.json"));
        let state = State::new(dir.join("state.json"), chart_str.clone()).unwrap();
        let renderer = MBTilesRenderer::new(
            &chart_str,
            state.zoom,
            512,
            256,
            &RenderingConfig::default(),
        )
        .unwrap();
        let session = Session::new(
            renderer,
            state,
            BoatState::default(),
            Route::from_points_at(
                vec![(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)],
                &dir.join("route.json"),
            ),
            Track::new(&dir.join("track.json")),
            Waypoints::empty_at(&dir.join("waypoints.json")),
            catalogue,
            chart_str,
            dir.join("export"),
        );
        Fixture {
            dir,
            chart,
            session,
        }
    }

    impl Fixture {
        /// Drops the session (which holds the chart open) before deleting.
        pub(in crate::app) fn finish(self) {
            let Fixture {
                dir,
                chart,
                session,
            } = self;
            drop(session);
            drop(chart);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::session;
    use crate::ais::{AisTarget, TARGET_TIMEOUT};
    use crate::boat::BoatState;
    use std::time::{Duration, Instant};

    fn boat_at(lon: f64, lat: f64) -> BoatState {
        BoatState {
            longitude: lon,
            latitude: lat,
            position_valid: true,
            speed_over_ground: 5.0,
            course_over_ground: 1.0,
        }
    }

    #[test]
    fn a_boat_update_moves_the_view_only_while_following() {
        let mut fx = session(&[2]);
        fx.session.state.follow = false;
        fx.session.state.position = Some((0.0, 0.0));
        fx.session.on_boat(boat_at(10.0, 20.0));
        assert_eq!(fx.session.boat.longitude, 10.0);
        assert_eq!(fx.session.state.position, Some((0.0, 0.0)), "not following");

        fx.session.state.follow = true;
        fx.session.on_boat(boat_at(11.0, 21.0));
        assert_eq!(fx.session.state.position, Some((11.0, 21.0)));

        let mut invalid = boat_at(12.0, 22.0);
        invalid.position_valid = false;
        fx.session.on_boat(invalid);
        assert_eq!(
            fx.session.state.position,
            Some((11.0, 21.0)),
            "invalid fix is not followed"
        );
        assert!(
            !fx.session.boat.position_valid,
            "but the boat state is kept"
        );
        fx.finish();
    }

    #[test]
    fn stale_ais_targets_expire_after_the_timeout_and_at_most_once_a_second() {
        let mut fx = session(&[2]);
        fx.session.set_ais(vec![
            AisTarget::new("vessels.a".to_string()),
            AisTarget::new("vessels.b".to_string()),
        ]);
        let now = Instant::now();
        assert!(!fx.session.expire_ais(now), "fresh targets stay");
        assert_eq!(fx.session.ais_targets.len(), 2);

        let later = now + TARGET_TIMEOUT + Duration::from_secs(1);
        assert!(
            !fx.session.expire_ais(now + Duration::from_millis(500)),
            "checked less than a second ago"
        );
        assert!(fx.session.expire_ais(later), "both expired");
        assert!(fx.session.ais_targets.is_empty());
        assert!(
            !fx.session.expire_ais(later + Duration::from_secs(2)),
            "nothing left to expire"
        );
        fx.finish();
    }
}
