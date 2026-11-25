# To do

Work that is known but not started. Highest priority first.

## Normal

- **A render error stops the app.** On 2026-11-25 a Windows smoke run
  stopped with `GetRenderTargetData(): DEVICELOST` (a Direct3D device
  loss) from `canvas.read_pixels` in `Streamer::capture`. Errors from
  `display` reach `main_loop` through `?` and end it. This breaks "degrade,
  never exit". A failed capture must skip that stream frame; a lost device
  must be logged, and drawing must continue or the renderer must be made
  again.
- **AIS over NMEA 0183.** Decode `!AIVDM` (types 1, 2, 3, 5, 18, 19, 24)
  in the NMEA 0183 input, with fragment assembly. Research:
  `docs/nmea0183-input-research-2026-11-25.md` section 3. Test data:
  `docs/simulator-ais-request.md`.
- **AIS data must be bounded.** Targets are removed after 5 min without a
  position update (`ais::TARGET_TIMEOUT`), but no rule limits how much
  AIS data is kept. Static data (name, call sign) must expire with the
  target, and the number of targets must have a limit. The crate
  `nmea-parser` keeps static data for each MMSI until `reset()`, so it
  must not be used as it is. The app runs for weeks.

## Low

- **Heading-up mode.** Rotate the chart to the heading, as an
  alternative to course-up. Now course-up uses COG only
  (`docs/nmea0183-input-design.md`, decision 11).

- **Talker filter for NMEA 0183.** An optional `talker = "GN"` setting,
  for a multiplexer that mixes two GPS receivers. Now the talker is
  ignored.
