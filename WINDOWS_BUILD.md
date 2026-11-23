# Building SDL2 Plotter on Windows

Date: 2026-11-23
Target machine: this Windows 11 PC (x86_64), Git Bash as the shell.

This guide has two halves. Part 1 installs the smallest Rust toolchain that can compile this app. Part 2 lists the changes that make the repo build on Windows and Linux from the same source, in the order to do them.

## What is already on this machine

Checked on 2026-11-23:

| Component | Status |
|---|---|
| Visual Studio Community 2026 and Build Tools 2026 | Installed, with MSVC 14.51 C++ toolset |
| Windows 11 SDK 10.0.26100 | Installed |
| Git for Windows (Git Bash) | Installed |
| winget, scoop, choco | Installed |
| rustup / cargo | Not installed |
| MSYS2 (pacman) | Not installed. The `/ucrt64` folder belongs to Git for Windows |
| cmake | Not installed (vcpkg fetches its own copy) |
| gcc | Only TDM gcc 5.1 at `F:\TOOLS\GCC`. Do not use it for this |

Because the MSVC toolset and SDK are already present, the MSVC Rust target needs nothing else from Visual Studio. The `cc` crate (used by `rusqlite` to compile SQLite) and the Rust linker both locate `cl.exe` and `link.exe` through vswhere, so a plain Git Bash or PowerShell window works. No Developer Command Prompt is required.

## Part 1: Minimal Rust install

### 1.1 Install rustup with the minimal profile

In Git Bash:

```bash
curl -sSfL https://win.rustup.rs/x86_64 -o /tmp/rustup-init.exe
/tmp/rustup-init.exe --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc -y
```

The minimal profile installs `rustc`, `cargo` and the standard library only, about 500 MB. It skips documentation and the extra tools of the default profile.

Alternative with winget (uses the default profile, so slightly larger):

```powershell
winget install Rustlang.Rustup
```

### 1.2 Put cargo on the PATH

The installer adds `%USERPROFILE%\.cargo\bin` to the user PATH, but open terminals do not see it. Either open a new terminal, or in the current Git Bash:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Check:

```bash
rustc --version
cargo --version
rustup show
```

`rustup show` should list `stable-x86_64-pc-windows-msvc` as the default.

### 1.3 Add rustfmt (recommended, small)

The code review recommends running rustfmt. It is a few MB:

```bash
rustup component add rustfmt
```

Add `clippy` the same way if wanted. Neither is needed to compile.

### 1.4 Install the SDL2 libraries through vcpkg

The app needs SDL2, SDL2_ttf and SDL2_gfx. SDL2 and SDL2_ttf publish prebuilt MSVC development zips, but SDL2_gfx does not, so the cleanest route is vcpkg driven by `cargo-vcpkg`. It builds static libraries once into `target/vcpkg`, and the `sdl2` crate finds them automatically. No DLLs to copy and nothing on the system PATH.

Install the helper:

```bash
cargo install cargo-vcpkg
```

`Cargo.toml` already carries the vcpkg metadata (Part 2, step 1 is done). Run from the repo root:

```bash
cd /f/projects/SDL2_plotter
cargo vcpkg build
```

This clones vcpkg into `target/vcpkg`, downloads its own cmake and ninja, and compiles SDL2, SDL2_ttf (with freetype and its dependencies) and SDL2_gfx. Expect roughly 10 to 20 minutes and 2 to 3 GB of disk the first time. It only needs re-running when the vcpkg revision or the dependency list changes.

**Known failure on this PC.** The first run stopped after the clone with:

```
'bootstrap-vcpkg.bat' is not recognized as an internal or external command
cargo-vcpkg: failed to run vcpkg bootstrap
```

cmd.exe here does not find a batch file by bare name even when it sits in the current directory, so the bootstrap has to be run once with a full path. In Git Bash:

```bash
cd /f/projects/SDL2_plotter/target/vcpkg
cmd //c "$(cygpath -w "$PWD")\\bootstrap-vcpkg.bat -disableMetrics"
cd /f/projects/SDL2_plotter
cargo vcpkg build
```

Once `target/vcpkg/vcpkg.exe` exists, `cargo vcpkg build` skips the bootstrap and carries on.

