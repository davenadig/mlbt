use crate::components::constants::TEAM_IDS;
use crate::state::app_settings::{AppSettings, compute_timezone_abbreviation};
use anyhow::Context;
use chrono_tz::Tz;
use chrono_tz::Tz::US__Pacific;
use directories::ProjectDirs;
use log::{LevelFilter, error};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static CONFIG_FILE_LOCATION: OnceLock<Option<PathBuf>> = OnceLock::new();
pub const DEFAULT_TIMEZONE: Tz = US__Pacific;
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Error;
pub const DEFAULT_AUTO_ADVANCE_DATE: bool = true;
/// Off. A delay only helps when watching a broadcast, so it is opt in.
pub const DEFAULT_TV_DELAY_SECONDS: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// `mlbt.toml` reader/writer.
///
/// - Fields are `Option` so a partial or hand-edited file still parses.
/// - Missing fields fall back to defaults when converting into `AppSettings`.
#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    /// See the `TEAM_NAMES` map in `components/constants.rs` for options.
    pub favorite_team: Option<String>,

    /// Timezone to display game start times in. Common options are:
    /// * "US/Pacific"
    /// * "US/Mountain"
    /// * "US/Central"
    /// * "US/Eastern"
    ///
    /// For the full list see https://en.wikipedia.org/wiki/List_of_tz_database_time_zones.
    pub timezone: Option<Tz>,

    /// Optional log level to use. If not present, the default is `Error`.
    /// Set the level using a lowercase string, e.g. "error".
    pub log_level: Option<LogLevel>,

    /// Automatically advance any date-driven tab to the new day when the system date rolls over,
    /// unless the tab selected a past date. Defaults to true.
    pub auto_advance_date: Option<bool>,

    /// Hold live game data this many seconds before displaying it, so the terminal
    /// stays behind a television broadcast instead of spoiling it. 0 disables the
    /// delay. The settings picker offers 5/15/30/45; any value parses here.
    pub tv_delay_seconds: Option<u16>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            favorite_team: None,
            timezone: Some(DEFAULT_TIMEZONE),
            log_level: Some(DEFAULT_LOG_LEVEL),
            auto_advance_date: Some(DEFAULT_AUTO_ADVANCE_DATE),
            tv_delay_seconds: Some(DEFAULT_TV_DELAY_SECONDS),
        }
    }
}

impl From<ConfigFile> for AppSettings {
    fn from(file: ConfigFile) -> Self {
        let favorite_team = file
            .favorite_team
            .as_deref()
            .and_then(|name| TEAM_IDS.get(name).copied());
        let timezone = file.timezone.unwrap_or(DEFAULT_TIMEZONE);
        let timezone_abbreviation = compute_timezone_abbreviation(timezone);
        let log_level = file.log_level.unwrap_or(DEFAULT_LOG_LEVEL);
        let auto_advance_date = file.auto_advance_date.unwrap_or(DEFAULT_AUTO_ADVANCE_DATE);
        let tv_delay_seconds = file.tv_delay_seconds.unwrap_or(DEFAULT_TV_DELAY_SECONDS);
        Self {
            favorite_team,
            full_screen: false,
            timezone,
            timezone_abbreviation,
            log_level,
            auto_advance_date,
            tv_delay_seconds,
        }
    }
}

impl From<&AppSettings> for ConfigFile {
    fn from(s: &AppSettings) -> Self {
        Self {
            favorite_team: s.favorite_team.map(|t| t.name.to_string()),
            timezone: Some(s.timezone),
            log_level: Some(s.log_level),
            auto_advance_date: Some(s.auto_advance_date),
            tv_delay_seconds: Some(s.tv_delay_seconds),
        }
    }
}

/// Filesystem settings store. Reads and writes `mlbt.toml` in the platform config directory.
pub struct TomlFileStore {
    path: Option<PathBuf>,
}

impl Default for TomlFileStore {
    fn default() -> Self {
        Self {
            path: Self::default_path(),
        }
    }
}

