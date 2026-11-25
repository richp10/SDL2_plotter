# To do

Work that is known but not started. Highest priority first.

## Next round

From the NMEA 0183 work (2026-11-25). See `docs/nmea0183-input-design.md`.

- **Check by eye.** The red and grey boat marker, the track and the AIS
  boat shapes were checked from the log and the unit tests only.
- **UDP broadcast not tested.** Only unicast (`gps-simulator -udp -host
  127.0.0.1`) was tested, not the simulator's default broadcast.
- **No serial transport.** A USB GPS or a serial multiplexer is not
  supported (`tokio-serial` would be needed).
- **Old VTG form not read.** A VTG with no unit letters (before NMEA 2.3:
  `$--VTG,054.7,034.4,005.5,010.2`) gives no speed or course.
- **NMEA 2000 path not decided.** SocketCAN on the Pi, or a gateway over
  IP (Yacht Devices RAW, Actisense W2K-1). `nmea2k` now logs an ERROR.
  Research: `docs/nmea0183-input-research-2026-11-25.md` section 6.

## Other

- **Other config errors still stop the start.** A toml syntax error, or a
  bad value outside the input tables (window, appearance, rendering,
  streaming), fails `AppConfig::load_or_create`, and `main` exits.

- **Flaky test:** `stream::tests::submitted_pixels_are_encoded_off_the_calling_thread`
  asserts `submit` takes under 5 ms, and failed once under a full parallel
  test run. It passes alone.
- **Errors other than drawing still stop the main loop.** A failed SDL
  event, cursor or texture re-creation (`Plotter::resize`) still reaches
  `main_loop` through `?`. Drawing and stream capture no longer do.
- **AIS from the simulator lacks edge cases.** It does not send a target
  that goes silent, "not available" values or tag blocks
  (`docs/simulator-ais-request.md`). Unit tests cover them.
