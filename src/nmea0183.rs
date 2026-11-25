//! NMEA 0183 input: the decoder turns one sentence into a `BoatState`
//! update. Only RMC (position, speed, course), HDT and HDG (heading) are
//! read; see `docs/nmea0183-input-design.md`.

mod transport;

use std::time::Instant;

use crate::boat::{BoatState, HEADING_TIMEOUT};
use crate::config::{Nmea0183Config, Transport};
use crate::feed::LiveFeed;

/// 1 knot = 1852 m per hour.
const METRES_PER_SECOND_PER_KNOT: f64 = 1852.0 / 3600.0;

/// The NMEA 0183 source task: reads lines from the configured transport,
/// decodes them and publishes on `feed`. Never returns. AIS is not read
/// yet, so the AIS list stays empty.
pub async fn run(config: Nmea0183Config, feed: LiveFeed) {
    crate::diagnostics::record(
        "INFO",
        format!(
            "Tâche NMEA 0183 démarrée, {:?} {}",
            config.transport, config.address
        ),
    );
    let mut decoder = Decoder::new();
    let on_line = |line: &str| {
        let update = decoder.apply(line, Instant::now());
        if update.boat_changed {
            let _ = feed.boat.send(decoder.boat());
        }
        if let Some(position) = update.track_point {
            let _ = feed.track.send(position);
        }
    };
    match config.transport {
        Transport::Tcp => transport::tcp(&config.address, on_line).await,
        Transport::Udp => transport::udp(&config.address, on_line).await,
    }
}

/// What one sentence changed.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Update {
    /// The boat state changed: publish `Decoder::boat()`.
    pub boat_changed: bool,
    /// A new fix: one track point, `(lon, lat)`.
    pub track_point: Option<(f64, f64)>,
}

/// The boat as the sentences so far describe it.
pub struct Decoder {
    boat: BoatState,
    /// The magnetic variation from the last RMC that had one, in degrees,
    /// east positive. HDG uses it when it has none of its own.
    variation: Option<f64>,
    /// When the last HDT came. HDG is ignored while it is fresh.
    hdt_at: Option<Instant>,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            boat: BoatState::default(),
            variation: None,
            hdt_at: None,
        }
    }

    pub fn boat(&self) -> BoatState {
        self.boat
    }

    /// Reads one line received at `now`. A line that fails a check (no
    /// checksum, bad checksum, a field out of range) changes nothing.
    pub fn apply(&mut self, line: &str, now: Instant) -> Update {
        let Some(body) = checked_body(line) else {
            return Update::default();
        };
        let fields: Vec<&str> = body.split(',').collect();
        // The talker (first two characters) is ignored.
        match fields[0].get(2..) {
            Some("RMC") => self.rmc(&fields, now),
            Some("HDT") => {
                let update = self.heading(field(&fields, 1).and_then(degrees), now);
                if update.boat_changed {
                    self.hdt_at = Some(now);
                }
                update
            }
            Some("HDG") if self.hdt_is_fresh(now) => Update::default(),
            Some("HDG") => {
                let variation = offset(field(&fields, 4), field(&fields, 5)).or(self.variation);
                // Deviation is optional; when it is given it must be valid.
                let deviation = match field(&fields, 2) {
                    None => Some(0.0),
                    deviation => offset(deviation, field(&fields, 3)),
                };
                let heading = field(&fields, 1)
                    .and_then(degrees)
                    .zip(deviation)
                    .zip(variation)
                    .map(|((magnetic, deviation), variation)| magnetic + deviation + variation);
                self.heading(heading, now)
            }
            _ => Update::default(),
        }
    }

    fn rmc(&mut self, fields: &[&str], now: Instant) -> Update {
        if let Some(variation) = offset(field(fields, 10), field(fields, 11)) {
            self.variation = Some(variation);
        }
        let mode_ok = !matches!(field(fields, 12), Some("N" | "E"));
        if field(fields, 2) != Some("A") || !mode_ok {
            self.boat.position_valid = false;
            return Update {
                boat_changed: true,
                track_point: None,
            };
        }
        let (Some(lat), Some(lon)) = (
            coordinate(field(fields, 3), field(fields, 4), 2, 'N', 'S'),
            coordinate(field(fields, 5), field(fields, 6), 3, 'E', 'W'),
        ) else {
            return Update::default();
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Update::default();
        }
        self.boat.latitude = lat;
        self.boat.longitude = lon;
        self.boat.position_valid = true;
        self.boat.fixed_at = Some(now);
        if let Some(knots) = field(fields, 7)
            .and_then(number)
            .filter(|knots| *knots >= 0.0)
        {
            self.boat.speed_over_ground = knots * METRES_PER_SECOND_PER_KNOT;
        }
        if let Some(course) = field(fields, 8).and_then(degrees) {
            self.boat.course_over_ground = course.to_radians();
        }
        Update {
            boat_changed: true,
            track_point: Some((lon, lat)),
        }
    }

    /// HDT is true heading, so it wins over HDG while it comes in.
    fn hdt_is_fresh(&self, now: Instant) -> bool {
        self.hdt_at
            .is_some_and(|at| now.saturating_duration_since(at) < HEADING_TIMEOUT)
    }

    /// A true heading in degrees, or nothing to change.
    fn heading(&mut self, degrees_true: Option<f64>, now: Instant) -> Update {
        let Some(heading) = degrees_true else {
            return Update::default();
        };
        self.boat.heading = Some(heading.rem_euclid(360.0).to_radians());
        self.boat.heading_at = Some(now);
        Update {
            boat_changed: true,
            track_point: None,
        }
    }
}