### 1.5 Build and test

```bash
cargo build
cargo test
```

Verified on this PC on 2026-11-23 with Rust 1.100.0 and vcpkg 2026.10.16: `cargo vcpkg build` took about 5 minutes, a clean `cargo build` about 70 seconds, and all 38 unit tests passed.

`cargo test` runs the unit tests in `config`, `geo_data`, `track`, `signalk`, `mbtiles_renderer`, `waypoints`, `state` and `app`. None of them need SDL at runtime, so they run headless.

### 1.6 Run

Two things in the committed configuration stop the app from starting on this PC. Fix both before the first run:

1. **Streaming bind address.** `default/plotter.toml` has `bind_address = "192.168.3.10"`, the boat computer's address. `TcpListener::bind` fails on any machine without that address, and `main.rs` treats that as fatal. Set `enabled = false`, or use `bind_address = "127.0.0.1"` to view the MJPEG stream locally at `http://127.0.0.1:8090/`.
2. **A chart file.** `*.mbtiles` and `map/` are gitignored, so a fresh checkout has no `assets/OSM.mbtiles`. `MBTilesRenderer::new` opens it read-only and fails if it is missing. Copy an MBTiles raster chart from the boat computer into `assets/` or `map/` and point `charts.default` at it.

Signal K being unreachable is fine. The background task retries forever and the UI runs without a boat position.

Then:

```bash
cargo run
```

Run from the repo root. All data paths (`default/`, `export/`, `plotter.log`, `assets/`) are relative to the working directory. Double-clicking `target\debug\plotter.exe` starts in the wrong folder and fails to create `default/plotter.toml`. Part 2 step 4 covers the fix.

If `bind_address` is `0.0.0.0`, Windows Firewall asks once whether to allow the app to listen.

## Part 2: Porting work for a shared Windows and Linux build

Ordered so each step leaves the Linux build unchanged.

### Step 1: Cargo.toml (done 2026-11-23)

Rust 2021 edition uses feature resolver 2, so features set in a `[target.'cfg(windows)']` table do not leak into Linux builds. The Windows-only `sdl2` features and the vcpkg metadata are in place, and the unused direct `tungstenite` dependency is gone (all access goes through `tokio_tungstenite`, and keeping it compiled two versions of the crate). For reference, the relevant parts:

```toml
[package]
name = "plotter"
version = "0.1.0"
edition = "2021"

[dependencies]
rusqlite = { version = "0.29", features = ["bundled"] }
image = { version = "0.24", default-features = false, features = ["png", "jpeg"] }
lru = "0.10"
anyhow = "1.0"
sdl2 = { version = "0.38.0", features = ["ttf", "gfx", "unsafe_textures"] }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
futures-util = "0.3"
url = "2.5"
serde = { version = "1", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"
chrono = "0.4.45"

# Windows: link SDL2, SDL2_ttf and SDL2_gfx statically from vcpkg.
# Linux keeps using the system libraries from apt.
[target.'cfg(windows)'.dependencies]
sdl2 = { version = "0.38.0", features = ["use-vcpkg", "static-link"] }

[package.metadata.vcpkg]
dependencies = ["sdl2", "sdl2-ttf", "sdl2-gfx"]
git = "https://github.com/microsoft/vcpkg"
# A release tag from https://github.com/microsoft/vcpkg/releases keeps builds reproducible.
rev = "2026.10.16"

[package.metadata.vcpkg.target]
x86_64-pc-windows-msvc = { triplet = "x64-windows-static-md" }
```

`x64-windows-static-md` means static SDL libraries linked against the dynamic C runtime, which matches what the Rust MSVC target uses. `native-tls` uses Schannel on Windows, so no OpenSSL is needed.

Run `cargo vcpkg build` once, then `cargo build`.

### Step 2: Line endings (done 2026-11-23)

Windows Git defaults to CRLF checkouts, which will show every file as modified on the Pi. `.gitattributes` now contains:

```
* text=auto eol=lf
*.ttf binary
*.mbtiles binary
*.dia binary
```

### Step 3: Hide the console window in release builds (optional)

A Rust binary is a console program by default, so the chart window opens alongside a black console. Add at the top of `src/main.rs`:

```rust
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
```

