// Athena — 面向 AI Agent 的工作协议 CLI
// Copyright (C) 2026 FlexiAtom
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but
// WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

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
    /// Athena 源码仓库在本机的位置，供 `athena onerror` 打印（不烤进二进制：换机/换宿主各自配）。
    #[serde(default)]
    pub source_repo: Option<String>,
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