/// The text between `$` and `*` when the line is a sentence with a correct
/// checksum. A leading tag block (`\...\`) is skipped; its own checksum
/// is not checked.
fn checked_body(line: &str) -> Option<&str> {
    let mut line = line.trim();
    if let Some(rest) = line.strip_prefix('\\') {
        line = &rest[rest.find('\\')? + 1..];
    }
    let line = line.strip_prefix('$')?;
    let (body, checksum) = line.rsplit_once('*')?;
    if checksum.len() != 2 {
        return None;
    }
    let expected = u8::from_str_radix(checksum, 16).ok()?;
    let actual = body.bytes().fold(0u8, |acc, b| acc ^ b);
    (actual == expected && body.len() >= 5).then_some(body)
}

/// Field `index`, or `None` when it is missing or empty.
fn field<'a>(fields: &[&'a str], index: usize) -> Option<&'a str> {
    fields.get(index).copied().filter(|f| !f.is_empty())
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// An angle in degrees, 0..=360.
fn degrees(text: &str) -> Option<f64> {
    number(text).filter(|d| (0.0..=360.0).contains(d))
}

/// A value with its hemisphere letter: negative for `negative`.
fn signed(
    value: Option<&str>,
    letter: Option<&str>,
    positive: char,
    negative: char,
) -> Option<f64> {
    let value = number(value?)?;
    match letter?.chars().next()? {
        c if c == positive => Some(value),
        c if c == negative => Some(-value),
        _ => None,
    }
}

/// A magnetic variation or deviation in degrees, east positive, at most
/// 180 either way.
fn offset(value: Option<&str>, letter: Option<&str>) -> Option<f64> {
    signed(value, letter, 'E', 'W').filter(|degrees| degrees.abs() <= 180.0)
}

