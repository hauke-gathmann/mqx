use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, Result};

const APP_NAME: &str = "mqx";

/// Default in-memory topic-store cap (12 GiB).
pub const DEFAULT_RAM_LIMIT_BYTES: u64 = 12 * 1024 * 1024 * 1024;
pub const RAM_LIMIT_MIN_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const RAM_LIMIT_MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;

pub fn clamp_ram_limit_bytes(bytes: u64) -> u64 {
    bytes.clamp(RAM_LIMIT_MIN_BYTES, RAM_LIMIT_MAX_BYTES)
}

fn deserialize_ram_limit<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(clamp_ram_limit_bytes(u64::deserialize(deserializer)?))
}

#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppDirs {
    pub fn new() -> Result<Self> {
        let base = directories::BaseDirs::new().ok_or(Error::Directories)?;
        let this = Self {
            config_dir: base.config_dir().join(APP_NAME),
            cache_dir: base.cache_dir().join(APP_NAME),
        };
        fs::create_dir_all(&this.config_dir)?;
        fs::create_dir_all(&this.cache_dir)?;
        Ok(this)
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn connections_file(&self) -> PathBuf {
        self.config_dir.join("connections.json")
    }

    pub fn secrets_dir(&self) -> PathBuf {
        self.config_dir.join("secrets")
    }

    pub fn log_file(&self) -> PathBuf {
        self.cache_dir.join("mqx.log")
    }

    pub fn jq_history_file(&self) -> PathBuf {
        self.cache_dir.join("history.jq")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub keys: KeyConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "defaults::theme")]
    pub theme: String,
    #[serde(default)]
    pub buffer_size: usize,
    #[serde(default = "defaults::fresh_until", with = "humantime_serde")]
    pub fresh_until: Duration,
    #[serde(default = "defaults::stale_after", with = "humantime_serde")]
    pub stale_after: Duration,
    #[serde(
        default = "defaults::ram_limit_bytes",
        deserialize_with = "deserialize_ram_limit"
    )]
    pub ram_limit_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyConfig {
    #[serde(default = "defaults::search")]
    pub search: char,
    #[serde(default = "defaults::ignore")]
    pub ignore: char,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: defaults::theme(),
            buffer_size: 0,
            fresh_until: defaults::fresh_until(),
            stale_after: defaults::stale_after(),
            ram_limit_bytes: defaults::ram_limit_bytes(),
        }
    }
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            search: defaults::search(),
            ignore: defaults::ignore(),
        }
    }
}

impl AppConfig {
    pub fn path() -> Result<PathBuf> {
        Ok(AppDirs::new()?.config_file())
    }

