//! AIS over NMEA 0183: `!AIVDM` sentences, assembled from their fragments
//! and decoded from 6-bit text. Only what the plotter draws is read:
//! position, speed, course, heading and name (message types 1, 2, 3, 5,
//! 18, 19 and 24).

use std::collections::HashMap;

/// A message has at most this many fragments (5 for a type 5 in practice).
const MAX_FRAGMENTS: u8 = 5;
/// Messages being assembled at one time (sequence ids are 0 to 9).
const MAX_PENDING: usize = 10;
const METRES_PER_SECOND_PER_KNOT: f64 = 1852.0 / 3600.0;

/// What the plotter reads from one AIS message.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Message {
    Position(PositionReport),
    /// A static report: whatever of `StaticData` it carries.
    Static {
        mmsi: u32,
        data: StaticData,
        /// Class B (type 24); class A (type 5) otherwise.
        class_b: bool,
    },
}

/// What a static report says about a vessel. "Not available" (an empty
/// text, a zero) is `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct StaticData {
    pub name: Option<String>,
    /// AIS ship type, 1 to 99.
    pub ship_type: Option<u8>,
    pub call_sign: Option<String>,
    pub destination: Option<String>,
    /// Bow to stern, from the AIS antenna offsets.
    pub length_m: Option<u16>,
    /// Port to starboard.
    pub beam_m: Option<u16>,
    pub draught_m: Option<f64>,
}

/// A position report. Every value the transmitter marks "not available"
/// (lon 181, lat 91, SOG 102.3 kn, COG 360, heading 511) is `None`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PositionReport {
    pub mmsi: u32,
    /// `(lon, lat)` in degrees.
    pub position: Option<(f64, f64)>,
    /// m/s.
    pub speed_over_ground: Option<f64>,
    /// Radians true.
    pub course_over_ground: Option<f64>,
    /// Radians true.
    pub heading: Option<f64>,
    /// Type 19 carries the name too.
    pub name: Option<String>,
    /// Navigational status 0 to 14 (class A only; 15, "not defined", is
    /// `None`).
    pub nav_status: Option<u8>,
    /// Type 19 carries the ship type.
    pub ship_type: Option<u8>,
    /// Types 18 and 19 are class B.
    pub class_b: bool,
}

/// Joins the fragments of multi-sentence messages.
#[derive(Default)]
pub(super) struct Assembler {
    pending: HashMap<String, Pending>,
}

struct Pending {
    count: u8,
    next: u8,
    payload: String,
}

impl Assembler {
    /// Takes the fields of one checked `!AIVDM` body. Returns the whole
    /// payload and its fill bits when the last fragment arrives. A fragment
    /// that does not follow the one before it drops the message.
    pub fn push(&mut self, fields: &[&str]) -> Option<(String, u8)> {
        let count: u8 = fields.get(1)?.parse().ok()?;
        let number: u8 = fields.get(2)?.parse().ok()?;
        if !(1..=MAX_FRAGMENTS).contains(&count) || !(1..=count).contains(&number) {
            return None;
        }
        let sequence = fields.get(3)?.to_string();
        let payload = *fields.get(5)?;
        let fill: u8 = fields.get(6)?.parse().ok()?;
        if count == 1 {
            return Some((payload.to_string(), fill));
        }
        if number == 1 {
            if self.pending.len() >= MAX_PENDING {
                self.pending.clear();
            }
            self.pending.insert(
                sequence,
                Pending {
                    count,
                    next: 2,
                    payload: payload.to_string(),
                },
            );
            return None;
        }
        let pending = self.pending.remove(&sequence)?;
        if pending.count != count || pending.next != number {
            return None;
        }
        let payload = pending.payload + payload;
        if number == count {
            return Some((payload, fill));
        }
        self.pending.insert(
            sequence,
            Pending {
                count,
                next: number + 1,
                payload,
            },
        );
        None
    }
}

/// Where a position report keeps its fields (bit offsets, gpsd AIVDM).
struct Layout {
    sog: usize,
    lon: usize,
    lat: usize,
    cog: usize,
    heading: usize,
    nav_status: Option<usize>,
    name: Option<usize>,
    ship_type: Option<usize>,
    class_b: bool,
}

const CLASS_A: Layout = Layout {
    sog: 50,
    lon: 61,
    lat: 89,
    cog: 116,
    heading: 128,
    nav_status: Some(38),
    name: None,
    ship_type: None,
    class_b: false,
};
const CLASS_B: Layout = Layout {
    sog: 46,
    lon: 57,
    lat: 85,
    cog: 112,
    heading: 124,
    nav_status: None,
    name: None,
    ship_type: None,
    class_b: true,
};
const CLASS_B_EXTENDED: Layout = Layout {
    name: Some(143),
    ship_type: Some(263),
    ..CLASS_B
};