impl TomlFileStore {
    const HEADER: &str = "# See https://github.com/mlb-rs/mlbt#config for options\n";
    const CONFIG_FILE_NAME: &str = "mlbt.toml";

    /// Build a store backed by a specific path. Used in tests so the real config file isn't clobbered.
    #[cfg(test)]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// Resolve the canonical config path:
    /// * Linux:   /home/alice/.config/mlbt/mlbt.toml
    /// * Windows: C:\Users\Alice\AppData\Roaming\mlbt\mlbt.toml
    /// * macOS:   /Users/Alice/Library/Application Support/mlbt/mlbt.toml
    pub fn default_path() -> Option<PathBuf> {
        CONFIG_FILE_LOCATION
            .get_or_init(|| {
                if let Some(proj_dirs) = ProjectDirs::from("", "", "mlbt") {
                    let dir = proj_dirs.config_dir();
                    if !dir.exists()
                        && let Err(err) = std::fs::create_dir_all(dir)
                    {
                        error!("could not create config dir: {err:?}");
                    }
                    Some(dir.join(Self::CONFIG_FILE_NAME))
                } else {
                    error!("could not get valid home directory for config file");
                    None
                }
            })
            .clone()
    }

    pub fn load(&self) -> anyhow::Result<AppSettings> {
        let path = self.path.as_ref().context("could not find config file")?;
        if !path.exists() {
            Self::write(path, &ConfigFile::default())?;
        }
        let contents = std::fs::read_to_string(path).context("could not read config file")?;
        let file: ConfigFile =
            toml::from_str(&contents).context("could not deserialize config file")?;
        Ok(file.into())
    }

    pub fn save(&self, settings: &AppSettings) -> anyhow::Result<()> {
        let path = self.path.as_ref().context("could not find config file")?;
        Self::write(path, &ConfigFile::from(settings))
    }

    fn write(path: &Path, file: &ConfigFile) -> anyhow::Result<()> {
        let body = toml::to_string(file).context("could not serialize config")?;
        // always rewrite the default header comment
        let contents = format!("{}{body}", Self::HEADER);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, contents).context("could not write config file")?;
        std::fs::rename(&tmp, path).context("could not finalize config file")?;
        Ok(())
    }
}

impl From<LogLevel> for LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Off => LevelFilter::Off,
            LogLevel::Trace => LevelFilter::Trace,
            LogLevel::Debug => LevelFilter::Debug,
            LogLevel::Info => LevelFilter::Info,
            LogLevel::Warn => LevelFilter::Warn,
            LogLevel::Error => LevelFilter::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mlbt.toml written before the delay existed lacks the key entirely, so
    /// loading one must not fail or turn a delay on for someone who never asked.
    #[test]
    fn a_config_without_a_delay_loads_with_it_off() {
        let dir = std::env::temp_dir().join("mlbt-config-no-delay");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mlbt.toml");
        std::fs::write(
            &path,
            "favorite_team = \"Boston Red Sox\"\ntimezone = \"US/Eastern\"\nlog_level = \"error\"\nauto_advance_date = true\n",
        )
        .unwrap();

        let settings = TomlFileStore::with_path(path.clone()).load().unwrap();
        assert_eq!(settings.tv_delay_seconds, DEFAULT_TV_DELAY_SECONDS);
        assert_eq!(settings.tv_delay(), std::time::Duration::ZERO);
        // the rest of the file still round trips
        assert_eq!(settings.timezone, chrono_tz::US::Eastern);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_saved_delay_survives_a_reload() {
        let dir = std::env::temp_dir().join("mlbt-config-delay-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mlbt.toml");
        let store = TomlFileStore::with_path(path.clone());

        let mut settings = store.load().unwrap();
        settings.tv_delay_seconds = 30;
        store.save(&settings).unwrap();

        assert_eq!(store.load().unwrap().tv_delay_seconds, 30);

        std::fs::remove_dir_all(&dir).ok();
    }
}
