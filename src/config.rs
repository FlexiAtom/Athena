use serde::Deserialize;

use crate::error::Result;
use crate::store::Store;

#[derive(Debug, Default, Deserialize)]
pub struct Behavior {
    #[serde(default)]
    pub falsification_mode: Option<String>,
    #[serde(default)]
    pub max_entry_lines: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default_project: Option<String>,
    #[serde(default)]
    pub behavior: Behavior,
}

impl Config {
    /// 热加载 config.toml；缺失则用默认。
    pub fn load(store: &Store) -> Result<Config> {
        let path = store.config_toml();
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg: Config = toml::from_str(&raw).map_err(|e| crate::error::Error::Toml {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Ok(cfg)
    }
}
