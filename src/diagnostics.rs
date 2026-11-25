use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// Size at which `plotter.log` is rotated to `plotter.log.1`. One backup is
/// kept, so the log never uses more than twice this on disk.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Environment variable selecting the lowest level written to the log:
/// `debug`, `info` (default), `warn` or `error`.
pub const LEVEL_ENV_VAR: &str = "PLOTTER_LOG";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    Panic,
}

impl Level {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            "panic" => Some(Self::Panic),
            _ => None,
        }
    }
}

/// Appends timestamped lines to a log file, dropping lines below a minimum
/// level and rotating the file once it reaches a size cap.
pub struct Logger {
    path: PathBuf,
    file: File,
    written: u64,
    min_level: Level,
    max_bytes: u64,
}

impl Logger {
    pub fn new(path: &Path, min_level: Level, max_bytes: u64) -> io::Result<Self> {
        let file = Self::open(path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file,
            written,
            min_level,
            max_bytes,
        })
    }

    fn open(path: &Path) -> io::Result<File> {
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// Writes one line. Returns false when the level is below the minimum.
    /// An unknown level name is treated as INFO so it is never lost.
    pub fn record(&mut self, level: &str, message: &str) -> bool {
        if Level::parse(level).unwrap_or(Level::Info) < self.min_level {
            return false;
        }
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("{timestamp} [{level}] {message}\n");

        if self.written > 0 && self.written + line.len() as u64 > self.max_bytes {
            if let Err(error) = self.rotate() {
                eprintln!(
                    "Impossible de faire tourner {} : {error}",
                    self.path.display()
                );
            }
        }
        match self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.flush())
        {
            Ok(()) => self.written += line.len() as u64,
            Err(error) => {
                eprintln!("Impossible d'écrire dans {} : {error}", self.path.display());
                eprint!("{line}");
            }
        }
        true
    }

    /// Moves the current file to `<name>.1`, replacing any older backup, and
    /// starts a fresh file.
    fn rotate(&mut self) -> io::Result<()> {
        let backup = self.path.with_extension("log.1");
        std::fs::rename(&self.path, &backup)?;
        self.file = Self::open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

/// Opens the application log. The minimum level comes from `PLOTTER_LOG`
/// and defaults to INFO, so DEBUG lines are not written on the boat unless
/// asked for.
pub fn init(path: &Path) -> io::Result<()> {
    let min_level = std::env::var(LEVEL_ENV_VAR)
        .ok()
        .and_then(|value| Level::parse(&value))
        .unwrap_or(Level::Info);
    let logger = Logger::new(path, min_level, MAX_LOG_BYTES)?;
    let _ = LOGGER.set(Mutex::new(logger));
    Ok(())
}

pub fn record(level: &str, message: impl std::fmt::Display) {
    let message = message.to_string();
    match LOGGER.get() {
        Some(logger) => {
            logger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record(level, &message);
        }
        None => {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            eprintln!("{timestamp} [{level}] {message}");
        }
    }
}

/// An error that can repeat every frame: the first time it is logged, then
/// only every thousandth time, so a lasting fault cannot fill the log.
#[derive(Default)]
pub struct Repeated {
    last: Option<String>,
    count: u64,
}

impl Repeated {
    /// The line to log for this error, if any.
    pub fn note(&mut self, message: &str) -> Option<String> {
        if self.last.as_deref() != Some(message) {
            self.last = Some(message.to_string());
            self.count = 1;
            return Some(message.to_string());
        }
        self.count += 1;
        self.count
            .is_multiple_of(1000)
            .then(|| format!("{message} ({} fois)", self.count))
    }

    /// The result of one step of the main loop: its value, or `None` after
    /// logging the error (`level`, prefixed with `context`) the way `note`
    /// allows. A success clears the error. The caller carries on either
    /// way: the chart must keep drawing.
    pub fn report<T, E: std::fmt::Display>(
        &mut self,
        result: Result<T, E>,
        level: &str,
        context: &str,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.clear();
                Some(value)
            }
            Err(error) => {
                if let Some(line) = self.note(&error.to_string()) {
                    record(level, format!("{context}: {line}"));
                }
                None
            }
        }
    }

    /// The operation worked again: the next error is logged at once.
    pub fn clear(&mut self) {
        self.last = None;
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_passes_a_value_through_and_turns_an_error_into_none() {
        let mut repeated = Repeated::default();
        assert_eq!(
            repeated.report(Ok::<_, String>(5), "ERROR", "étape"),
            Some(5)
        );
        assert_eq!(
            repeated.report(Err::<i32, _>("lost"), "ERROR", "étape"),
            None
        );
        assert_eq!(repeated.note("lost"), None, "the error was noted");
        assert_eq!(
            repeated.report(Ok::<_, String>(6), "ERROR", "étape"),
            Some(6)
        );
        assert_eq!(
            repeated.note("lost"),
            Some("lost".to_string()),
            "a success cleared it"
        );
    }

    #[test]
    fn a_repeated_error_is_logged_once_then_every_thousandth_time() {
        let mut repeated = Repeated::default();
        assert_eq!(repeated.note("lost"), Some("lost".to_string()));
        for _ in 2..1000 {
            assert_eq!(repeated.note("lost"), None);
        }
        assert_eq!(repeated.note("lost"), Some("lost (1000 fois)".to_string()));
        assert_eq!(
            repeated.note("other"),
            Some("other".to_string()),
            "a new message"
        );
        repeated.clear();
        assert_eq!(
            repeated.note("other"),
            Some("other".to_string()),
            "after a success"
        );
    }

    fn temp_log(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("plotter-log-{tag}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("plotter.log")
    }

    #[test]
    fn levels_parse_case_insensitively() {
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("INFO"), Some(Level::Info));
        assert_eq!(Level::parse("Warn"), Some(Level::Warn));
        assert_eq!(Level::parse("error"), Some(Level::Error));
        assert_eq!(Level::parse("PANIC"), Some(Level::Panic));
        assert_eq!(Level::parse("verbose"), None);
        assert!(Level::Debug < Level::Info && Level::Info < Level::Warn);
    }

    #[test]
    fn lines_below_the_minimum_level_are_not_written() {
        let path = temp_log("level");
        let mut logger = Logger::new(&path, Level::Info, 1_000_000).unwrap();

        assert!(!logger.record("DEBUG", "mouse moved"));
        assert!(logger.record("INFO", "started"));
        assert!(logger.record("WARN", "slow"));
        drop(logger);

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("mouse moved"));
        assert!(contents.contains("[INFO] started"));
        assert!(contents.contains("[WARN] slow"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn log_rotates_to_one_backup_when_the_size_cap_is_reached() {
        let path = temp_log("rotate");
        let backup = path.with_extension("log.1");
        let cap = 400;
        let mut logger = Logger::new(&path, Level::Debug, cap).unwrap();
        for i in 0..40 {
            assert!(logger.record("INFO", &format!("line number {i:02}")));
        }
        drop(logger);

        assert!(backup.exists(), "a backup should exist after rotation");
        assert!(std::fs::metadata(&path).unwrap().len() <= cap);
        assert!(std::fs::metadata(&backup).unwrap().len() <= cap);
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.contains("line number 39"));
        // Only one backup is kept, so the oldest lines are gone.
        assert!(!current.contains("line number 00"));
        assert!(!std::fs::read_to_string(&backup)
            .unwrap()
            .contains("line number 00"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reopening_an_existing_log_counts_its_current_size() {
        let path = temp_log("reopen");
        std::fs::write(&path, "x".repeat(390)).unwrap();
        let mut logger = Logger::new(&path, Level::Debug, 400).unwrap();

        assert!(logger.record("INFO", "this line pushes the file over the cap"));
        drop(logger);

        assert!(path.with_extension("log.1").exists());
        assert!(std::fs::metadata(&path).unwrap().len() < 390);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