/// Decodes one whole payload. `None` for a bad payload, a type the plotter
/// does not read, or a static report with neither name nor ship type.
pub(super) fn decode(payload: &str, fill: u8) -> Option<Message> {
    let bits = Bits::new(payload, fill)?;
    let static_report = |data: StaticData, class_b| {
        if data == StaticData::default() {
            return None;
        }
        Some(Message::Static {
            mmsi: bits.mmsi()?,
            data,
            class_b,
        })
    };
    match bits.uint(0, 6)? {
        1..=3 => bits.position(&CLASS_A),
        18 => bits.position(&CLASS_B),
        19 => bits.position(&CLASS_B_EXTENDED),
        5 => {
            let (length_m, beam_m) = bits.dimensions(240);
            static_report(
                StaticData {
                    name: bits.name(112),
                    ship_type: bits.ship_type(232),
                    call_sign: bits.text(70, 7),
                    destination: bits.text(302, 20),
                    length_m,
                    beam_m,
                    draught_m: bits
                        .uint(294, 8)
                        .filter(|d| *d > 0)
                        .map(|d| d as f64 / 10.0),
                },
                false,
            )
        }
        24 => match bits.uint(38, 2)? {
            0 => static_report(
                StaticData {
                    name: bits.name(40),
                    ..StaticData::default()
                },
                true,
            ),
            1 => {
                let (length_m, beam_m) = bits.dimensions(132);
                static_report(
                    StaticData {
                        ship_type: bits.ship_type(40),
                        call_sign: bits.text(90, 7),
                        length_m,
                        beam_m,
                        ..StaticData::default()
                    },
                    true,
                )
            }
            _ => None,
        },
        _ => None,
    }
}

/// The payload as a bit string.
struct Bits(Vec<bool>);

impl Bits {
    fn new(payload: &str, fill: u8) -> Option<Self> {
        if fill > 5 {
            return None;
        }
        let mut bits = Vec::with_capacity(payload.len() * 6);
        for byte in payload.bytes() {
            let value = match byte {
                48..=87 => byte - 48,
                96..=119 => byte - 56,
                _ => return None,
            };
            bits.extend((0..6).rev().map(|shift| value >> shift & 1 == 1));
        }
        bits.truncate(bits.len().checked_sub(usize::from(fill))?);
        Some(Self(bits))
    }

    fn uint(&self, start: usize, width: usize) -> Option<u64> {
        let field = self.0.get(start..start + width)?;
        Some(field.iter().fold(0, |acc, &bit| acc << 1 | u64::from(bit)))
    }

    fn int(&self, start: usize, width: usize) -> Option<i64> {
        let value = self.uint(start, width)? as i64;
        Some(if value >> (width - 1) == 1 {
            value - (1 << width)
        } else {
            value
        })
    }

    /// The 8-bit ship type at `start`: 1 to 99; 0 ("not available") and the
    /// reserved values above 99 are `None`.
    fn ship_type(&self, start: usize) -> Option<u8> {
        self.uint(start, 8)
            .filter(|kind| (1..=99).contains(kind))
            .map(|kind| kind as u8)
    }

    fn mmsi(&self) -> Option<u32> {
        u32::try_from(self.uint(8, 30)?).ok()
    }

    /// Twenty 6-bit characters, trimmed of `@` padding and spaces.
    fn name(&self, start: usize) -> Option<String> {
        self.text(start, 20)
    }