Debug builds keep the console so `println!` output is visible. Release builds rely on `plotter.log`, which the app already writes. Note that the remaining `println!` calls go nowhere in a release build on Windows, another reason to route them through `diagnostics::record` as the review suggests.

### Step 4: Resolve data paths against a base directory

Every data path is relative to the working directory: `default/plotter.toml`, `default/state.json`, `default/track.json`, `export/`, `plotter.log`, and the font and chart paths in the config. On Linux the app is launched from its folder, so this works. On Windows a shortcut or double-click sets the working directory to the exe's folder.

Smallest fix: at the top of `main`, if `default/plotter.toml` is not found in the current directory, walk up from `std::env::current_exe()` looking for a folder containing `default/plotter.toml` and `std::env::set_current_dir` to it. That handles `target/debug/plotter.exe` and `target/release/plotter.exe` without touching any other module. Also let `fs::write` in `AppConfig::load_or_create` create the `default/` directory first with `create_dir_all`, which it currently does not.

A cleaner later step is a `PLOTTER_HOME` environment variable or `--home` argument stored once in a `Paths` struct.

### Step 5: Do not die when streaming cannot bind

`main.rs` propagates a `Streamer::start` error and exits. On a laptop whose address changes, or when the port is taken, the navigation display should still start. Log the error with `diagnostics::record` and continue with `streamer = None`. This helps on the boat too.

### Step 6: Windows runtime notes

- **File replace.** `std::fs::rename` over an existing file works on Windows (it uses `MOVEFILE_REPLACE_EXISTING`), so the atomic save pattern in `geo_data.rs` is fine. It can fail transiently if an antivirus scanner or editor has the target open. The code already logs the error and retries on the next checkpoint, which is the right behaviour.
- **High DPI.** On a display scaled above 100 percent, a 1600 by 800 window may be scaled by Windows and look blurry. If it matters, set the SDL hint `SDL_WINDOWS_DPI_AWARENESS` to `permonitorv2` before `sdl2::init()`, or accept the default. Not needed for a development build.
- **Fullscreen.** `fullscreen_desktop` and `current_display_mode(0)` behave the same on Windows. No change.
- **Temp files in tests.** The tests use `std::env::temp_dir()` and process-id based names, which works unchanged on Windows.

### Step 7: Update README

Add a Windows section pointing at this file, and move the apt package list out of the comment block at the top of `src/main.rs` into the README where the Linux instructions already live.

### Step 8 (optional): Drop SDL2_gfx

SDL2_gfx is the one dependency without official Windows binaries and the reason vcpkg is needed. The app uses five calls from it: `thick_line`, `filled_circle`, `circle`, `filled_trigon` and the `DrawRenderer` trait in `screen.rs`, `waypoints.rs` and `ais.rs`. `screen.rs` already contains hand-rolled `draw_filled_circle` and `draw_circle`. Adding a thick line (a filled quad from four offset points) and a filled triangle (scanline fill) in the same style is around 80 lines. After that:

- Linux needs one less apt package (`libsdl2-gfx-dev`).
- Windows can use the prebuilt `SDL2-devel-VC` and `SDL2_ttf-devel-VC` zips from the SDL GitHub releases instead of vcpkg, or keep vcpkg with a shorter dependency list.
- The `as i16` truncation issue from the code review disappears, because the replacements take `i32`.

Worth doing after the Windows build works with vcpkg, not before.

## Building for the Raspberry Pi from this PC

Cross-compiling to `aarch64-unknown-linux-gnu` with SDL2 is possible with the `cross` tool and a Docker image carrying the SDL dev packages, but it is more setup than it saves. Building on the Pi itself with the apt packages in the README remains the simplest path. The Windows build is for development and testing of the UI and data logic.

## Quick reference

```bash
# one-time
curl -sSfL https://win.rustup.rs/x86_64 -o /tmp/rustup-init.exe
/tmp/rustup-init.exe --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc -y
export PATH="$HOME/.cargo/bin:$PATH"
rustup component add rustfmt
cargo install cargo-vcpkg

# once per machine (Cargo.toml already has the vcpkg metadata)
cd /f/projects/SDL2_plotter
cargo vcpkg build

# every day
cargo build
cargo test
cargo run
```
