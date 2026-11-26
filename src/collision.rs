use crate::ais::AisTarget;
use crate::boat::BoatState;
use crate::gps;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskKind {
    Predicted,
    ProximityUncertain,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollisionRisk {
    pub kind: RiskKind,
    pub target: String,
    pub position: (f64, f64),
    pub cpa_nm: f64,
    pub tcpa_minutes: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionThresholds {
    pub cpa_nm: f64,
    pub tcpa_minutes: f64,
    pub proximity_nm: f64,
}

/// Finds AIS targets whose predicted closest approach is inside both limits.
pub fn risks(
    boat: BoatState,
    targets: &[AisTarget],
    thresholds: CollisionThresholds,
) -> Vec<CollisionRisk> {
    if !boat.position_valid
        || !boat.longitude.is_finite()
        || !boat.latitude.is_finite()
        || !boat.speed_over_ground.is_finite()
        || !boat.course_over_ground.is_finite()
    {
        return Vec::new();
    }

    targets
        .iter()
        .filter_map(|target| {
            let position = target.position?;
            let target_name = || {
                target
                    .name
                    .clone()
                    .unwrap_or_else(|| target.context.clone())
            };
            match (
                target.course_over_ground.filter(|value| value.is_finite()),
                target.speed_over_ground.filter(|value| value.is_finite()),
            ) {
                (Some(cog), Some(sog)) => {
                    let (cpa_nm, tcpa_minutes) = closest_approach(
                        (boat.longitude, boat.latitude),
                        boat.course_over_ground,
                        boat.speed_over_ground,
                        position,
                        cog,
                        sog,
                    )?;
                    (tcpa_minutes >= 0.0
                        && cpa_nm <= thresholds.cpa_nm
                        && tcpa_minutes <= thresholds.tcpa_minutes)
                        .then(|| CollisionRisk {
                            kind: RiskKind::Predicted,
                            target: target_name(),
                            position,
                            cpa_nm,
                            tcpa_minutes,
                        })
                }
                _ => {
                    let distance_nm = gps::distance_nm((boat.longitude, boat.latitude), position);
                    (distance_nm <= thresholds.proximity_nm).then(|| CollisionRisk {
                        kind: RiskKind::ProximityUncertain,
                        target: target_name(),
                        position,
                        cpa_nm: distance_nm,
                        tcpa_minutes: f64::NAN,
                    })
                }
            }
        })
        .collect()
}

fn closest_approach(
    own_position: (f64, f64),
    own_course: f64,
    own_speed_mps: f64,
    target_position: (f64, f64),
    target_course: f64,
    target_speed_mps: f64,
) -> Option<(f64, f64)> {
    let latitude = own_position.1.to_radians();
    // Across the antimeridian 179.9 E to 179.9 W is 0.2 degrees, not 359.8.
    let east_degrees = (target_position.0 - own_position.0 + 180.0).rem_euclid(360.0) - 180.0;
    let relative_position = (
        east_degrees * latitude.cos() * 60.0,
        (target_position.1 - own_position.1) * 60.0,
    );
    let own_velocity = velocity(own_course, own_speed_mps);
    let target_velocity = velocity(target_course, target_speed_mps);
    let relative_velocity = (
        (target_velocity.0 - own_velocity.0) / 60.0,
        (target_velocity.1 - own_velocity.1) / 60.0,
    );
    let speed_squared =
        relative_velocity.0 * relative_velocity.0 + relative_velocity.1 * relative_velocity.1;
    if speed_squared <= f64::EPSILON {
        return Some((distance(relative_position), 0.0));
    }

    let tcpa = -(relative_position.0 * relative_velocity.0
        + relative_position.1 * relative_velocity.1)
        / speed_squared;
    let closest = (
        relative_position.0 + relative_velocity.0 * tcpa,
        relative_position.1 + relative_velocity.1 * tcpa,
    );
    Some((distance(closest), tcpa))
}

fn velocity(course: f64, speed_mps: f64) -> (f64, f64) {
    let speed_knots = speed_mps * 1.943_844_492_44;
    (course.sin() * speed_knots, course.cos() * speed_knots)
}

fn distance((east, north): (f64, f64)) -> f64 {
    (east * east + north * north).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(position: (f64, f64), course: f64, speed: f64) -> AisTarget {
        let mut target = AisTarget::new("vessels.target".into());
        target.name = Some("Target".into());
        target.position = Some(position);
        target.course_over_ground = Some(course);
        target.speed_over_ground = Some(speed);
        target
    }

    #[test]
    fn detects_ahead_target_on_crossing_course() {
        let boat = BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: true,
            speed_over_ground: 5.144,
            course_over_ground: 0.0,
            ..BoatState::default()
        };
        let target = target((0.0, 0.03), 0.0, 0.0);
        let risks = risks(
            boat,
            &[target],
            CollisionThresholds {
                cpa_nm: 0.5,
                tcpa_minutes: 15.0,
                proximity_nm: 2.0,
            },
        );
        assert_eq!(risks.len(), 1);
        assert!(risks[0].cpa_nm < 0.5);
        assert!(risks[0].tcpa_minutes < 15.0);
    }

    #[test]
    fn ignores_targets_that_are_behind_in_time() {
        let boat = BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: true,
            speed_over_ground: 5.144,
            course_over_ground: 0.0,
            ..BoatState::default()
        };
        let target = target((0.0, -0.1), 0.0, 5.144);
        assert!(risks(
            boat,
            &[target],
            CollisionThresholds {
                cpa_nm: 0.5,
                tcpa_minutes: 15.0,
                proximity_nm: 2.0,
            }
        )
        .is_empty());
    }

    #[test]
    fn a_target_across_the_antimeridian_is_near_not_half_a_world_away() {
        let boat = BoatState {
            longitude: 179.99,
            latitude: 0.0,
            position_valid: true,
            speed_over_ground: 0.0,
            course_over_ground: 0.0,
            ..BoatState::default()
        };
        // 0.02 degrees east at the equator: 1.2 NM, closing at 6 knots.
        let target = target(
            (-179.99, 0.0),
            -std::f64::consts::FRAC_PI_2,
            6.0 * 1852.0 / 3600.0,
        );
        let risks = risks(
            boat,
            &[target],
            CollisionThresholds {
                cpa_nm: 0.5,
                tcpa_minutes: 15.0,
                proximity_nm: 2.0,
            },
        );
        assert_eq!(risks.len(), 1, "{risks:?}");
        assert!(risks[0].cpa_nm < 0.01);
        assert!((risks[0].tcpa_minutes - 12.0).abs() < 0.1, "{risks:?}");
    }

    #[test]
    fn warns_about_a_close_target_without_motion_data() {
        let boat = BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: true,
            speed_over_ground: 0.0,
            course_over_ground: 0.0,
            ..BoatState::default()
        };
        let mut target = AisTarget::new("vessels.unknown".into());
        target.position = Some((0.0, 0.01));
        let risks = risks(
            boat,
            &[target],
            CollisionThresholds {
                cpa_nm: 0.5,
                tcpa_minutes: 15.0,
                proximity_nm: 2.0,
            },
        );
        assert_eq!(risks[0].kind, RiskKind::ProximityUncertain);
        assert!(risks[0].tcpa_minutes.is_nan());
    }
}
