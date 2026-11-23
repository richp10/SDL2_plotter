# SDL2 Plotter

SDL2 Plotter is a marine navigation application written in Rust. It displays MBTiles charts, vessel data and AIS targets received from a Signal K server, together with routes, tracks and waypoints.
The principles that led me to write this program are:

1/ Optimize code execution using a compiled language (Rust)
2/ To be able to install the code on a desktop-less Linux distribution (such as Dietpi) using the SDL library
3/ To optimize the user interface as much as possible
4/ Enable remote viewing on a tablet via a web page for the cockpit

SDL2 Plotter can run in a desktop Linux environment, but the idea is to create a dedicated, minimalist Linux distribution with rock-solid navigation that's easy to use for everyone at the chart table.


## Features

- Local MBTiles charts
- North Up and Course Up display modes
- Automatic vessel following
- Persistent JSON routes and tracks, with explicit GPX export
- Routes and waypoints
- Distance and bearing measurements
- AIS target display
- Animated zoom
- MJPEG chart streaming over the local network
- Startup configuration through `plotter.toml`

## Requirements

The application builds on Linux (the deployment target, including Raspberry Pi) and on Windows (for development). The Rust code is identical on both; only the way the SDL2 libraries are supplied differs.

### Linux

On Debian, Raspberry Pi OS or Ubuntu, install Rust and the SDL2 libraries:

```bash
sudo apt update
sudo apt install \
  libsdl2-dev \
  libsdl2-ttf-dev \
  libsdl2-image-dev \
  libsdl2-gfx-dev \
  libgbm-dev \
  libegl1-mesa-dev
```

The Signal K server must be reachable from the computer or Raspberry Pi running the application.

### Windows

The step-by-step guide, including what was verified on a real machine, is in [WINDOWS_BUILD.md](WINDOWS_BUILD.md). In short:

1. Install the Visual Studio C++ build tools and Windows SDK (Visual Studio Installer, "Desktop development with C++").
2. Install Rust with the MSVC toolchain: `rustup-init.exe --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc`.
3. Install the SDL2 libraries once with vcpkg: `cargo install cargo-vcpkg` then `cargo vcpkg build` from the repository root.
4. `cargo build`, `cargo test`, `cargo run` as on Linux.

#### How the Windows build works

`Cargo.toml` declares the `sdl2` crate once, with the `ttf`, `gfx` and `unsafe_textures` features, for every platform. A `[target.'cfg(windows)'.dependencies]` table adds two more features on Windows only: `use-vcpkg`, so the build script locates SDL2 in the vcpkg tree under `target/vcpkg`, and `static-link`, so SDL2, SDL2_ttf and SDL2_gfx are linked into `plotter.exe` and no DLLs need to be shipped. A `[package.metadata.vcpkg]` section pins the vcpkg revision and the three ports to build.

Cargo's feature resolver (version 2, the default for edition 2021) ignores target-specific dependencies for platforms that are not being built, so Linux builds resolve `sdl2` with exactly the same three features as before and never see vcpkg. `cargo tree --target x86_64-unknown-linux-gnu -e features -i sdl2-sys` shows this.

#### Alternative approaches on Windows

- **Prebuilt SDL2 development zips.** SDL2 and SDL2_ttf publish `*-devel-*-VC.zip` archives on their GitHub release pages. Point the `LIB` environment variable (or a `.cargo/config.toml` `-L` flag) at the `lib/x64` folders, drop `use-vcpkg` and `static-link`, and copy the DLLs next to the exe. SDL2_gfx has no official Windows binaries, so this only works after replacing the five SDL2_gfx drawing calls with the hand-written primitives already present in `src/screen.rs`. That change would also remove `libsdl2-gfx-dev` from the Linux requirements.
- **GNU toolchain with MSYS2.** Install MSYS2, then `pacman -S mingw-w64-ucrt-x86_64-toolchain mingw-w64-ucrt-x86_64-SDL2 mingw-w64-ucrt-x86_64-SDL2_ttf mingw-w64-ucrt-x86_64-SDL2_gfx`, and use the `stable-x86_64-pc-windows-gnu` Rust toolchain from the UCRT64 shell. This mirrors the apt workflow and needs no Visual Studio, at the cost of a second compiler environment and a GNU-built binary.
- **WSL2.** Follow the Linux instructions inside Ubuntu on WSL2. The SDL window appears through WSLg. Simplest when a Linux-identical build matters more than a native Windows binary.