    pub fn load() -> Result<Self> {
        let dirs = AppDirs::new()?;
        migrate_legacy(&dirs)?;
        Self::load_or_write(dirs.config_file())
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|source| Error::Toml {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn load_or_write(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let config = Self::default();
            fs::write(path, toml::to_string_pretty(&config)?)?;
            return Ok(config);
        }
        Self::load_from(path)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(Self::path()?)
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Copy XDG `mqttui` config/history into the new `mqx` locations once.
pub(crate) fn migrate_legacy(dirs: &AppDirs) -> Result<()> {
    let new_config = dirs.config_file();
    if !new_config.exists()
        && let Some(old) = legacy_xdg_file("XDG_CONFIG_HOME", ".config", "config.toml")
        && old.exists()
    {
        from_legacy_toml(&old)?.save_to(&new_config)?;
    }

    let new_history = dirs.jq_history_file();
    if !new_history.exists()
        && let Some(old) = legacy_xdg_file("XDG_CACHE_HOME", ".cache", "history.jq")
        && old.exists()
    {
        if let Some(parent) = new_history.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(old, new_history)?;
    }
    Ok(())
}

fn legacy_xdg_file(env: &str, fallback: &str, name: &str) -> Option<PathBuf> {
    let base = std::env::var_os(env)
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(fallback)))?;
    Some(base.join("mqttui").join(name))
}

fn from_legacy_toml(path: &Path) -> Result<AppConfig> {
    let content = fs::read_to_string(path)?;
    from_legacy_str(&content).map_err(|source| Error::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn from_legacy_str(content: &str) -> std::result::Result<AppConfig, toml::de::Error> {
    #[derive(Deserialize, Default)]
    struct LegacyFile {
        #[serde(default)]
        topics: LegacyTopics,
        #[serde(default)]
        keys: LegacyKeys,
    }

    #[derive(Deserialize)]
    struct LegacyTopics {
        #[serde(default)]
        buffer_size: usize,
        #[serde(default = "defaults::fresh_until", with = "humantime_serde")]
        fresh_until: Duration,
        #[serde(default = "defaults::stale_after", with = "humantime_serde")]
        stale_after: Duration,
    }

    impl Default for LegacyTopics {
        fn default() -> Self {
            Self {
                buffer_size: 0,
                fresh_until: defaults::fresh_until(),
                stale_after: defaults::stale_after(),
            }
        }
    }

    #[derive(Deserialize)]
    struct LegacyKeys {
        #[serde(default = "defaults::search")]
        search: char,
        #[serde(default = "defaults::ignore")]
        ignore: char,
    }

    impl Default for LegacyKeys {
        fn default() -> Self {
            Self {
                search: defaults::search(),
                ignore: defaults::ignore(),
            }
        }
    }

    if content.contains("[ui]") {
        return toml::from_str(content);
    }
    let legacy: LegacyFile = toml::from_str(content)?;
    Ok(AppConfig {
        ui: UiConfig {
            theme: defaults::theme(),
            buffer_size: legacy.topics.buffer_size,
            fresh_until: legacy.topics.fresh_until,
            stale_after: legacy.topics.stale_after,
            ram_limit_bytes: defaults::ram_limit_bytes(),
        },
        keys: KeyConfig {
            search: legacy.keys.search,
            ignore: legacy.keys.ignore,
        },
    })
}

mod defaults {
    use std::time::Duration;

    pub fn theme() -> String {
        "dark".into()
    }

    pub fn fresh_until() -> Duration {
        Duration::from_millis(500)
    }

    pub fn stale_after() -> Duration {
        Duration::from_secs(5)
    }

    pub fn search() -> char {
        '/'
    }

    pub fn ignore() -> char {
        '?'
    }

    pub fn ram_limit_bytes() -> u64 {
        super::DEFAULT_RAM_LIMIT_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_keys_are_ignored() {
        let parsed: AppConfig = toml::from_str(
            r#"
            extra = 1
            [ui]
            theme = "light"
            buffer_size = 4
            protocols = []
            [keys]
            search = "s"
            copy = "y"
            "#,
        )
        .unwrap();
        assert_eq!(parsed.ui.theme, "light");
        assert_eq!(parsed.ui.buffer_size, 4);
        assert_eq!(parsed.ui.fresh_until, Duration::from_millis(500));
        assert_eq!(parsed.ui.ram_limit_bytes, DEFAULT_RAM_LIMIT_BYTES);
        assert_eq!(parsed.keys.search, 's');
        assert_eq!(parsed.keys.ignore, '?');
    }

    #[test]
    fn load_or_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let loaded = AppConfig::load_or_write(&path).unwrap();
        assert_eq!(loaded, AppConfig::default());
        assert!(path.exists());
        let again = AppConfig::load_from(&path).unwrap();
        assert_eq!(again, loaded);
    }

    #[test]
    fn maps_legacy_topics_schema() {
        let parsed = from_legacy_str(
            r#"
            [topics]
            buffer_size = 8
            fresh_until = "1s"
            [keys]
            search = "f"
            "#,
        )
        .unwrap();
        assert_eq!(parsed.ui.buffer_size, 8);
        assert_eq!(parsed.ui.fresh_until, Duration::from_secs(1));
        assert_eq!(parsed.ui.stale_after, Duration::from_secs(5));
        assert_eq!(parsed.keys.search, 'f');
        assert_eq!(parsed.keys.ignore, '?');
    }

    #[test]
    fn ram_limit_bytes_clamped_to_product_range() {
        let low: AppConfig = toml::from_str("[ui]\nram_limit_bytes = 1\n").unwrap();
        assert_eq!(low.ui.ram_limit_bytes, RAM_LIMIT_MIN_BYTES);
        let high: AppConfig = toml::from_str("[ui]\nram_limit_bytes = 999999999999999\n").unwrap();
        assert_eq!(high.ui.ram_limit_bytes, RAM_LIMIT_MAX_BYTES);
        assert_eq!(clamp_ram_limit_bytes(0), RAM_LIMIT_MIN_BYTES);
        assert_eq!(
            clamp_ram_limit_bytes(DEFAULT_RAM_LIMIT_BYTES),
            DEFAULT_RAM_LIMIT_BYTES
        );
    }
}
