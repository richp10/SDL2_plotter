use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct AppConfig {
    pub input: InputConfig,
    pub signalk: SignalKConfig,
    pub nmea0183: Nmea0183Config,
    pub charts: ChartsConfig,
    pub window: WindowConfig,
    pub fonts: FontConfig,
    pub appearance: AppearanceConfig,
    pub rendering: RenderingConfig,
    pub streaming: StreamingConfig,
    pub track: TrackConfig,
}

/// Where live boat data comes from. One source at a time, chosen at start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputSource {
    #[default]
    Signalk,
    Nmea0183,
    /// Accepted, not supported yet: the app runs with no live data.
    Nmea2k,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct InputConfig {
    pub source: InputSource,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// The plotter connects to `address`.
    #[default]
    Tcp,
    /// The plotter binds `address` and listens.
    Udp,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Nmea0183Config {
    pub transport: Transport,
    pub address: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SignalKConfig {
    pub url: String,
    pub paths: SignalKPaths,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SignalKPaths {
    pub position: String,
    pub course_over_ground: String,
    pub speed_over_ground: String,
    pub heading: String,
    pub ais_position: String,
    pub ais_name: String,
    pub ais_course_over_ground: String,
    pub ais_speed_over_ground: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ChartsConfig {
    pub directory: String,
    pub default: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct WindowConfig {
    pub title: String,
    pub fullscreen: bool,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct FontConfig {
    pub file: String,
    pub window_size: u16,
    pub ui_size: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub boat_size: i16,
    pub boat_circle_radius: i16,
    pub mark_size: i16,
    pub route_point_size: i16,
    pub route_width: u8,
    pub cog_length: f64,
    pub cog_width: u8,
    pub ais_course_length: i16,
    pub ais_course_width: u8,
    pub ais_size: i16,
    pub ais_outline_size: i16,
    pub boat_color: [u8; 4],
    /// The boat marker while there is no fix.
    pub boat_no_fix_color: [u8; 4],
    pub measure_color: [u8; 4],
    pub route_color: [u8; 4],
    pub track_color: [u8; 4],
    pub waypoint_color: [u8; 4],
    /// The chart manager's "Where" outline.
    pub coverage_color: [u8; 4],
    pub cog_color: [u8; 4],
    pub ais_color: [u8; 4],
    pub ais_outline_color: [u8; 4],
    pub map_background_color: [u8; 4],
    pub line_color: [u8; 4],
    pub night_overlay_color: [u8; 4],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct RenderingConfig {
    pub tile_cache_size: usize,
    pub prefetch_margin: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct TrackConfig {
    pub checkpoint_interval_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamingConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub fps: u32,
    pub jpeg_quality: u8,
}

impl AppConfig {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        let config = match fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).with_context(|| {
                format!("Impossible de lire la configuration {}", path.display())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                let contents = toml::to_string_pretty(&config)?;
                fs::write(path, contents).with_context(|| {
                    format!("Impossible de créer la configuration {}", path.display())
                })?;
                config
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Impossible de lire la configuration {}", path.display())
                });
            }
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        match self.input.source {
            InputSource::Signalk => {
                let url = url::Url::parse(&self.signalk.url)
                    .context("signalk.url doit être une URL WebSocket valide")?;
                if !matches!(url.scheme(), "ws" | "wss") {
                    bail!("signalk.url doit utiliser le protocole ws:// ou wss://");
                }
                let paths = [
                    &self.signalk.paths.position,
                    &self.signalk.paths.course_over_ground,
                    &self.signalk.paths.speed_over_ground,
                    &self.signalk.paths.heading,
                    &self.signalk.paths.ais_position,
                    &self.signalk.paths.ais_name,
                    &self.signalk.paths.ais_course_over_ground,
                    &self.signalk.paths.ais_speed_over_ground,
                ];
                if paths.iter().any(|path| path.trim().is_empty()) {
                    bail!("Les chemins Signal K ne peuvent pas être vides");
                }
            }
            InputSource::Nmea0183 => {
                if self
                    .nmea0183
                    .address
                    .parse::<std::net::SocketAddr>()
                    .is_err()
                {
                    bail!(
                        "nmea0183.address doit être une adresse IP:port (par ex. 127.0.0.1:10110)"
                    );
                }
            }
            InputSource::Nmea2k => {}
        }
        if self.charts.directory.is_empty() || self.charts.default.is_empty() {
            bail!("charts.directory et charts.default ne peuvent pas être vides");
        }
        if self.window.width == 0 || self.window.height == 0 {
            bail!("window.width et window.height doivent être supérieurs à zéro");
        }
        if self.fonts.file.is_empty() || self.fonts.window_size == 0 || self.fonts.ui_size == 0 {
            bail!("fonts.file, fonts.window_size et fonts.ui_size doivent être renseignés");
        }
        let appearance_sizes = [
            self.appearance.boat_size,
            self.appearance.boat_circle_radius,
            self.appearance.mark_size,
            self.appearance.route_point_size,
            self.appearance.ais_course_length,
            self.appearance.ais_size,
            self.appearance.ais_outline_size,
        ];
        if appearance_sizes.iter().any(|size| *size <= 0)
            || self.appearance.route_width == 0
            || self.appearance.cog_width == 0
            || self.appearance.ais_course_width == 0
            || !self.appearance.cog_length.is_finite()
            || self.appearance.cog_length <= 0.0
            || self.appearance.cog_length > i16::MAX as f64
        {
            bail!(
                "Les tailles de appearance doivent être positives et cog_length ne pas dépasser {}",
                i16::MAX
            );
        }
        if self.rendering.tile_cache_size == 0
            || !(0..=16).contains(&self.rendering.prefetch_margin)
        {
            bail!("rendering.tile_cache_size doit être positif et prefetch_margin compris entre 0 et 16");
        }
        if self.track.checkpoint_interval_seconds == 0 {
            bail!("track.checkpoint_interval_seconds doit être supérieur à zéro");
        }
        if self.streaming.enabled
            && (self.streaming.port == 0
                || self.streaming.bind_address.trim().is_empty()
                || !(1..=30).contains(&self.streaming.fps)
                || self.streaming.jpeg_quality == 0)
        {
            bail!("streaming.port, streaming.bind_address, streaming.fps et streaming.jpeg_quality doivent être valides");
        }
        Ok(())
    }
}

impl Default for Nmea0183Config {
    fn default() -> Self {
        Self {
            transport: Transport::Tcp,
            address: "127.0.0.1:10110".into(),
        }
    }
}

impl Default for SignalKConfig {
    fn default() -> Self {
        Self {
            url: "ws://192.168.3.2/signalk/v1/stream?subscribe=none".into(),
            paths: SignalKPaths::default(),
        }
    }
}

impl Default for SignalKPaths {
    fn default() -> Self {
        Self {
            position: "navigation.position".into(),
            course_over_ground: "navigation.courseOverGroundLastMinute".into(),
            speed_over_ground: "navigation.speedOverGroundLastMinute".into(),
            heading: "navigation.headingTrue".into(),
            ais_position: "navigation.position".into(),
            ais_name: "name".into(),
            ais_course_over_ground: "navigation.courseOverGroundTrue".into(),
            ais_speed_over_ground: "navigation.speedOverGround".into(),
        }
    }
}

impl Default for ChartsConfig {
    fn default() -> Self {
        Self {
            directory: "map".into(),
            default: "assets/OSM.mbtiles".into(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "KALYMNOS".into(),
            fullscreen: false,
            width: 1600,
            height: 800,
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            file: "assets/Montserrat-Regular.ttf".into(),
            window_size: 28,
            ui_size: 16,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            boat_size: 6,
            boat_circle_radius: 50,
            mark_size: 6,
            route_point_size: 6,
            route_width: 3,
            cog_length: 250.0,
            cog_width: 4,
            ais_course_length: 28,
            ais_course_width: 3,
            ais_size: 7,
            ais_outline_size: 9,
            boat_color: [255, 0, 0, 255],
            boat_no_fix_color: [128, 128, 128, 255],
            measure_color: [0, 0, 255, 255],
            route_color: [255, 0, 0, 255],
            track_color: [255, 0, 0, 255],
            waypoint_color: [220, 30, 30, 255],
            coverage_color: [255, 140, 0, 255],
            cog_color: [0, 0, 255, 255],
            ais_color: [255, 140, 0, 255],
            ais_outline_color: [0, 0, 0, 255],
            map_background_color: [250, 250, 250, 255],
            line_color: [0, 0, 255, 255],
            night_overlay_color: [255, 0, 0, 255],
        }
    }
}

impl Default for RenderingConfig {
    fn default() -> Self {
        Self {
            tile_cache_size: 512,
            prefetch_margin: 2,
        }
    }
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            checkpoint_interval_seconds: 15,
        }
    }
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_address: "0.0.0.0".into(),
            port: 8080,
            fps: 5,
            jpeg_quality: 70,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;
    use super::{InputSource, Transport};

    #[test]
    fn a_config_without_an_input_table_reads_signal_k() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert_eq!(config.input.source, InputSource::Signalk);
        assert_eq!(config.nmea0183.transport, Transport::Tcp);
        assert_eq!(config.nmea0183.address, "127.0.0.1:10110");
        assert_eq!(config.signalk.paths.heading, "navigation.headingTrue");
        assert_eq!(config.appearance.boat_color, [255, 0, 0, 255]);
        assert_eq!(config.appearance.boat_no_fix_color, [128, 128, 128, 255]);
    }

    #[test]
    fn nmea0183_input_reads_its_own_table() {
        let config: AppConfig = toml::from_str(
            "[input]
source = \"nmea0183\"
[nmea0183]
transport = \"udp\"
address = \"0.0.0.0:10110\"
",
        )
        .unwrap();
        assert_eq!(config.input.source, InputSource::Nmea0183);
        assert_eq!(config.nmea0183.transport, Transport::Udp);
        assert_eq!(config.nmea0183.address, "0.0.0.0:10110");
        config.validate().unwrap();
    }

    #[test]
    fn a_bad_nmea0183_address_is_rejected_only_when_that_source_is_chosen() {
        let mut config = AppConfig::default();
        config.nmea0183.address = "not an address".into();
        config.validate().unwrap();
        config.input.source = InputSource::Nmea0183;
        assert!(config.validate().is_err());
    }

    #[test]
    fn the_signal_k_url_is_not_checked_when_nmea0183_is_chosen() {
        let mut config = AppConfig::default();
        config.signalk.url = "http://wrong".into();
        assert!(config.validate().is_err());
        config.input.source = InputSource::Nmea0183;
        config.validate().unwrap();
    }

    #[test]
    fn nmea2k_is_accepted_by_the_config() {
        let config: AppConfig = toml::from_str(
            "[input]
source = \"nmea2k\"
",
        )
        .unwrap();
        assert_eq!(config.input.source, InputSource::Nmea2k);
        config.validate().unwrap();
    }

    #[test]
    fn omitted_fields_use_defaults() {
        let config: AppConfig = toml::from_str("[window]\nwidth = 1024").unwrap();
        assert_eq!(config.window.width, 1024);
        assert_eq!(config.window.height, 800);
        assert_eq!(config.rendering.tile_cache_size, 512);
        config.validate().unwrap();
    }

    #[test]
    fn default_configuration_round_trips() {
        let serialized = toml::to_string(&AppConfig::default()).unwrap();
        let config: AppConfig = toml::from_str(&serialized).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn invalid_websocket_url_is_rejected() {
        let mut config = AppConfig::default();
        config.signalk.url = "http://localhost".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_appearance_sizes_are_rejected() {
        let mut config = AppConfig::default();
        config.appearance.boat_size = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn missing_configuration_is_created_with_defaults() {
        let path =
            std::env::temp_dir().join(format!("plotter-config-test-{}.toml", std::process::id()));
        let config = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(config.window.width, 1600);
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
    }
}
