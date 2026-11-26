# Simulator: AIS output wanted

What the plotter needs from `go-gps-simulator` to test its NMEA 0183 AIS
(Automatic Identification System) input. AIS decoding in the plotter is
planned for later (see `docs/todo.md`).

## Output

- Send `!AIVDM` sentences on the same TCP or UDP stream as the GPS
  sentences, between them. Talker `AI`. Correct `*hh` checksum. CRLF.
- Use channel `A` and `B` both.
- Each sentence is at most 82 characters. A long message is split into
  fragments with a shared sequence ID.

## Message types

| Type | What | Rate |
|---|---|---|
| 1 (or 3) | Class A position: MMSI, lat, lon, SOG, COG, heading | every 2 to 10 s for each target |
| 18 | Class B position | every 30 s for each target |
| 5 | Class A static: name, call sign, ship type (2 fragments) | every 6 min, or faster with an option |
| 24 A and B | Class B static: name (part A), call sign and type (part B) | every 6 min, or faster with an option |

## Targets

- An option for the number of targets, for example `-ais 5`. Default 0
  (no AIS, the same as now).
- Each target has its own MMSI (9 digits) and its own name.
- Targets move on straight or curved paths within about 5 NM (nautical
  miles) of the own boat. Some are Class A, some are Class B.

## Cases the plotter must handle

- One target that stops sending after some minutes. The plotter must
  remove it after 5 min.
- One target that sends "not available" values: lon 181, lat 91,
  SOG 1023 (102.3 kn), COG 3600 (360.0°), heading 511.
- One Class A target that sends no type 5, so it has no name.
- An option to put a tag block (`\s:sim,c:<unix time>*hh\`) before each
  sentence, to test that the plotter skips it.
