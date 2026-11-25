# To do

Work that is known but not started. Highest priority first.

## Next round

From the NMEA 0183 work (2026-11-25). See `docs/nmea0183-input-design.md`.

- **Check by eye.** The red and grey boat marker, the track and the AIS
  boat shapes were checked from the log and the unit tests only.
- **No serial transport.** A USB GPS or a serial multiplexer is not
  supported (`tokio-serial` would be needed).
- **NMEA 2000 path not decided.** SocketCAN on the Pi, or a gateway over
  IP (Yacht Devices RAW, Actisense W2K-1). `nmea2k` now logs an ERROR.
  Research: `docs/nmea0183-input-research-2026-11-25.md` section 6.

## Other

- **Other config errors still stop the start.** A toml syntax error, or a
  bad value outside the input tables (window, appearance, rendering,
  streaming), fails `AppConfig::load_or_create`, and `main` exits.

- **A panic still stops the app.** No `Err` ends the main loop now, but a
  panic on the main thread (an index out of range, an `unwrap`) does.
- **AIS from the simulator lacks edge cases.** It does not send a target
  that goes silent, "not available" values or tag blocks
  (`docs/simulator-ais-request.md`). Unit tests cover them.
