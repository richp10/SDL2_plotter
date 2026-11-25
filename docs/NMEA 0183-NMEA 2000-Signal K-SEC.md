# NMEA 0183, NMEA 2000 & Signal K — Specification Sources

Nov 25, 2026 · @Richard Phillips

The official specs for NMEA 0183 and NMEA 2000 are closed and expensive. Signal K is fully open. For all three, the community references below are what most open-source implementers actually work from.

## NMEA 0183

**Official**

- The NMEA sells the standard. Non-members paid at least US$2000 as of September 2020, so check [nmea.org](https://www.nmea.org) for today's price.
- The current version appears to be **4.30** (December 2023), replacing 4.11 (2018).
- The international equivalent is **IEC 61162-1**, which DIN sells from about €366 (that price is for the 2011 edition). It's cheaper than buying from NMEA, though not word-for-word identical.

**Best free references**

- **gpsd's "NMEA Revealed"** by Eric S. Raymond: [gpsd.gitlab.io/gpsd/NMEA.html](https://gpsd.gitlab.io/gpsd/NMEA.html). The standard community reference: sentences with field descriptions, talker IDs, validity flags and version differences (for example, the extra signal-integrity field added in NMEA 2.3 to APB, BWC, BWR, GLL, RMA, RMB, RMC, VTG, WCV and XTE). It leans towards GPS sentences, but the marine instrument sentences are included too.
- **adrianmo/go-nmea**: [github.com/adrianmo/go-nmea](https://github.com/adrianmo/go-nmea). Worth using as a cross-check because its sentence list is based on the IEC 61162-1:2016 table of contents, so it shows you the full set of official sentences. It also supports NMEA 4.10 TAG blocks.

## NMEA 2000

**Official**

- The PGN database (Appendix B) is paid and NDA-bound. Anyone who buys it can't share its contents, so open-source developers can't get it that way.

**Best free reference: CANboat**

- PGN documentation (HTML): [canboat.github.io/canboat/canboat.html](https://canboat.github.io/canboat/canboat.html)
- Machine-readable `canboat.json`: [raw.githubusercontent.com/canboat/canboat/master/docs/canboat.json](https://raw.githubusercontent.com/canboat/canboat/master/docs/canboat.json). This is the file every open library generates its code from.
- Repo: [github.com/canboat/canboat](https://github.com/canboat/canboat)

**Libraries built on CANboat**

- **canboatjs** (JavaScript/TypeScript, used by Signal K Server)
- **nmea2000** (Python, on PyPI)
- **boatkit-io/n2k** (Go): generates its Go code from the current canboat.json
- **open-ships/n2k** (Go): decodes about 600 PGNs into Go structs, with SI-unit accessors

**For the ESP32 multiplexer**

- **Timo Lappalainen's NMEA2000 C++ library**: [github.com/ttlappalainen/NMEA2000](https://github.com/ttlappalainen/NMEA2000), with docs at [ttlappalainen.github.io/NMEA2000](https://ttlappalainen.github.io/NMEA2000). It handles the mandatory NMEA 2000 requirements automatically and has been used in several commercially certified devices. `N2kMessages.h` doubles as readable documentation of each PGN's fields.
- One catch: the original **NMEA2000\_esp32** driver writes to the CAN registers directly. That doesn't work on the newer ESP32-C3 (and people have had the same trouble on the S2), so for those chips use a TWAI-based fork such as `mosvov/NMEA2000_esp32_c3`.

## Signal K

Everything is open, but the documentation is split in two.

- **The v1 specification**: [signalk.org/specification](https://signalk.org/specification/1.8.2/doc/). Covers the data model, full and delta formats, the REST and WebSocket APIs, and the keys reference generated from the JSON Schema. The newest version I found is 1.8.2, but I haven't confirmed that's the latest.
- **The Signal K Server docs**: [demo.signalk.org/documentation](https://demo.signalk.org/documentation/). v2 is where development is now. Server v2 moves away from v1's fully defined data-model schema towards modular REST APIs defined as OpenAPI, served from `/signalk/v2/api`.
  - Gotcha: deltas from the v2 APIs never get into the full model, so those values can only be read with GET requests.
- **NMEA → Signal K mappings**: the `n2k-signalk` and `nmea0183-signalk` repos in the SignalK GitHub org show how NMEA data turns into Signal K paths. These were noted from general knowledge rather than verified in this search, so check they're still current.

## Recommendations

- Use CANboat as the single source of truth for N2K, and treat any PGN marked as unverified in it with caution.
- For 0183, gpsd plus go-nmea's IEC sentence list covers nearly everything without paying for the spec.
- The exceptions are the newer 4.x GNSS changes and the exact AIS/DSC field definitions. If you need certainty on those, buying IEC 61162-1 is the cheaper legitimate route.

## Sources

- [NMEA 0183 — Wikipedia](https://en.wikipedia.org/wiki/NMEA_0183)
- [NMEA 0183 — HandWiki (version history)](https://handwiki.org/wiki/NMEA_0183)
- [IEC 61162-1 at DIN](https://www.din.de/en/getting-involved/standards-committees/dke/publications/wdc-beuth:din21:143482391)
- [CANboat](https://github.com/canboat/canboat)
- [boatkit-io/n2k](https://github.com/boatkit-io/n2k)
- [open-ships/n2k](https://pkg.go.dev/github.com/open-ships/n2k@v0.2.2)
- [NMEA2000\_esp32](https://github.com/ttlappalainen/NMEA2000_esp32)
- [Signal K Server — Developing](https://demo.signalk.org/documentation/Developing.html)
