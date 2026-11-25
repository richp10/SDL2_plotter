# AIS target symbology: research

How to make AIS (Automatic Identification System) targets easier to tell
apart. Sources: the OpenCPN source (`OpenCPN/OpenCPN`, `master`, read
2026-11-25) and the gpsd AIVDM document. The IEC 62288 and IMO SN.1/Circ.243
symbol sets were not read (the documents are not free); claims about them are
marked (unverified).

## 1. What OpenCPN does

### Colour: by state, not by ship type

`gui/src/ais.cpp` lines 1039–1067 choose the fill colour of a target:

| Condition | Colour name | Meaning |
|---|---|---|
| default | `UINFG` | green |
| Euro inland target | `TEAL1` | teal |
| name from the cache | `GREEN5` | green |
| no valid name yet | `CHYLW` | yellow |
| DSC distress (relayed) | `URED` | red |
| CPA/TCPA alert set | `URED` | red |
| position doubtful | `UINFF` | (unverified: grey) |

There is no colour per ship type in this code. A "colour for large ships"
is not an OpenCPN core feature (unverified: a plugin may add it). The
best-known colour-by-type convention is MarineTraffic's web map (cargo
green, tanker red, passenger blue, fishing orange, pleasure purple, tug
cyan, high-speed yellow) (unverified: no primary source read).

### Shape

- **Class A:** a pointed quad icon (`ais.cpp` 985–998).
- **Class B:** the same icon with a flat back (`ais.cpp` 1001: `if (td->Class == AIS_CLASS_B) ais_quad_icon[3].y = 0;`).
- **Real size:** with `g_bDrawAISSize`, the hull is drawn to the real dimensions A, B, C, D from the static report when it is large enough on screen (`ais.cpp` 940–971, 1576–1583).
- **Inactive (sleeping) target:** a cross-out line (`ais.cpp`, "Draw the inactive cross-out line").
- **AtoN:** a diamond, dashed when virtual, red when off position (`ais.cpp` 434–613). **Base station:** a square (`Base_Square`, 622). **SART:** its own symbol (`SART_Render`, 647).

### Navigational-status marks (day shapes)

`ais.cpp` 1588–1680 draw a small mark on the target, at target scale over
90 %. Each mark copies the day shape that the vessel shows by COLREGs:

| Status (AIS nav status) | Mark |
|---|---|
| 1 at anchor, 5 moored | one ball (circle) |
| 2 not under command | two balls |
| 3 restricted manoeuvrability | ball, diamond, ball |
| 4 constrained by draught | cylinder |
| 6 aground | three balls |
| 7 engaged in fishing | two triangles, points together |
| ship type 40–49 (HSC) or WIG | an arrow at the stern |

So the "different icon for fishing" is this mark, and it comes from nav
status 7, not from ship type 30. A fishing vessel on passage (status 0)
gets no mark.

## 2. AIS fields needed

From gpsd AIVDM (https://gpsd.gitlab.io/gpsd/AIVDM.html):

| Field | Where | Bits |
|---|---|---|
| Navigational status | types 1, 2, 3 | 38–41 |
| Ship type | type 5 | 232–239 |
| Ship type | type 19 | 263–270 |
| Ship type | type 24 part B | 40–47 |
| Dimensions A, B, C, D | type 5 | 240–269 |
| Class | type 18, 19, 24 = B; 1, 2, 3, 5 = A | message type |

Ship type: first digit is the category (3x special, 4x HSC, 5x service,
6x passenger, 7x cargo, 8x tanker); 30 fishing, 31–32 towing, 35 military,
36 sailing, 37 pleasure, 50 pilot, 51 SAR, 52 tug, 55 law enforcement.

Signal K carries the same as `design.aisShipType` (`{id, name}`),
`navigation.state` (text, for example `moored`, `fishing`), `design.length`,
`design.beam` and `sensors.ais.class` (unverified: paths from memory of
the Signal K specification; check before coding).

The plotter decodes none of these today (`src/nmea0183/aivdm.rs` reads
position, SOG, COG, heading and name only).

## 3. Night use

The night overlay tints the whole frame red. Dark blue and dark green go
nearly black under it; light, saturated colours stay apart. The own boat
is bright red, so no target colour may be red or near it. IEC 62288 day,
dusk and night palettes exist (unverified: not read).

## 4. Options

| | A. Colour by ship category | B. OpenCPN-like | C. Both |
|---|---|---|---|
| Colour | per category (table below) | one colour; yellow until named | per category |
| Status marks | none | day shapes (section 1) | day shapes |
| Class B | same hull | flat-backed hull | flat-backed hull |
| Decoding work | ship type | nav status, class | ship type, nav status, class |
| Answers "large ships stand out" | yes | no | yes |
| Answers "fishing looks different" | only by type 30 | yes, by status 7 | yes |

Proposed colours (none near red; light enough for the night overlay):

| Category | Ship types | RGB |
|---|---|---|
| Cargo | 70–79 | 0, 170, 0 (green) |
| Tanker | 80–89 | 200, 0, 200 (magenta) |
| Passenger | 60–69 | 0, 120, 255 (blue) |
| High-speed craft | 40–49 | 240, 220, 0 (yellow) |
| Tug, pilot, SAR, law (service) | 50–59 | 0, 200, 210 (cyan) |
| Fishing | 30 | 255, 140, 0 (orange) |
| Sailing, pleasure | 36, 37 | 160, 100, 230 (violet) |
| Other, unknown, no static report yet | everything else | 150, 150, 150 (grey) |

**Recommendation: C.** Colour answers "what is it, how big"; the day-shape
marks answer "what is it doing", and they are what the navigator sees on
the water by day.

## 5. What the simulator must send to test option C

Today it sends types 1, 18, 5, 24 with ship types 30, 36, 37, 60, 70, 80
and nav status always 0 (`gps/ais.go` 118–126, 343).

1. **Nav status per Class A target** (type 1): 1 at anchor (SOG 0), 5 moored
   (SOG 0), 7 engaged in fishing (ship type 30, SOG 2–4 kn, turning), 2 not
   under command, 3 restricted manoeuvrability, 4 constrained by draught
   (tanker), 6 aground (SOG 0), 8 under way sailing (ship type 36). At
   least one target each.
2. **More ship types:** 31 or 32 (towing), 40 (HSC, fast: 25–35 kn), 50
   (pilot), 52 (tug), 55 (law enforcement), 35 (military), 0 (not
   available).
3. **One fishing vessel on passage:** ship type 30 with nav status 0, to
   show the type colour without the fishing mark.
4. **One Class A target that never sends type 5**, so it has no name and no
   type (grey).
5. **Class B targets** of types 36 and 37 (types 18 and 24), as now.
6. Option: a `-ais-status-cycle` flag that changes some targets' nav status
   every few minutes (under way, then at anchor), to test a live change.