    /// `chars` 6-bit characters, trimmed of `@` padding and spaces; `None`
    /// when nothing is left.
    fn text(&self, start: usize, chars: usize) -> Option<String> {
        let text: String = (0..chars)
            .map(|i| {
                self.uint(start + 6 * i, 6)
                    .map(|v| char::from(if v < 32 { v as u8 + 64 } else { v as u8 }))
            })
            .collect::<Option<String>>()?;
        let text = text.trim_end_matches(['@', ' ']).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Length and beam from the antenna offsets at `start` (to bow 9 bits,
    /// to stern 9, to port 6, to starboard 6); zero is not available.
    fn dimensions(&self, start: usize) -> (Option<u16>, Option<u16>) {
        let part = |offset, width| self.uint(start + offset, width).unwrap_or(0) as u16;
        let length = part(0, 9) + part(9, 9);
        let beam = part(18, 6) + part(24, 6);
        ((length > 0).then_some(length), (beam > 0).then_some(beam))
    }

    fn position(&self, layout: &Layout) -> Option<Message> {
        let longitude = self.int(layout.lon, 28)? as f64 / 600_000.0;
        let latitude = self.int(layout.lat, 27)? as f64 / 600_000.0;
        let position = ((-180.0..=180.0).contains(&longitude)
            && (-90.0..=90.0).contains(&latitude))
        .then_some((longitude, latitude));
        let speed = self.uint(layout.sog, 10)?;
        let course = self.uint(layout.cog, 12)?;
        let heading = self.uint(layout.heading, 9)?;
        Some(Message::Position(PositionReport {
            mmsi: self.mmsi()?,
            position,
            speed_over_ground: (speed < 1023)
                .then(|| speed as f64 / 10.0 * METRES_PER_SECOND_PER_KNOT),
            course_over_ground: (course < 3600).then(|| (course as f64 / 10.0).to_radians()),
            heading: (heading < 360).then(|| (heading as f64).to_radians()),
            name: layout.name.and_then(|start| self.name(start)),
            nav_status: layout
                .nav_status
                .and_then(|start| self.uint(start, 4))
                .filter(|status| *status < 15)
                .map(|status| status as u8),
            ship_type: layout.ship_type.and_then(|start| self.ship_type(start)),
            class_b: layout.class_b,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields of `!AIVDM,count,number,seq,channel,payload,fill`.
    fn fields(text: &str) -> Vec<&str> {
        text.split(',').collect()
    }

    fn one(payload: &str, fill: u8) -> Option<Message> {
        decode(payload, fill)
    }

    // Vectors from go-gps-simulator's tests, checked there against pyais.
    const TYPE_1: &str = "13M@=r@01s02OwHMMkJ3Q2lt0000";
    const TYPE_1_NA: &str = "101r4PEP00wrFE9th2572?wp2000";
    const TYPE_18: &str = "B3M@AlP0@@0`B87GIs2`j77P0000";
    const TYPE_24A: &str = "H3M@AlPpu8@T>1=@580000000000";
    const TYPE_24B: &str = "H3M@AlTT1234001GHIJopq104220";
    const TYPE_5: &str = "53M@=r@0000048<C7;=<D61=0U8U@000000000166@D556tfN;C3kQ3kP00000000000000";

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-6)
    }

    #[test]
    fn a_class_a_position_report_is_decoded() {
        let Some(Message::Position(p)) = one(TYPE_1, 0) else {
            panic!("not a position");
        };
        assert_eq!(p.mmsi, 232001001);
        let (lon, lat) = p.position.unwrap();
        assert!((lon - 0.5461).abs() < 1e-4 && (lat - 51.4950).abs() < 1e-4);
        assert!(close(p.speed_over_ground, 12.3 * 1852.0 / 3600.0));
        assert!(close(p.course_over_ground, 90.0_f64.to_radians()));
        assert!(close(p.heading, 90.0_f64.to_radians()));
    }

    #[test]
    fn not_available_values_are_none() {
        let Some(Message::Position(p)) = one(TYPE_1_NA, 0) else {
            panic!("not a position");
        };
        assert_eq!(p.mmsi, 2000001);
        let (lon, lat) = p.position.unwrap();
        assert!((lon + 1.2345).abs() < 1e-4 && (lat + 5.6789).abs() < 1e-4);
        assert_eq!(p.heading, None, "511 = not available");
        assert!(close(p.course_over_ground, std::f64::consts::PI));
    }

    #[test]
    fn a_position_of_181_and_91_degrees_is_none() {
        // Type 1 with lon 181 (0x6791AC0) and lat 91 (0x3412140), SOG 1023
        // and COG 3600, built bit by bit here.
        let mut bits = String::new();
        let mut put = |value: u64, width: usize| {
            bits.push_str(&format!("{value:0width$b}"));
        };
        put(1, 6);
        put(0, 2);
        put(123456789, 30);
        put(0, 4);
        put(0, 8);
        put(1023, 10);
        put(0, 1);
        put(0x6791AC0, 28);
        put(0x3412140, 27);
        put(3600, 12);
        put(511, 9);
        put(0, 168 - 137);
        let payload: String = bits
            .as_bytes()
            .chunks(6)
            .map(|chunk| {
                let value = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap();
                char::from(if value < 40 { value + 48 } else { value + 56 })
            })
            .collect();
        let Some(Message::Position(p)) = one(&payload, 0) else {
            panic!("not a position");
        };
        assert_eq!(p.mmsi, 123456789);
        assert_eq!(p.position, None);
        assert_eq!(p.speed_over_ground, None);
        assert_eq!(p.course_over_ground, None);
        assert_eq!(p.heading, None);
    }

    #[test]
    fn a_class_b_position_report_is_decoded() {
        let Some(Message::Position(p)) = one(TYPE_18, 0) else {
            panic!("not a position");
        };
        assert_eq!(p.mmsi, 232002002);
        let (lon, lat) = p.position.unwrap();
        assert!((lon - 0.55).abs() < 1e-4 && (lat - 51.49).abs() < 1e-4);
        assert!(close(p.speed_over_ground, 6.5 * 1852.0 / 3600.0));
        assert!(close(p.course_over_ground, 270.0_f64.to_radians()));
        assert!(close(p.heading, 270.0_f64.to_radians()));
    }

    #[test]
    fn static_reports_give_the_name_and_the_ship_type() {
        assert_eq!(
            one(TYPE_24A, 0),
            Some(Message::Static {
                mmsi: 232002002,
                data: StaticData {
                    name: Some("NORDIC STAR".into()),
                    ..StaticData::default()
                },
                class_b: true,
            })
        );
        assert_eq!(
            one(TYPE_24B, 0),
            Some(Message::Static {
                mmsi: 232002002,
                data: StaticData {
                    ship_type: Some(36),
                    call_sign: Some("WXYZ789".into()),
                    length_m: Some(12),
                    beam_m: Some(4),
                    ..StaticData::default()
                },
                class_b: true,
            }),
            "part B: type, call sign and size (pyais: bow 8, stern 4, port 2, starboard 2)"
        );
        assert_eq!(
            one(TYPE_5, 2),
            Some(Message::Static {
                mmsi: 232001001,
                data: StaticData {
                    name: Some("SEA SPIRIT".into()),
                    ship_type: Some(70),
                    call_sign: Some("ABCD123".into()),
                    destination: Some("LONDON".into()),
                    length_m: Some(70),
                    beam_m: Some(10),
                    draught_m: Some(4.5),
                },
                class_b: false,
            }),
            "pyais: bow 50, stern 20, port 5, starboard 5, draught 4.5, LONDON"
        );
    }

    #[test]
    fn position_reports_give_the_nav_status_and_the_class() {
        let Some(Message::Position(a)) = one(TYPE_1, 0) else {
            panic!()
        };
        assert_eq!((a.nav_status, a.class_b), (Some(0), false));
        let Some(Message::Position(moored)) = one(TYPE_1_NA, 0) else {
            panic!()
        };
        assert_eq!(moored.nav_status, Some(5), "pyais: status 5");
        let Some(Message::Position(b)) = one(TYPE_18, 0) else {
            panic!()
        };
        assert_eq!((b.nav_status, b.class_b), (None, true), "class B has none");
    }

    #[test]
    fn a_two_fragment_message_is_assembled() {
        let (first, second) = TYPE_5.split_at(60);
        let mut a = Assembler::default();
        let one = format!("AIVDM,2,1,3,A,{first},0");
        let two = format!("AIVDM,2,2,3,A,{second},2");
        assert_eq!(a.push(&fields(&one)), None);
        assert_eq!(a.push(&fields(&two)), Some((TYPE_5.to_string(), 2)));
    }

    #[test]
    fn a_fragment_out_of_order_or_alone_is_dropped() {
        let (first, second) = TYPE_5.split_at(60);
        let mut a = Assembler::default();
        assert_eq!(
            a.push(&fields(&format!("AIVDM,2,2,3,A,{second},2"))),
            None,
            "a second fragment with no first"
        );
        a.push(&fields(&format!("AIVDM,2,1,3,A,{first},0")));
        a.push(&fields(&format!("AIVDM,2,1,3,A,{first},0")));
        assert_eq!(
            a.push(&fields(&format!("AIVDM,2,2,3,A,{second},2"))),
            Some((TYPE_5.to_string(), 2)),
            "a repeated first fragment starts again"
        );
    }

    #[test]
    fn a_single_fragment_message_is_passed_through() {
        let mut a = Assembler::default();
        assert_eq!(
            a.push(&fields(&format!("AIVDM,1,1,,B,{TYPE_1},0"))),
            Some((TYPE_1.to_string(), 0))
        );
    }

    #[test]
    fn bad_payloads_give_nothing() {
        assert_eq!(one("", 0), None);
        assert_eq!(one("13M@", 0), None, "too short");
        assert_eq!(
            one("13M@=r@01s02OwHMMkJ3Q2lt000~", 0),
            None,
            "bad character"
        );
        assert_eq!(one("13M@=r@01s02OwHMMkJ3Q2lt0000", 7), None, "bad fill");
        assert_eq!(
            one("83M@=r@01s02OwHMMkJ3Q2lt0000", 0),
            None,
            "type 8 is not read"
        );
        let mut a = Assembler::default();
        assert_eq!(
            a.push(&fields("AIVDM,9,1,1,A,13M@,0")),
            None,
            "too many fragments"
        );
        assert_eq!(a.push(&fields("AIVDM,x,1,1,A,13M@,0")), None);
        assert_eq!(a.push(&fields("AIVDM,1")), None);
    }
}
