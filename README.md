# SDL2 Plotter

SDL2 Plotter is a marine navigation application written in Rust. It displays MBTiles charts, vessel data and AIS targets received from a Signal K server, together with routes, tracks and waypoints.

## Features

- Local MBTiles charts
- North Up and Course Up display modes
- Automatic vessel following
- Persistent GPX tracks
- Routes and waypoints
- Distance and bearing measurements
- AIS target display
- Animated zoom
- MJPEG chart streaming over the local network
- Startup configuration through `plotter.toml`

## Requirements

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

At first launch, `plotter.toml` is created with default values. It can then be edited without recompiling the application.

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

Available charts are `.mbtiles` files stored in `directory`. The current chart selection is saved in `state.json`.

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

### Track

```toml
[track]
checkpoint_interval_seconds = 15
```

The current track is loaded from `track/default.gpx` and saved periodically.

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

## Runtime files

- `plotter.toml`: configuration editable without recompiling
- `state.json`: position, zoom and dynamic preferences
- `route.gpx`: current route
- `waypoints`: waypoint list
- `track/default.gpx`: current track
- `plotter.log`: log and diagnostics

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