/// `ddmm.mmmm` (or `dddmm.mmmm` with `degree_digits` 3) and its hemisphere
/// letter, in decimal degrees. Minutes must be below 60.
fn coordinate(
    text: Option<&str>,
    letter: Option<&str>,
    degree_digits: usize,
    positive: char,
    negative: char,
) -> Option<f64> {
    let text = text?;
    if !text.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let dot = text.find('.').unwrap_or(text.len());
    if dot != degree_digits + 2 {
        return None;
    }
    let degrees: f64 = text[..degree_digits].parse::<u32>().ok()?.into();
    let minutes: f64 = text[degree_digits..].parse().ok()?;
    if minutes >= 60.0 {
        return None;
    }
    let value = degrees + minutes / 60.0;
    signed(Some(&value.to_string()), letter, positive, negative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `$body*hh` with the checksum computed here, independently of the
    /// decoder.
    fn sentence(body: &str) -> String {
        let sum = body.bytes().fold(0u8, |acc, b| acc ^ b);
        format!("${body}*{sum:02X}")
    }

    const GPSD_RMC: &str = "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A";

    fn fixed_decoder(now: Instant) -> Decoder {
        let mut d = Decoder::new();
        d.apply(GPSD_RMC, now);
        d
    }

    #[test]
    fn an_rmc_with_a_fix_gives_position_speed_course_and_a_track_point() {
        let mut d = Decoder::new();
        let now = Instant::now();
        let u = d.apply(GPSD_RMC, now);
        let b = d.boat();
        assert!(u.boat_changed);
        assert!((b.latitude - 48.1173).abs() < 1e-6);
        assert!((b.longitude - 11.516_666_7).abs() < 1e-6);
        assert!((b.speed_over_ground - 22.4 * 0.514_444).abs() < 1e-3);
        assert!((b.course_over_ground - 84.4_f64.to_radians()).abs() < 1e-9);
        assert!(b.position_valid);
        assert_eq!(b.fixed_at, Some(now));
        assert_eq!(u.track_point, Some((b.longitude, b.latitude)));
    }

    #[test]
    fn status_v_is_no_fix_and_keeps_the_position() {
        let now = Instant::now();
        let mut d = fixed_decoder(now);
        let u = d.apply(
            &sentence("GPRMC,123520,V,4900.000,N,01200.000,E,022.4,084.4,230394,,"),
            now + Duration::from_secs(1),
        );
        let b = d.boat();
        assert!(u.boat_changed);
        assert_eq!(u.track_point, None);
        assert!(!b.position_valid);
        assert!((b.latitude - 48.1173).abs() < 1e-6, "position kept");
        assert_eq!(b.fixed_at, Some(now), "no new fix time");
    }

    #[test]
    fn mode_n_and_mode_e_are_no_fix() {
        for mode in ["N", "E"] {
            let mut d = Decoder::new();
            let u = d.apply(
                &sentence(&format!(
                    "GPRMC,123519,A,4807.038,N,01131.000,E,5.0,90.0,230394,,,{mode}"
                )),
                Instant::now(),
            );
            assert_eq!(u.track_point, None, "mode {mode}");
            assert!(!d.boat().position_valid, "mode {mode}");
            assert_eq!(d.boat().fixed_at, None, "mode {mode}");
        }
        let mut d = Decoder::new();
        d.apply(
            &sentence("GPRMC,123519,A,4807.038,N,01131.000,E,5.0,90.0,230394,,,A"),
            Instant::now(),
        );
        assert!(d.boat().position_valid, "mode A is a fix");
    }

    #[test]
    fn the_simulator_no_fix_rmc_is_accepted() {
        let mut d = Decoder::new();
        let u = d.apply(
            &sentence("GPRMC,120000.00,V,,,,,,,,251126,,,N"),
            Instant::now(),
        );
        assert_eq!(u.track_point, None);
        assert!(!d.boat().position_valid);
    }

    #[test]
    fn a_bad_checksum_is_ignored() {
        let mut d = Decoder::new();
        let bad = GPSD_RMC.replace("*6A", "*6B");
        assert_eq!(d.apply(&bad, Instant::now()), Update::default());
        assert_eq!(d.boat().fixed_at, None);
    }

    #[test]
    fn a_line_without_a_checksum_is_ignored() {
        let mut d = Decoder::new();
        let bare = GPSD_RMC.trim_end_matches("*6A");
        assert_eq!(d.apply(bare, Instant::now()), Update::default());
    }

    #[test]
    fn line_endings_are_accepted() {
        let mut d = Decoder::new();
        let u = d.apply(&format!("{GPSD_RMC}\r\n"), Instant::now());
        assert!(u.track_point.is_some());
    }

    #[test]
    fn a_tag_block_is_skipped() {
        let mut d = Decoder::new();
        let u = d.apply(
            &format!("\\s:sim,c:1764000000*00\\{GPSD_RMC}"),
            Instant::now(),
        );
        assert!(u.track_point.is_some());
    }

    #[test]
    fn the_talker_is_ignored() {
        for talker in ["GN", "II"] {
            let mut d = Decoder::new();
            let u = d.apply(
                &sentence(&format!(
                    "{talker}RMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W"
                )),
                Instant::now(),
            );
            assert!(u.track_point.is_some(), "talker {talker}");
        }
    }

    #[test]
    fn extra_fields_are_accepted() {
        let mut d = Decoder::new();
        let u = d.apply(
            &sentence("GPRMC,123519,A,4807.038,N,01131.000,E,5.0,90.0,230394,,,A,V,extra"),
            Instant::now(),
        );
        assert!(u.track_point.is_some());
    }

    #[test]
    fn out_of_range_or_bad_coordinates_are_ignored() {
        for body in [
            "GPRMC,123519,A,9100.000,N,01131.000,E,5.0,90.0,230394,,",
            "GPRMC,123519,A,4860.000,N,01131.000,E,5.0,90.0,230394,,",
            "GPRMC,123519,A,4807.038,X,01131.000,E,5.0,90.0,230394,,",
            "GPRMC,123519,A,4807.038,N,18100.000,E,5.0,90.0,230394,,",
            "GPRMC,123519,A,48x7.038,N,01131.000,E,5.0,90.0,230394,,",
            "GPRMC,123519,A,,N,01131.000,E,5.0,90.0,230394,,",
        ] {
            let mut d = Decoder::new();
            let u = d.apply(&sentence(body), Instant::now());
            assert_eq!(u.track_point, None, "{body}");
            assert_eq!(d.boat().fixed_at, None, "{body}");
        }
    }

    #[test]
    fn south_and_west_are_negative() {
        let mut d = Decoder::new();
        d.apply(
            &sentence("GPRMC,123519,A,3330.000,S,07030.000,W,5.0,90.0,230394,,"),
            Instant::now(),
        );
        let b = d.boat();
        assert!((b.latitude + 33.5).abs() < 1e-9);
        assert!((b.longitude + 70.5).abs() < 1e-9);
    }

    #[test]
    fn empty_course_and_speed_keep_the_last_values() {
        let now = Instant::now();
        let mut d = fixed_decoder(now);
        d.apply(
            &sentence("GPRMC,123520,A,4807.040,N,01131.000,E,,,230394,,"),
            now,
        );
        let b = d.boat();
        assert!((b.speed_over_ground - 22.4 * 0.514_444).abs() < 1e-3);
        assert!((b.course_over_ground - 84.4_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn hdt_gives_a_true_heading() {
        let mut d = Decoder::new();
        let now = Instant::now();
        let u = d.apply(&sentence("HEHDT,274.07,T"), now);
        assert!(u.boat_changed);
        assert_eq!(u.track_point, None);
        let b = d.boat();
        assert!((b.heading.unwrap() - 274.07_f64.to_radians()).abs() < 1e-9);
        assert_eq!(b.heading_at, Some(now));
        assert_eq!(b.fixed_at, None, "a heading is not a fix");
    }

    #[test]
    fn hdt_wins_over_hdg_while_it_is_fresh() {
        let mut d = Decoder::new();
        let now = Instant::now();
        d.apply(&sentence("HEHDT,10.0,T"), now);
        let u = d.apply(
            &sentence("HCHDG,100.0,,,0.0,E"),
            now + Duration::from_secs(1),
        );
        assert_eq!(u, Update::default());
        assert!((d.boat().heading.unwrap() - 10.0_f64.to_radians()).abs() < 1e-9);
        d.apply(
            &sentence("HCHDG,100.0,,,0.0,E"),
            now + crate::boat::HEADING_TIMEOUT,
        );
        assert!(
            (d.boat().heading.unwrap() - 100.0_f64.to_radians()).abs() < 1e-9,
            "HDG once HDT has stopped"
        );
    }

    #[test]
    fn hdg_applies_its_deviation() {
        let mut d = Decoder::new();
        d.apply(&sentence("HCHDG,100.0,2.0,E,5.0,W"), Instant::now());
        assert!((d.boat().heading.unwrap() - 97.0_f64.to_radians()).abs() < 1e-9);
        d.apply(&sentence("HCHDG,100.0,2.0,W,5.0,W"), Instant::now());
        assert!((d.boat().heading.unwrap() - 93.0_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn a_variation_or_deviation_beyond_180_degrees_is_ignored() {
        let mut d = Decoder::new();
        let u = d.apply(&sentence("HCHDG,100.0,,,200.0,W"), Instant::now());
        assert_eq!(u, Update::default());
        let u = d.apply(&sentence("HCHDG,100.0,190.0,E,5.0,W"), Instant::now());
        assert_eq!(u, Update::default());
        d.apply(
            &sentence("GPRMC,123519,A,4807.038,N,01131.000,E,5.0,90.0,230394,200.0,W"),
            Instant::now(),
        );
        let u = d.apply(&sentence("HCHDG,100.0,,,,"), Instant::now());
        assert_eq!(u, Update::default(), "the RMC variation was not kept");
    }

    #[test]
    fn hdg_uses_its_own_variation() {
        let mut d = Decoder::new();
        d.apply(&sentence("HCHDG,100.0,,,5.0,W"), Instant::now());
        assert!((d.boat().heading.unwrap() - 95.0_f64.to_radians()).abs() < 1e-9);
        d.apply(&sentence("HCHDG,358.0,,,5.0,E"), Instant::now());
        assert!(
            (d.boat().heading.unwrap() - 3.0_f64.to_radians()).abs() < 1e-9,
            "wraps past north"
        );
    }

    #[test]
    fn hdg_uses_the_rmc_variation_when_it_has_none() {
        // GPSD_RMC has 003.1,W.
        let mut d = fixed_decoder(Instant::now());
        d.apply(&sentence("HCHDG,100.0,,,,"), Instant::now());
        assert!((d.boat().heading.unwrap() - 96.9_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn hdg_without_any_variation_gives_no_heading() {
        let mut d = Decoder::new();
        let u = d.apply(&sentence("HCHDG,100.0,,,,"), Instant::now());
        assert_eq!(u, Update::default());
        assert_eq!(d.boat().heading, None);
    }

    #[test]
    fn gga_gll_and_vtg_give_no_update() {
        let mut d = Decoder::new();
        for body in [
            "GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,",
            "GPGLL,4916.45,N,12311.12,W,225444,A",
            "GPVTG,054.7,T,034.4,M,005.5,N,010.2,K",
        ] {
            assert_eq!(d.apply(&sentence(body), Instant::now()), Update::default());
        }
        assert_eq!(d.boat().fixed_at, None);
    }

    #[test]
    fn aivdm_gives_no_update() {
        let mut d = Decoder::new();
        let line = "!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5C";
        assert_eq!(d.apply(line, Instant::now()), Update::default());
    }
}
