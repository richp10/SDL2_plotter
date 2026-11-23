use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::mbtiles_renderer::MBTilesRenderer;
use crate::screen::Screen;

const TARGET_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Removes targets that have not been heard from within the timeout.
/// Returns true when at least one target was removed.
pub fn retain_fresh(targets: &mut HashMap<String, AisTarget>, now: Instant) -> bool {
    let before = targets.len();
    targets.retain(|_, target| target.is_fresh(now));
    targets.len() != before
}

#[derive(Clone, Debug)]
pub struct AisTarget {
    pub context: String,
    pub name: Option<String>,
    pub position: Option<(f64, f64)>,
    pub course_over_ground: Option<f64>,
    pub speed_over_ground: Option<f64>,
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

pub fn show_targets(
    targets: &[AisTarget],
    renderer: &MBTilesRenderer,
    screen: &mut Screen,
) -> Result<(), String> {
    let view = renderer.view();

    for target in targets {
        let Some(position) = target.position else {
            continue;
        };
        let point = view.to_pixel(position);
        if !view.contains(point, 32) {
            continue;
        }

        screen.show_ais_target(
            point.x,
            point.y,
            target
                .course_over_ground
                .map(|course| view.course_to_screen(course)),
        )?;
        screen.show_text(&target.label(), point.x + 10, point.y - 10)?;
    }

    Ok(())
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
}
