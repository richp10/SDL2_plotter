# NMEA 0183 input for SDL2 Plotter: research

Date: 2026-11-25. Research for a new setting that selects the live data source: Signal K (as now), NMEA 0183, and later NMEA 2000. Sources are primary where possible: gpsd's protocol pages, the IANA port registry, crates.io and the crate sources, the canboat repository and `canboat.json`, the Yacht Devices manual, and the source of the simulator at `F:\projects\go-gps-simulator`. Paths with line numbers point to code. **(unverified)** marks a statement that no source in this file confirms.

Terms used in this file:

- **COG**: course over ground. **SOG**: speed over ground.
- **AIS**: Automatic Identification System. **MMSI**: Maritime Mobile Service Identity, the 9-digit vessel number.
- **PGN**: Parameter Group Number, the message number in NMEA 2000.
- **Fix**: a position that the receiver says is valid.

## Summary

- NMEA 0183 is one ASCII line per sentence, with an XOR checksum. A parser for the four position sentences (RMC, GGA, GLL, VTG) is small. AIS (`!AIVDM`) is a separate layer with 6-bit payloads and fragments.
- Port 10110 TCP and UDP is registered at IANA as `nmea-0183`. Signal K server, OpenPlotter and the simulator all use it.
- The simulator sends GGA, RMC, GLL, VTG, GSA, GSV, ZDA and marine instrument sentences once a second. It is a TCP **server** or a UDP **broadcaster** on port 10110. It sends no AIS. The plotter must therefore be a TCP client or a UDP listener.
- The `nmea` crate parses `$` sentences only. It rejects `!AIVDM` and tag blocks. `nmea-parser` does both, but its last release is June 2024. A hand-written parser for RMC/GGA/GLL/VTG needs no new dependency.
- NMEA 2000 later can use the same shape: one task that owns a transport and a decoder and publishes the same three channels. PGN 129026 already gives COG in radians and SOG in m/s.
- **Hazard in the current code:** `app.rs:329` calls `self.state_rx.has_changed()?`. If the source task ends and drops its sender, the main loop returns an error and the app quits. A new source task must never return, or that line must change.

## 0. What the app has now

| Item | Fact | Where |
|---|---|---|
| Source task | `signalk::signalk_task(state_tx, track_tx, ais_tx, config.signalk)` is spawned on tokio | `src/app.rs:115-123` |
| Boat channel | `watch<BoatState>`: lon, lat, `position_valid`, SOG (m/s), COG (radians) | `src/boat.rs:5-12`, `src/app.rs:115` |
| Track channel | unbounded `mpsc<(lon, lat)>`, one item per new own position | `src/signalk.rs:400-402` |
| AIS channel | `watch<Vec<AisTarget>>`; `AisTarget` has `context`, `name`, `position`, COG, SOG | `src/ais.rs:25-32` |
| AIS expiry | `retain_fresh` runs at most once a second in the task; timeout 5 min | `src/signalk.rs:404-411`, `src/ais.rs:7` |
| Coordinate check | finite, lat in -90..=90, lon in -180..=180 before `position_valid = true` | `src/signalk.rs:240-247`, `296-299` |
| Fix loss | the Signal K path never sets `position_valid` back to false | `grep position_valid = false` finds only tests |
| Reconnect | WebSocket retry with a delay of `attempt.min(10)` seconds; HTTP 429 backs off to 60 s | `src/signalk.rs:326-365` |
| Main loop drain | `state_rx.has_changed()?` (quits on a closed channel); `ais_rx.has_changed().unwrap_or(false)` (does not) | `src/app.rs:329`, `src/app.rs:338` |
| Dependencies | `tokio` with `full` features (includes `net`: `TcpStream`, `UdpSocket`) | `Cargo.toml:13` |

