# NMEA 0183 input: design

The plotter can take live boat data from NMEA 0183 over TCP or UDP, as an
alternative to Signal K. Signal K works as before. The code has room for an
NMEA 2000 input later.

Research: `docs/nmea0183-input-research-2026-11-25.md`.
Later work: `docs/todo.md`.

**Done when:** with `source = "nmea0183"`, the plotter shows the boat
from `go-gps-simulator` (TCP and UDP both). The boat marker moves, the COG
(course over ground) vector shows, and the track grows. When the simulator
stops, the marker goes grey within 10 s. With `source = "signalk"`, the
behaviour is the same as now.

## Decisions

| # | Decision |
|---|---|
| 1 | One source at a time, chosen at start. No change while the app runs, no menu entry. |
| 2 | `[input] source = "signalk" \| "nmea0183" \| "nmea2k"`. Missing = `"signalk"`, so an old `plotter.toml` works with no change. |
| 3 | `[signalk]` does not change, except one new path (decision 14). |
| 4 | `[nmea0183]`: `transport = "tcp" \| "udp"`, `address`. TCP: the plotter is a client and connects to `address`. UDP: the plotter binds `address` and listens. Default `transport = "tcp"`, `address = "127.0.0.1:10110"` (IANA port `nmea-0183`). |
| 5 | `AppConfig::validate` checks only the table of the selected source. |
| 6 | `nmea2k` is accepted by the config. At start the app logs an ERROR ("not supported yet") and runs with no live data. |
| 7 | AIS (Automatic Identification System) over NMEA 0183 was not in the first version. Added later: `!AIVDM` types 1, 2, 3, 5, 18, 19 and 24 (`nmea0183/aivdm.rs`). |
| 8 | RMC is the only source of position, COG and SOG. GGA, GLL and VTG are ignored. One RMC with a fix = one track point at most. |
| 9 | A fix is RMC status `A` with a mode indicator that is not `N` (or no mode field, NMEA before 2.3). Anything else, mode `E` (dead reckoning) included, is no fix. A no-fix RMC does not change the position. |
| 10 | Heading: HDT (true) first. Else HDG converted to true with the variation in HDG, else the variation in the last RMC. With no variation, no heading. A magnetic value is never shown as true. |
| 11 | Heading draws a heading line on the boat marker. The COG vector stays. Course-up rotation stays on COG. |
| 12 | The talker ID (`GP`, `GN`, `II`, ...) is ignored. Sentences match on the 3-letter type. |
| 13 | Fix timeout: no position for 10 s = no fix. Heading timeout: no heading for 10 s = no heading. Both apply to every source, in `Session`. |
| 14 | Signal K gets heading from a new path `heading`, default `navigation.headingTrue` (radians). |
| 15 | Boat marker: `boat_color` default becomes red `[255, 0, 0, 255]` (code and `default/plotter.toml`). New `boat_no_fix_color`, default grey `[128, 128, 128, 255]`. |
| 16 | With no fix, the marker is drawn grey at the last position that had a fix, with no COG vector and no heading line. Before the first fix, no marker. Follow mode keeps the view where it is. |
| 17 | A source task that ends (or a closed channel) never stops the app. `state_rx.has_changed()?` in `app.rs` is replaced: a closed channel logs an ERROR once and the loop continues. |

## Structure

```
config ─► Plotter::new ─match source─► signalk::run(config, feed)
                                   ├─► nmea0183::run(config, feed)
                                   └─► nmea2k: ERROR, no task
                                          │
                     LiveFeed { boat: watch, track: mpsc, ais: watch }
                                          │
                     main loop drains ─► Session / Track (unchanged path)
```

- **`LiveFeed`** (new, small): the three senders that exist now, in one
  struct. Every source gets one. It is the seam a future NMEA 2000 input
  uses.
- **`nmea0183` module** (new), two parts:
  - **Transport** (async, no tests without a socket): the TCP client
    reconnects with the same back-off as Signal K and has a read timeout of
    30 s, after which it reconnects. The UDP listener only receives. Both
    give lines of text, at most 256 bytes (an 82-character sentence plus a tag block); a longer line is dropped.
  - **Decoder** (pure, tested): `Nmea0183::apply(line) -> Update`. It skips
    a tag block (`\...\`), checks the start character and the `*hh` XOR
    checksum, and rejects a line with no checksum. It decodes RMC, HDT and
    HDG, validates every field and coordinate, accepts extra fields (the
    simulator sends them), and converts units: knots × 1852/3600 to m/s,
    degrees to radians, `ddmm.mmmm` to decimal degrees. An empty COG or SOG
    field leaves the last value. It keeps the last values and the last RMC
    variation.
- **`BoatState`** gains `heading: Option<f64>` (radians, true). The source
  stamps when the fix and the heading were received; `Session` decides if
  they are still fresh (decision 13), with injected `Instant`s, the same as
  `expire_ais`. A change to stale marks the frame dirty.
- **`Shape::Boat`** gains `heading: Option<f64>` and `fix: bool`. `Screen`
  colours the marker from `boat_color` or `boat_no_fix_color`.

## Tests (seams)

- Decoder: a valid RMC from the simulator; status `V`; mode `N` and `E`;
  bad checksum; no checksum; a tag block; extra fields; out-of-range
  latitude and longitude; empty COG and SOG; HDT; HDG with variation, with
  RMC variation, and with none; GGA, GLL and VTG give no update; unit
  conversions against independent values (1 kn = 0.514444 m/s, 4807.038 N =
  48.1173°).
- `Session`: the fix goes stale after 10 s and the heading after 10 s
  (injected `Instant`s); a fresh fix after a stale one is valid again.
- `boat.shape()` / `overlay::plan`: a grey marker with no vectors without a
  fix; no marker before the first fix; a heading line when heading is
  present.
- Config: `source` missing = `signalk`; `nmea0183` defaults; a bad
  `transport` or `address` is rejected; the Signal K table is not checked
  when `nmea0183` is selected; round trip and defaults.

## Smoke test

1. `F:\projects\go-gps-simulator\gps-simulator.exe -tcp` (see `--help`).
   Set `source = "nmea0183"`, `transport = "tcp"`, `address =
   "127.0.0.1:10110"`. Run the release build. The marker is red, moves, and
   shows the COG vector; the track grows.
2. Stop the simulator. Within 10 s the marker goes grey. Start it again:
   the plotter reconnects and the marker goes red.
3. Repeat with `-udp`, `transport = "udp"`, `address = "0.0.0.0:10110"`.
4. `source = "signalk"`: the log shows the same Signal K messages as now.
5. `source = "nmea2k"`: an ERROR in the log; the app runs with no boat.
