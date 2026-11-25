# To do

Work that is known but not started. Highest priority first.

## Next round

From the NMEA 0183 work (2026-11-25). See `docs/nmea0183-input-design.md`.

- **Check by eye.** The red and grey boat marker, the track and the AIS
  boat shapes were checked from the log and the unit tests only.
- **Heading line never seen live.** The simulator sends HDG with no
  variation and no HDT, so no heading is shown. Only unit tests cover it.
- **A bad `nmea0183.address` stops the start.** A hostname or a typo fails
  `AppConfig::validate`, and `main` exits. The Signal K URL has the same
  rule. This breaks "degrade, never exit".
- **An unknown `[input] source` word stops the start.** For example
  `source = "nmea"` is a toml parse error, and the app exits.
- **UDP broadcast not tested.** Only unicast (`gps-simulator -udp -host
  127.0.0.1`) was tested, not the simulator's default broadcast.
- **No serial transport.** A USB GPS or a serial multiplexer is not
  supported (`tokio-serial` would be needed).
- **No GGA or GLL fallback.** A device that sends no RMC gives no
  position.
- **NMEA 2000 path not decided.** SocketCAN on the Pi, or a gateway over
  IP (Yacht Devices RAW, Actisense W2K-1). `nmea2k` now logs an ERROR.
  Research: `docs/nmea0183-input-research-2026-11-25.md` section 6.

## Signal K gaps (found before the NMEA 0183 work)

- **No reconnect when the connection goes quiet.** A WebSocket that dies
  without a Close (Wi-Fi lost) is never reconnected. The marker goes grey
  after 10 s, but no new data comes. The NMEA 0183 TCP client has a 30 s
  read timeout; Signal K has none.
- **The hello is not read after a reconnect,** so `self_context` from the
  first server stays in use.
- **Own SOG is not range-checked:** a negative value is accepted.

## Other

- **Flaky test:** `stream::tests::submitted_pixels_are_encoded_off_the_calling_thread`
  asserts `submit` takes under 5 ms, and failed once under a full parallel
  test run. It passes alone.
- **Errors other than drawing still stop the main loop.** A failed SDL
  event, cursor or texture re-creation (`Plotter::resize`) still reaches
  `main_loop` through `?`. Drawing and stream capture no longer do.
- **AIS from the simulator lacks edge cases.** It does not send a target
  that goes silent, "not available" values or tag blocks
  (`docs/simulator-ais-request.md`). Unit tests cover them.