## Building and running

From the project root:

```bash
cargo run
```

To build an optimized release:

```bash
cargo build --release
./target/release/plotter
```

## Configuration

At first launch, `default/plotter.toml` is created with default values. It can then be edited without recompiling the application.

### Signal K

```toml
[signalk]
url = "ws://192.168.3.2/signalk/v1/stream?subscribe=none"

[signalk.paths]
position = "navigation.position"
course_over_ground = "navigation.courseOverGroundLastMinute"
speed_over_ground = "navigation.speedOverGroundLastMinute"
ais_position = "navigation.position"
ais_name = "name"
ais_course_over_ground = "navigation.courseOverGroundTrue"
ais_speed_over_ground = "navigation.speedOverGround"
```

The subscription uses these configured paths for vessel data and AIS targets.

### Charts

```toml
[charts]
directory = "map"
default = "assets/OSM.mbtiles"
```

Available charts are `.mbtiles` files stored in `directory`. The current chart selection is saved in `default/state.json`.

### Window

```toml
[window]
title = "KALYMNOS"
fullscreen = false
width = 1600
height = 800
```

### Appearance

Symbol sizes are expressed in pixels. Colors use `[red, green, blue, alpha]`, with values between 0 and 255.

```toml
[appearance]
boat_size = 6
boat_circle_radius = 50
boat_color = [0, 0, 255, 255]
mark_size = 6
measure_color = [0, 0, 255, 255]
route_width = 3
route_color = [255, 0, 0, 255]
```

### Rendering

```toml
[rendering]
tile_cache_size = 512
prefetch_margin = 2
```

### Routes and tracks

```toml
[track]
checkpoint_interval_seconds = 15
```

Routes and the current track are stored as versioned JSON in `default/route.json` and `default/track.json`. The track is checkpointed periodically. GPX exports for routes, tracks and waypoints are available from their respective menus and written to `export/`.

On first launch after upgrading, an existing `default/route.gpx` or `default/track.gpx` is migrated to its corresponding JSON file. The original GPX file is left unchanged.

### Network streaming

The MJPEG stream can be viewed in a browser on a local computer or tablet:

```toml
[streaming]
enabled = true
bind_address = "0.0.0.0"
port = 8080
fps = 5
jpeg_quality = 70
```

With `bind_address = "0.0.0.0"`, the server listens on every network interface. From a tablet, open:

```text
http://RPI_ADDRESS:8080/
```

The direct MJPEG stream is available at:

```text
http://RPI_ADDRESS:8080/stream
```

`bind_address` must be the real IP address of the machine running the application, or `0.0.0.0` to listen on all interfaces. A network address such as `192.168.3.0` does not identify a host. Use the machine firewall to restrict access.

If the address cannot be bound (for example the boat's address on a development PC), the application logs an ERROR in `plotter.log` and runs without the stream; the chart is never held back by the tablet feed. On a development machine set `enabled = false` or `bind_address = "127.0.0.1"` to avoid the error.

## Runtime files

Only `default/plotter.toml` is part of the repository. Everything else below is written by the application, belongs to the installation it runs on, and is ignored by git, so a fresh clone starts with an empty route, track and waypoint list and the position saved in the chart's own centre.

- `default/plotter.toml`: configuration editable without recompiling
- `default/state.json`: position, zoom and dynamic preferences
- `default/route.json`: current route
- `default/waypoints.json`: waypoint list
- `default/track.json`: current track
- `default/route.gpx` and `default/track.gpx`: legacy files retained after migration
- `export/`: timestamped GPX route, track and waypoint exports
- `plotter.log`: log and diagnostics. INFO and above are written by default; set the environment variable `PLOTTER_LOG=debug` to include every input event, or `warn` to keep only problems. The file is capped at 5 MB and rotated once to `plotter.log.1`, so the log never takes more than 10 MB of the SD card.

## Architecture

- `src/app.rs`: main loop and orchestration
- `src/app/actions.rs`: application actions
- `src/app/input.rs`: keyboard and mouse input
- `src/app/render.rs`: chart and overlay assembly
- `src/mbtiles_renderer.rs`: projection, tiles, zoom and cache
- `src/signalk.rs`: Signal K connection and decoding
- `src/screen.rs`: SDL2 rendering
- `src/stream.rs`: network MJPEG server
- `src/config.rs`: `plotter.toml` loading and validation

## Tests

```bash
cargo test
```