A tokio `watch::Receiver::has_changed` returns an error when the channel is closed, that is, when the sender is dropped ([tokio docs](https://docs.rs/tokio/latest/tokio/sync/watch/struct.Receiver.html#method.has_changed)).

## 1. NMEA 0183 framing

### 1.1 The sentence

| Part | Rule | Source |
|---|---|---|
| Start | `$` (ASCII 36). AIS data uses `!`. Up to NMEA 4.00 these are the only start characters. | [gpsd NMEA Revealed, "NMEA Encoding Conventions"](https://gpsd.gitlab.io/gpsd/NMEA.html#_nmea_encoding_conventions) |
| First field ("tag") | 2-letter talker ID + 3-letter sentence type, for example `GPRMC` | same |
| Fields | comma-separated; an empty field means "no valid data". Many receivers send zeros instead. | same, and "Error status indications" |
| Checksum | `*` then 2 hex digits (0-9, A-F), high nibble first. It is the 8-bit XOR of all characters between the start character and `*`. It includes every `,` and `^`. gpsd calls it mandatory. | same. (The gpsd text lists the excluded characters as `$`, `I`, `*`; the `I` is a typo for `!`: gpsd's AIVDM page says the checksum is computed "excluding the leading `!`".) |
| End | `<CR><LF>` | same |
| Length | maximum 82 bytes, including the `$` and `<CR><LF>` | same |
| Coordinates | `ddmm.mmmm` for latitude, `dddmm.mmmm` for longitude. The two digits left of the decimal point are whole minutes. The number of decimals depends on the device. | same |

The simulator computes the checksum the same way: XOR from index 1 (after `$`) to the end, then `*%02X\r\n` (`go-gps-simulator/gps/nmea.go:11-22`).

### 1.2 The standard

- NMEA 0183 is sold by NMEA. The current version appears to be 4.30 (December 2023). The international equivalent is IEC 61162-1, sold by DIN ([user's notes](NMEA%200183-NMEA%202000-Signal%20K-SEC.md); [IEC 61162-1 at DIN](https://www.din.de/en/getting-involved/standards-committees/dke/publications/wdc-beuth:din21:143482391)).
- gpsd says that IEC 61162-1 confirms the "mode indicator dominates status" rule in §2.2 below ([gpsd NMEA Revealed](https://gpsd.gitlab.io/gpsd/NMEA.html)).
- go-nmea takes its sentence list from the IEC 61162-1:2016 table of contents ([adrianmo/go-nmea](https://github.com/adrianmo/go-nmea)).

### 1.3 Tag blocks

- From NMEA 4.10 a sentence can have a "tag block" in front of it: `\` + comma-separated `key:value` fields + `*hh` + `\`. Example: `\g:1-2-73874,n:157036,s:r003669945,c:1241544035*4A\!AIVDM,1,1,,B,15N4cJ`005Jrek0H@9n`DW5608EP,0*13` ([gpsd AIVDM, "NMEA Tag Blocks"](https://gpsd.gitlab.io/gpsd/AIVDM.html#_nmea_tag_blocks)).
- Keys include `c` (UNIX time), `s` (source), `g` (group), `n` (line count). A tag block field may not contain a backslash (same source).
- **How to skip it:** if a line starts with `\`, find the second `\` and parse from the next character. The sentence checksum does not cover the tag block. A simpler rule, used by `nmea-parser`, is to discard every byte before the first `$` or `!` (`nmea-parser-0.11.0/src/lib.rs:277-287`).
- The `nmea` crate does not do this. It requires `$` as the first character (`nmea-0.8.0/src/parse.rs:80`).

### 1.4 Rules for a robust line reader

These are design consequences of the facts above, not quotes:

- Split the byte stream on `\n`. Strip a trailing `\r`. Accept a line that has no `\r`.
- Discard a line longer than about 100 bytes without parsing it. (The `nmea` crate uses 102: `SENTENCE_MAX_LEN`, `nmea-0.8.0/src/parse.rs:32`.) Then a stream with no newline cannot grow a buffer without limit.
- Reject a line with no `*hh` or a wrong checksum, and count it. gpsd notes some devices send garbage with a valid checksum (quoted in `nmea-0.8.0/src/parse.rs:20`), so the field checks in §2 are still necessary.
- Over UDP, one datagram can hold one or several lines. The simulator sends one sentence per datagram (`cmd/gps-simulator/network.go:48-50`, one `Write` per `fmt.Fprint`).

## 2. Sentences for position, COG and SOG

### 2.1 Fields

| Sentence | Fields (1-based after the tag) | Validity signal | Source |
|---|---|---|---|
| RMC | 1 UTC time, 2 status `A`=valid / `V`=warning, 3-4 lat + N/S, 5-6 lon + E/W, 7 SOG knots, 8 COG degrees true, 9 date `ddmmyy`, 10-11 magnetic variation + E/W, 12 mode indicator (NMEA 2.3+), 13 nav status (NMEA 4.1+) | status `A`, and mode not `N` | [gpsd RMC](https://gpsd.gitlab.io/gpsd/NMEA.html#_rmc_recommended_minimum_navigation_information) |
| GGA | 1 UTC time, 2-3 lat, 4-5 lon, 6 quality, 7 satellites, 8 HDOP, 9-10 altitude + M, 11-12 geoid separation + M, 13 DGPS age, 14 station | quality `0` = fix not available; 1 GPS, 2 DGPS, 3 PPS, 4 RTK, 5 float RTK, 6 estimated, 7 manual, 8 simulation | [gpsd GGA](https://gpsd.gitlab.io/gpsd/NMEA.html#_gga_global_positioning_system_fix_data) |
| GLL | 1-2 lat, 3-4 lon, 5 UTC time, 6 status `A`/`V`, 7 mode (2.3+) | status `A`, mode not `N` | [gpsd GLL](https://gpsd.gitlab.io/gpsd/NMEA.html#_gll_geographic_position_latitudelongitude) |
| VTG | 1 COG true + `T`, 3 COG magnetic + `M`, 5 SOG knots + `N`, 7 SOG km/h + `K`, 9 mode (2.3+). Old form: `$--VTG,x.x,x,x.x,x.x` with no unit letters | mode not `N` | [gpsd VTG](https://gpsd.gitlab.io/gpsd/NMEA.html#_vtg_track_made_good_and_ground_speed) |
| HDT | 1 heading degrees true + `T`. gpsd marks it "Deprecated 2008". | none | [gpsd HDT](https://gpsd.gitlab.io/gpsd/NMEA.html#_hdt_heading_true) |
| HDG | heading magnetic, deviation, variation | none | [gpsd HDG](https://gpsd.gitlab.io/gpsd/NMEA.html#_hdg_heading_deviation_variation) |

Mode indicator values: `A` autonomous, `D` differential, `E` estimated (dead reckoning), `M` manual, `S` simulator, `N` not valid. In pre-2.3 sentences the field is absent. gpsd: the mode indicator "dominates the Status field": status is `A` only for modes `A` and `D` ([gpsd, FAA Mode Indicator](https://gpsd.gitlab.io/gpsd/NMEA.html)).

Facts for the plotter:

- **RMC alone gives everything `BoatState` holds**: position, validity, SOG and COG. It is the sentence that GPS units most commonly send (gpsd, RMC section).
- Heading (HDT/HDG) is not in `BoatState` today. The app draws COG only (`src/boat.rs:28-33`). HDG is magnetic and needs variation to become true.
- COG in RMC/VTG can be empty when the boat is still. The gpsd example `$GNRMC,001031.00,A,4404.13993,N,12118.86023,W,0.146,,100117,,,A*7B` has SOG 0.146 and an empty COG.
- RMC status `V` "means the GPS has a valid fix that is below an internal quality threshold" (gpsd). For a safety display, treat `V` as no fix.

### 2.2 Unit conversions

| From | To the app | Formula | Source |
|---|---|---|---|
| `ddmm.mmmm`, N/S | latitude degrees | `dd + mm.mmmm / 60`, negative for S | gpsd encoding conventions |
| `dddmm.mmmm`, E/W | longitude degrees | `ddd + mm.mmmm / 60`, negative for W | same |
| knots | m/s | `× 1852 / 3600` (= 0.514444...) | 1 nautical mile = 1852 m, 1 knot = 1 nmi/h ([BIPM SI Brochure, Table 8](https://www.bipm.org/en/publications/si-brochure)) |
| km/h (VTG field 7) | m/s | `÷ 3.6` | definition |
| degrees true | radians | `× π / 180` | definition |

Then apply the boundary rule from `CLAUDE.md`: finite, lat in -90..=90, lon in -180..=180. Also reject minutes ≥ 60 and a hemisphere letter other than N/S/E/W. One edge case: the simulator truncates degrees and prints minutes with `%07.4f`, so a value of 59.99996 minutes prints as `60.0000` (`gps/nmea.go:76-79, 102`). A strict check drops that one sentence; the next one is good.

### 2.3 Talker IDs

- gpsd lists: `GP` GPS, `GL` GLONASS, `GA` Galileo, `GB`/`BD` BeiDou, `GI` NavIC, `GQ` QZSS, `GN` "combination of multiple satellite systems", `II` integrated instrumentation, `HC` heading/compass, `AI` (AIS), and more ([gpsd, Talker IDs](https://gpsd.gitlab.io/gpsd/NMEA.html#_talker_ids)).
- A modern multi-constellation receiver sends `GN` (gpsd RMC example above). A multiplexer can send `II` or `IN`. **(unverified for `IN`)**
- **Consequence:** match on the 3-letter sentence type and ignore the talker. The `nmea` crate and `nmea-parser` both do this.
- Risk: a multiplexer that merges two GPS receivers sends two RMC streams. Without a talker filter, the boat jumps between them. A talker filter in the config is an option (open question 4).

### 2.4 Which sentence feeds which channel

- One receiver often sends RMC, GGA, GLL and VTG for the same instant. The simulator sends all four every second (`gps/simulator.go:463-476`).
- If every sentence with a position sends a track point, the track gets 3 points per second for one fix. The design must choose one sentence as the position source, or drop a point at the same position and time.
- A simple rule (design choice, not from a source): RMC is primary; use GGA/GLL only if no RMC has arrived for some seconds; take SOG/COG from RMC, else from VTG.

## 3. AIS over NMEA 0183

### 3.1 Sentence layer

From [gpsd AIVDM, "AIVDM/AIVDO Sentence Layer"](https://gpsd.gitlab.io/gpsd/AIVDM.html#_aivdmaivdo_sentence_layer):

- `!AIVDM,1,1,,B,177KQJ5000G?tO`K>RA1wUbN0TKH,0*5C`. Field 1 fragment count; 2 fragment number (1-based); 3 sequential message ID for multi-fragment messages (empty otherwise); 4 radio channel `A`/`B` (also `1`/`2` in the wild); 5 payload; 6 fill bits (0-5) to ignore at the end of the payload.
- Fragments exist because of the 82-character limit. A type 5 message takes two sentences: `!AIVDM,2,1,3,B,55P5TL01VIaAL@7WK...,0*3E` then `!AIVDM,2,2,3,B,1@0000000000000,2*55`.
- `AIVDM` is a report from other vessels. `AIVDO` is own vessel. `nmea-parser` exposes this as `own_vessel: bool` ("True if the data is about own vessel", `nmea-parser-0.11.0/src/ais/mod.rs:127-128`).

### 3.2 Payload armouring

- Each payload character carries 6 bits: subtract 48; if the result is greater than 40, subtract 8. Valid characters are `0` to `w`, without `X` to `_` ([gpsd, "Payload Armoring"](https://gpsd.gitlab.io/gpsd/AIVDM.html#_aivdmaivdo_payload_armoring)).
- Concatenate the 6-bit values MSB first into one bit vector, then read fields by bit offset (same source).
- Text fields ("six-bit ASCII"): values 0-31 map to `@`..`_`, 32-63 map to space..`?`. `@` ends the text; strip trailing spaces ([gpsd, Sixbit ASCII and trailing `@`](https://gpsd.gitlab.io/gpsd/AIVDM.html)).

### 3.3 Fields needed for `AisTarget`

Bit offsets are from the gpsd tables for each type.

| Type | Content | MMSI | SOG | Lon | Lat | COG | Name |
|---|---|---|---|---|---|---|---|
| 1, 2, 3 | Class A position | 8-37 | 50-59 | 61-88 (signed) | 89-115 (signed) | 116-127 | - |
| 18 | Class B position | 8-37 | 46-55 | 57-84 | 85-111 | 112-123 | - |
| 19 | Extended Class B position | 8-37 | 46-55 | 57-84 | 85-111 | 112-123 | 143-262 |
| 5 | Class A static and voyage (2 fragments) | 8-37 | - | - | - | - | 112-231 (20 chars) |
| 24 | Class B static; part number 38-39; part A has the name | 8-37 | - | - | - | - | 40-159 (part A) |

Scales and "not available" values ([gpsd, Types 1, 2 and 3](https://gpsd.gitlab.io/gpsd/AIVDM.html#_types_1_2_and_3_position_report_class_a)):

- Longitude and latitude are in 1/10000 minute: divide by 600000.0 for degrees. Lon 181° (`0x6791AC0`) and lat 91° (`0x3412140`) mean "not available".
- SOG is in 0.1 knot. 1023 means not available. 1022 means 102.2 knots or more.
- COG is in 0.1°. 3600 (`0xE10`) means not available.
- Heading 511 means not available. The app does not use heading today.

Mapping to the app: `context` is a free string today (Signal K gives `vessels.<id>`). A form such as `urn:mrn:imo:mmsi:<mmsi>` keeps it unique per vessel **(Signal K naming not verified in this file)**. SOG goes to m/s and COG to radians as in §2.2.

### 3.4 Is AIS in scope for a first version? (facts only)

For:

- The AIS channel, the drawing and the 5-minute expiry already exist. A decoder only fills `Vec<AisTarget>` (`src/ais.rs`, `src/signalk.rs:404-414`).
- A crate can decode it (§5).

Against:

- The simulator sends no AIS. A `grep` for `AIVDM` or `AIS` in its Go source finds nothing. Tests need recorded sentences, for example the gpsd examples above.
- AIS adds fragment assembly, 6-bit decoding, 5 message types and per-MMSI state. This is more code than RMC/GGA/GLL/VTG together.
- An AIS receiver often sits on its own port or serial line (38400 baud is common for AIS **(unverified)**). A multiplexer or Signal K server merges it with GPS.
- If the boat has Signal K, AIS already works through it.

## 4. Transport

### 4.1 Port 10110

The IANA registry lists `nmea-0183 10110/tcp` and `nmea-0183 10110/udp`, "NMEA-0183 Navigational Data", registered 2009-02-18 by Meindert Sprang. Port 10111/udp is `nmea-onenet`, "NMEA OneNet multicast" ([IANA service names and port numbers](https://www.iana.org/assignments/service-names-port-numbers/service-names-port-numbers.txt)).

### 4.2 Who listens, who connects

| Device or program | Role | Port | Source |
|---|---|---|---|
| Signal K server | TCP **server**, created by default; sends all multiplexed NMEA 0183 input (`nmea0183` event) and the converted output of `signalk-to-nmea0183` (`nmea0183out`). Its UDP connections are inbound only. | 10110 | [OpenPlotter, NMEA 0183 multiplexing](https://openplotter.readthedocs.io/latest/signalk/multiplexing.html) |
| Signal K server issue | the raw stream on 10110 bypasses source priorities (server 2.35.1) | 10110 | [signalk-server #3270](https://github.com/SignalK/signalk-server/issues/3270) |
| Yacht Devices YDWG-02 | Server #1 on by default: TCP, NMEA 0183 protocol. Servers can be TCP or UDP; TCP limited to 9 clients; the manual recommends UDP. Default gateway address 192.168.4.1. | 1456 | [YDWG-02 manual](https://www.yachtd.com/downloads/ydwg02.pdf), section "Server #1" (pdftotext lines 419-425) |
| Actisense W2K-1 | TCP and UDP data server (canboat drives it at `tcp://...:60002`). Its NMEA 0183 port is **(unverified)**. | 60002 | [canboat doc/gateways.md](https://github.com/canboat/canboat/blob/master/doc/gateways.md) |
| ShipModul MiniPlex-3 | TCP or UDP | 10110 | same |
| go-gps-simulator | TCP **server** (`-tcp`) or UDP sender to broadcast or one host (`-udp`) | 10110 | §7 |
| Typical client GUI (spec in the simulator repo) | UDP (default) or TCP **client**, default `192.168.4.1:10110` | 10110 | `go-gps-simulator/docs/superpowers/specs/2026-07-03-marine-nmea-and-network-output-design.md:21-22` |

Consequences:

- The plotter needs two roles: **TCP client** (connect to a server) and **UDP listener** (bind a local port, receive broadcast or unicast). Both are normal. A TCP server role in the plotter is not needed for any device above.
- UDP has no connection. "Lost" can only mean "no datagram for N seconds".
- TCP needs reconnect with a delay, as the Signal K task does (`src/signalk.rs:326-365`). A gateway on Wi-Fi drops often. A read timeout is also needed: a half-open TCP socket can stay silent for ever **(design consequence)**.
- Do not connect to `127.0.0.1:10110` on a Pi that runs Signal K and also feed that data back to Signal K: a user reported 100 % CPU from such a loop (OpenPlotter page and [Signal K group thread](https://groups.google.com/g/signalk/c/kRzRBa1vbQs)). The plotter only reads, so this applies only if it ever writes.
- Serial (USB adapter at 4800 or 38400 baud) is a third transport. gpsd: "The standard specifies 4800 as the speed, but this is no longer common." The request does not include it; it fits the same line reader.

## 5. Rust crates

Data from the crates.io API and GitHub API on 2026-11-25, and from the downloaded crate sources.

| Crate | Version, date | Licence | Scope | Normal dependencies | Checksum | `!` / AIS | Tag block |
|---|---|---|---|---|---|---|---|
| [`nmea`](https://docs.rs/nmea) (AeroRust) | 0.8.0, 2026-08-08 (0.7.0 2024-10-12); repo pushed 2026-11-03; MSRV 1.87 | Apache-2.0 | ~30 `$` sentences incl. GGA, GLL, GNS, RMC, VTG, HDT, MWV, DPT, TTM; `no_std` without alloc; sentences behind features | arrayvec, cfg-if, chrono, heapless 0.9, nom 8, num-traits | yes (`parse.rs:48-65, 207-209`) | no: requires `$` (`parse.rs:80`) | no |
| [`nmea-parser`](https://docs.rs/nmea-parser) (zaari) | 0.11.0, 2024-06-13; repo pushed 2024-08-14; 15 open issues | Apache-2.0 | AIS VDM/VDO types 1-5, 9-27; GNSS ALM, DBS, DPT, DTM, GGA, GLL, GNS, GSA, GSV, HDT, MTW, MWV, RMC, VTG, ZDA and more | bitvec, chrono, hashbrown 0.14, log, num-traits, serde | yes (`lib.rs:289-305`) | yes; assembles 2-fragment messages (`lib.rs:478-500`); merges type 24 A+B per MMSI | skips all bytes before the first `$`/`!` (`lib.rs:277-287`) |
| [`ais`](https://docs.rs/ais) (squidpickles) | 0.12.0, 2024-10-27; repo pushed 2026-02-17 | Apache-2.0 | AIS only: types 1-5, 8, 11, 15, 17-21, 24 | heapless 0.7, nom 7 | yes (`sentence.rs:174-183`) | yes; `AisFragments::Incomplete` | yes (tests `sentence.rs:295-297, 428-455`) |
| [`nmea0183`](https://crates.io/crates/nmea0183) (nsforth) | 0.6.0, 2025-11-09 | BSD-3-Clause | embedded-first GNSS parser | none | not checked | not checked | not checked |
| `nmea-kit` / `nmea-0183-rs`, `aivdm`, `marlin-ais` | 2026 releases, few hundred to 2.5k downloads | not checked | NMEA 0183 + AIS | not checked | - | - | - |

Notes:

- `nmea` pulls in `nom 8`; `ais` pulls in `nom 7`. Using both builds two versions of nom.
- `nmea-parser` keeps type 24 part data in `saved_vsds: HashMap<u32, VesselStaticData>` keyed by MMSI (`lib.rs:212, 259`). Only `reset()` clears it (`lib.rs:232-235`). Over weeks at sea this grows with every MMSI seen. The caller must call `reset()` from time to time. The fragment map key is type, message ID, count, number and channel, so that map is bounded.
- `nmea-parser` has had no release for 17 months. That is not proof of abandonment; the format is stable.
- The plotter's MSRV is not fixed in `Cargo.toml`. `nmea` 0.8 needs Rust 1.87.

**Hand-written parser for RMC/GGA/GLL/VTG (estimate):**

- Framing, checksum, tag-block skip: about 40 lines. Four sentences with field checks: about 150 lines. Tests from the gpsd examples and the simulator output: about 200 lines.
- No new dependency. All checks (minutes < 60, hemisphere letter, mode `N`, status `V`, finite, range) sit in one place under the project's rules.
- It tolerates the simulator's extra fields (§7.3) by design, because it reads fields by index and ignores extra ones.

**Recommendation:**

1. Write the NMEA 0183 framing and the RMC/GGA/GLL/VTG decoders by hand. The scope is small, the rules are project-specific, and the `nmea` crate would still need our own tag-block and `!` handling.
2. If AIS is in scope, decode it with a crate rather than by hand. `ais` is AIS-only, handles tag blocks and fragments, and has 2 small dependencies; it fits behind our own framing. `nmea-parser` is the alternative if one crate should do both; then plan for `reset()`.
3. Feed every crate a line that our reader already bounded and checked, so a crate bug cannot see unbounded input.

## 6. NMEA 2000 later

### 6.1 PGNs that map to the three channels

From `canboat.json` version 10.0.0 ([canboat.json](https://raw.githubusercontent.com/canboat/canboat/master/docs/canboat.json)):

| PGN | Name | Frame | Interval | Fields used |
|---|---|---|---|---|
| 129025 | Position, Rapid Update | single (8 bytes) | 100 ms | lat, lon: 32-bit signed, 1e-7 deg |
| 129026 | COG & SOG, Rapid Update | single | 250 ms | COG 16-bit, 0.0001 rad; SOG 16-bit, 0.01 m/s; COG reference (true/magnetic) |
| 129029 | GNSS Position Data | fast packet | 1000 ms | lat, lon: 64-bit signed, 1e-16 deg; method (fix type); integrity |
| 129038 | AIS Class A Position Report | fast packet | - | MMSI, lon, lat (1e-7 deg), COG (0.0001 rad), SOG (0.01 m/s) |
| 129039 | AIS Class B Position Report | fast packet | - | same fields |
| 129794 | AIS Class A Static and Voyage | fast packet | - | MMSI, name (160 bits) |
| 129809 | AIS Class B Static, msg 24 part A | fast packet | - | MMSI, name |
| 127250 | Vessel Heading | single | 100 ms | heading 0.0001 rad (not used today) |

- COG in radians and SOG in m/s match `BoatState` with no conversion.
- "Fast packet" PGNs span several CAN frames and need reassembly. 129025/129026 are one frame each.

### 6.2 Physical paths

| Path | What arrives | Source |
|---|---|---|
| SocketCAN on Linux (CAN HAT on the Pi, or a USB-CAN adapter) | raw CAN frames, `can0` | canboat gateways.md; [`socketcan`](https://crates.io/crates/socketcan) crate |
| Actisense NGT-1 (USB serial, 115200) | Actisense binary protocol | canboat gateways.md |
| Yacht Devices RAW over TCP/UDP | text, one CAN frame per line: `hh:mm:ss.ddd D msgid b0..b7<CR><LF>`, `D` = `R` received / `T` transmitted, `msgid` = 29-bit CAN ID in hex. A fast packet spans several lines (example: two `19F51323` lines). | [YDWG-02 manual, Appendix E](https://www.yachtd.com/downloads/ydwg02.pdf) p. 62 |
| Actisense W2K-1 | N2K ASCII (`A173321.107 23FF7 1F513 012F...`) or RAW ASCII as above, TCP 60002 or UDP | canboat [doc/file-formats.md](https://github.com/canboat/canboat/blob/master/doc/file-formats.md), gateways.md |
| Any gateway in NMEA 0183 mode | ordinary NMEA 0183 sentences (YDWG-02 Server #1 default) | YDWG-02 manual |

The last row matters: many NMEA 2000 boats can feed the plotter through the NMEA 0183 input, with no N2K code.

### 6.3 Crates

| Crate | Version, date | Licence | Notes |
|---|---|---|---|
| [`canboat`](https://crates.io/crates/canboat) | 10.0.0, 2026-11-17 | Apache-2.0 | Official, in the canboat repository (`crates/canboat`). Feature `decode`: schema, decode, encode, NGT-1/iKonvert/Maretron codecs, "no threads, no sockets, no I/O". Feature `io`: gateway readers incl. Yacht Devices and SocketCAN. 10.0.0 is 8 days old; the project rule asks for a version at least 2 weeks old. ([crate README](https://github.com/canboat/canboat/blob/master/crates/canboat/README.md)) |
| [`socketcan`](https://crates.io/crates/socketcan) | 4.0.0, 2026-09-05 | MIT | Linux only; libc, nix, socket2; optional tokio |
| [`nmea2000`](https://crates.io/crates/nmea2000) | 0.2.2 stable; 0.3.0-beta.0, 2025-06-13 | MIT | bytes, embedded-can, generic-array |
| [`korri-n2k`](https://crates.io/crates/korri-n2k) | 0.7.1, 2026-09-23 | MIT OR Apache-2.0 | `no_std`-first stack, generated from canboat.json |
| `nmea2000-rs` (herostrat) | GitHub, canboat v6.1.3 | - | listed in the canboat README |

### 6.4 What a common source abstraction must allow

1. **One task per source, same three outputs.** Each source owns its loop and sends on `watch<BoatState>`, `mpsc<(lon, lat)>` and `watch<Vec<AisTarget>>`. `app.rs` does not change except the spawn.
2. **Transport separate from decoder.** NMEA 0183 over TCP, UDP or serial uses one line decoder. YD RAW is also line text over TCP/UDP. SocketCAN delivers frames, not lines. So the decoder input is "a line" or "a CAN frame", and the transport produces one of them.
3. **One shared publisher.** Coordinate validation, "send only on change", track points from one primary source, and AIS expiry (`retain_fresh`, once a second) are the same for every source. Today they live inside `signalk_task` (`src/signalk.rs:394-414`).
4. **Fix loss by time.** NMEA 0183 says "no fix" explicitly (RMC `V`, GGA quality 0). It also goes silent when a cable or Wi-Fi drops. A source must set `position_valid = false` after a silence timeout. Signal K does not do this today (§0).
5. **Never end.** A source task loops for ever with reconnect. A bad address logs an ERROR and keeps retrying. Otherwise `app.rs:329` quits the app (§0).
6. **Testable without a network.** The decoder takes `&str` lines (or frames) and an injected `Instant`, like `apply_delta`. The transport is thin.

## 7. The simulator (`F:\projects\go-gps-simulator`)

### 7.1 Output

| Item | Value | Source |
|---|---|---|
| GPS sentences with a fix | `$GPGGA`, `$GPRMC`, `$GPGLL`, `$GPVTG`, `$GPGSA`, `$GPGSV` (several), `$GPZDA`, in that order | `gps/simulator.go:463-488` |
| GPS sentences before lock | `$GPGGA` (quality 0), `$GPRMC` (status `V`, mode `N`), `$GPGLL` (status `V`), `$GPVTG` (mode `N`), all with empty position fields | `gps/simulator.go:490-494`, `gps/nmea.go:68-118, 219-221, 262` |
| Talker | `GP` for all GPS sentences | `gps/nmea.go:55, 102, 143, 165, 208, 248, 280` |
| Instrument sentences (every cycle, with or without lock) | `$SDDPT`, `$SDDBT`, `$VWVHW`, `$VWVLW`, `$WIMWV`, `$YXMTW`, `$HCHDG` (magnetic heading only, other fields empty), `$IIXDR` | `gps/instruments_nmea.go:10-62`, README "Marine Instrument Sentences" |
| HDT | not sent | `grep HDT` in `gps/` finds nothing |
| AIS | not sent | `grep AIVDM\|AIS` in the Go source finds nothing |
| RMC fields with a fix | 12 fields: time, `A`, lat `%02d%07.4f`, N/S, lon `%03d%07.4f`, E/W, SOG `%.1f` knots, COG `%.1f`, date, empty variation, empty E/W, mode `A` | `gps/nmea.go:76-109` |
| VTG fields | COG true, `T`, empty magnetic, `M`, SOG knots, `N`, km/h, `K`, mode `A` | `gps/nmea.go:188-215` |
| Checksum, line end | XOR after `$`, `*%02X\r\n` | `gps/nmea.go:11-22` |

### 7.2 Transport and options

| Item | Value | Source |
|---|---|---|
| Default output | stdout (no network) | README "Network Output", `cmd/gps-simulator/main.go:160-170` |
| `-tcp` | TCP **server** on `:port` (all interfaces), many clients, each write fanned out; a client that blocks a write for 2 s is dropped | `cmd/gps-simulator/network.go` (`newTCPServer`, `Write`) |
| `-udp` | UDP sender. Default target `255.255.255.255:port` with `SO_BROADCAST`; `-host` gives a unicast target. Each sentence is one datagram, sent from an ephemeral port. | `cmd/gps-simulator/network.go:21-50`, `broadcast_windows.go` |
| `-port` | default 10110 | `cmd/gps-simulator/main.go:52` |
| `-tcp` with `-udp` | refused (mutually exclusive) | `cmd/gps-simulator/main.go:117` |
| `-rate` | default 1 s; all sentences of one cycle go out together | `main.go:38`, `gps/simulator.go:139` |
| `-lock-time` | default 2 s of no-fix output first | `main.go:37` |
| Position, motion | `-lat` 51.4947, `-lon` 0.546063, `-speed` 5 kn, `-course` 90°, `-radius`, `-jitter` | `main.go:28-35`, `--help` output |
| Replay | `-replay file.gpx`, `-replay-speed`, `-replay-loop` | `main.go:44-46` |
| Serial | `-serial`, `-baud` 9600 | `main.go:39-40` |

`gps-simulator.exe --help` (run on 2026-11-25) shows the same flags and defaults.

### 7.3 Quirks the plotter parser must accept

- The no-fix RMC has 13 fields, not 12: one extra empty field before the date (`$GPRMC,%s,V,,,,,,,,%s,,,N`, `gps/nmea.go:117`). Check status first; do not reject on field count.
- The no-fix GGA has 16 fields, not 14 (`$GPGGA,%s,,,,,0,00,,,,,,,,,`, `gps/nmea.go:71`). Quality is still field 6.
- Minutes can print as `60.0000` (§2.2).

### 7.4 What the plotter must speak to it

- NMEA 0183 with talker `GP`, checksum, CRLF.
- RMC (or GGA + VTG) for position, SOG and COG. Status `V` and mode `N` for "no fix".
- Either a **TCP client** to `<sim host>:10110` (`gps-simulator -tcp`), or a **UDP listener** on port 10110 (`gps-simulator -udp`). On one Windows PC both work on loopback; for UDP use `-host 127.0.0.1` if broadcast reception is blocked **(unverified on this PC)**.
- No AIS test data from it.

## Facts that shape the design

1. RMC alone fills `BoatState`; GGA, GLL and VTG are fallbacks. The simulator sends all four each second, so the track needs one primary source.
2. Validity is explicit: RMC/GLL status `A`, mode not `N`, GGA quality > 0. Silence must also clear the fix after a timeout.
3. Match on the 3-letter type, ignore the talker (`GP`, `GN`, `II`, ...).
4. Bound every line (about 100 bytes), check the XOR checksum, skip a `\...\` tag block, then validate every field and coordinate.
5. Units: knots × 1852/3600 to m/s, degrees × π/180 to radians, `ddmm.mmmm` / 60.
6. Port 10110 TCP and UDP is the registered default. The plotter needs a TCP client and a UDP listener, not a server.
7. TCP needs reconnect with a delay and a read timeout; UDP needs only a silence timeout.
8. The source task must never return: `app.rs:329` `has_changed()?` quits the app when the boat channel closes.
9. Shared rules (validation, change detection, track source, AIS expiry) belong in one publisher, not in each source.
10. NMEA 2000 fits the same shape: transport (TCP/UDP text, SocketCAN frames) separate from decoder (PGN 129025/129026/129029, AIS 129038/129039/129794/129809), same three channels.
11. AIS over 0183 is a separate, larger decoder. The simulator cannot test it.
12. A hand-written parser for the four position sentences needs no new dependency. For AIS, the `ais` crate is the smallest fit.

## Open questions for the user

1. Is AIS over NMEA 0183 in the first version, or later? (The simulator sends none.)
2. Which transports in the first version: TCP client and UDP listener only, or also serial?
3. Config shape. One example, not a decision:
   ```toml
   [input]
   source = "nmea0183"        # "signalk" | "nmea0183" | "nmea2k"
   [nmea0183]
   transport = "udp"          # "tcp" (client) | "udp" (listener)
   address = "0.0.0.0:10110"  # tcp: server to connect to; udp: local address to bind
   ```
   Is `signalk` the default when the key is missing (so the existing `plotter.toml` keeps working)?
4. Add an optional talker filter (for example `talker = "GN"`) for multiplexers with two GPS receivers?
5. Silence timeout before the boat marker goes to "no fix": how many seconds? Apply it to Signal K too?
6. Accept RMC status `V` with mode `E` (dead reckoning) as a position, shown differently, or always treat it as no fix?
7. Use heading (HDT, or HDG with variation) for course-up or the boat symbol, or keep COG only as now?
8. Must the NMEA 0183 input also accept the Signal K server's 10110 stream as a fallback when the WebSocket fails, or is the source strictly one at a time?
9. For NMEA 2000 later: SocketCAN on the Pi, or a gateway over IP (YD RAW, W2K-1)? This decides whether `socketcan` or only a text decoder is needed.

## Sources

- gpsd, "NMEA Revealed": https://gpsd.gitlab.io/gpsd/NMEA.html
- gpsd, "AIVDM/AIVDO protocol decoding" v1.58: https://gpsd.gitlab.io/gpsd/AIVDM.html
- IANA service names and port numbers: https://www.iana.org/assignments/service-names-port-numbers/service-names-port-numbers.txt
- OpenPlotter, NMEA 0183 multiplexing (Signal K server): https://openplotter.readthedocs.io/latest/signalk/multiplexing.html
- signalk-server issue #3270: https://github.com/SignalK/signalk-server/issues/3270
- Yacht Devices YDWG-02 user manual: https://www.yachtd.com/downloads/ydwg02.pdf
- canboat README, gateways, file formats, crate README, canboat.json: https://github.com/canboat/canboat
- crates.io API: https://crates.io/api/v1/crates/{nmea,nmea-parser,ais,nmea0183,nmea2000,socketcan,canboat,korri-n2k}
- Crate sources read: `nmea-0.8.0`, `nmea-parser-0.11.0`, `ais-0.12.0` from https://static.crates.io/crates/
- tokio `watch::Receiver::has_changed`: https://docs.rs/tokio/latest/tokio/sync/watch/struct.Receiver.html#method.has_changed
- BIPM SI Brochure (nautical mile, knot): https://www.bipm.org/en/publications/si-brochure
- User's notes: `docs/NMEA 0183-NMEA 2000-Signal K-SEC.md`
- Simulator: `F:\projects\go-gps-simulator` (README.md, `cmd/gps-simulator/*.go`, `gps/*.go`, `docs/superpowers/specs/`)
