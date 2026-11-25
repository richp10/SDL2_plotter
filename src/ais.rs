use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::overlay::Shape;

/// A target not heard from for this long is dropped from the display.
pub const TARGET_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Removes targets that have not been heard from within the timeout.
/// Returns true when at least one target was removed.
pub fn retain_fresh(targets: &mut HashMap<String, AisTarget>, now: Instant) -> bool {
    let before = targets.len();
    targets.retain(|_, target| target.is_fresh(now));
    targets.len() != before
}

/// At most this many targets are kept. A busy port has a few hundred.
pub const MAX_TARGETS: usize = 500;

/// Keeps the target map bounded for a passage of weeks: removes silent
/// targets (`retain_fresh`), then the least recently heard ones above
/// `MAX_TARGETS`. Static data (name) lives in the target, so it goes with
/// it. Returns true when at least one target was removed.
pub fn bound(targets: &mut HashMap<String, AisTarget>, now: Instant) -> bool {
    let mut removed = retain_fresh(targets, now);
    if targets.len() > MAX_TARGETS {
        let mut by_age: Vec<(Instant, String)> = targets
            .iter()
            .map(|(context, target)| (target.last_update, context.clone()))
            .collect();
        by_age.sort();
        let excess = targets.len() - MAX_TARGETS;
        for (_, context) in by_age.into_iter().take(excess) {
            targets.remove(&context);
        }
        removed = true;
    }
    removed
}

/// The same rule for the displayed list. Returns true when one was removed.
pub fn expire(targets: &mut Vec<AisTarget>, now: Instant) -> bool {
    let before = targets.len();
    targets.retain(|target| target.is_fresh(now));
    targets.len() != before
}

#[derive(Clone, Debug)]
pub struct AisTarget {
    pub context: String,
    pub name: Option<String>,
    pub position: Option<(f64, f64)>,
    pub course_over_ground: Option<f64>,
    pub speed_over_ground: Option<f64>,
    /// True heading in radians, when the target sends one.
    pub heading: Option<f64>,
    /// AIS ship type, 1 to 99, once a static report gave it.
    pub ship_type: Option<u8>,
    /// AIS navigational status, 0 to 14 (class A position reports).
    pub nav_status: Option<u8>,
    /// A class B transponder (small craft).
    pub class_b: bool,
    last_update: Instant,
}

impl AisTarget {
    pub(crate) fn new(context: String) -> Self {
        Self {
            context,
            name: None,
            position: None,
            course_over_ground: None,
            speed_over_ground: None,
            heading: None,
            ship_type: None,
            nav_status: None,
            class_b: false,
            last_update: Instant::now(),
        }
    }

    pub fn is_fresh(&self, now: Instant) -> bool {
        now.duration_since(self.last_update) < TARGET_TIMEOUT
    }

    pub(crate) fn update_received(&mut self) {
        self.last_update = Instant::now();
    }

    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.context
                .rsplit(':')
                .next()
                .unwrap_or(&self.context)
                .to_string()
        })
    }
}

/// One labelled marker per target that has a position.
pub fn shapes(targets: &[AisTarget]) -> Vec<Shape> {
    targets
        .iter()
        .filter_map(|target| {
            target.position.map(|at| Shape::Ais {
                at,
                course: target.course_over_ground,
                heading: target.heading,
                label: target.label(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_expires_after_five_minutes_without_updates() {
        let now = Instant::now();
        let mut target = AisTarget::new("vessels.test".to_string());

        target.last_update = now - Duration::from_secs(299);
        assert!(target.is_fresh(now));

        target.last_update = now - TARGET_TIMEOUT;
        assert!(!target.is_fresh(now));
    }

    #[test]
    fn retain_fresh_drops_expired_targets_and_reports_whether_any_went() {
        let now = Instant::now();
        let mut targets = std::collections::HashMap::new();
        let mut stale = AisTarget::new("vessels.stale".to_string());
        stale.last_update = now - TARGET_TIMEOUT - Duration::from_secs(1);
        targets.insert(stale.context.clone(), stale);
        targets.insert(
            "vessels.fresh".to_string(),
            AisTarget::new("vessels.fresh".to_string()),
        );

        assert!(retain_fresh(&mut targets, now));
        assert_eq!(targets.len(), 1);
        assert!(targets.contains_key("vessels.fresh"));

        assert!(!retain_fresh(&mut targets, now));
        assert_eq!(targets.len(), 1);
    }

    #[test]
    fn a_target_shape_carries_its_course_and_heading() {
        let mut target = AisTarget::new("vessels.urn:mrn:imo:mmsi:232001001".to_string());
        target.position = Some((0.5, 51.5));
        target.course_over_ground = Some(1.0);
        target.heading = Some(1.5);
        assert_eq!(
            shapes(&[target]),
            vec![Shape::Ais {
                at: (0.5, 51.5),
                course: Some(1.0),
                heading: Some(1.5),
                label: "232001001".into(),
            }]
        );
    }

    #[test]
    fn the_target_list_is_capped_by_dropping_the_least_recent() {
        let now = Instant::now();
        let mut targets = HashMap::new();
        for i in 0..MAX_TARGETS + 2 {
            let mut target = AisTarget::new(format!("vessels.{i}"));
            target.last_update = now - Duration::from_millis(i as u64);
            targets.insert(target.context.clone(), target);
        }
        assert!(bound(&mut targets, now));
        assert_eq!(targets.len(), MAX_TARGETS);
        assert!(targets.contains_key("vessels.0"), "the newest stays");
        assert!(!targets.contains_key(&format!("vessels.{}", MAX_TARGETS + 1)));
        assert!(!targets.contains_key(&format!("vessels.{MAX_TARGETS}")));
        assert!(!bound(&mut targets, now), "nothing more to drop");
    }

    #[test]
    fn bound_also_drops_silent_targets() {
        let now = Instant::now();
        let mut targets = HashMap::new();
        let mut stale = AisTarget::new("vessels.stale".to_string());
        stale.last_update = now - TARGET_TIMEOUT;
        targets.insert(stale.context.clone(), stale);
        assert!(bound(&mut targets, now));
        assert!(targets.is_empty());
    }
}
