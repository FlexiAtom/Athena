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

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

/// 单个术语的机读定义（terms.local.toml 的 `[term.<slug>]`，§2.1b）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Term {
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub require_fields: Vec<String>,
    #[serde(default)]
    pub enforce_on: Vec<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub definition: Option<String>,
    // 注：反证模式开关**不在这里**（C35）——从前 `[term.falsification].mode` 与
    // `config.toml [behavior].falsification_mode` 两处并存，config 压死 terms，而 config
    // 语法坏时错误被 `.ok()` 吞掉、回落到 terms 的值，生效值随"哪份坏了"翻转。
    // 现在 config 是唯一权威；本文件里残留的 `mode` 键由 `term validate` 点名报废弃。
    #[serde(default)]
    pub skip_field: Option<String>,
    #[serde(default)]
    pub is_entry: bool,
    #[serde(default)]
    pub is_recorded: bool,
    #[serde(default, rename = "schema_version")]
    pub schema_version: Option<u32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct Root {
    #[serde(default)]
    term: BTreeMap<String, Term>,
    /// 快速通道累计上限（§5.4），默认 5。
    #[serde(default)]
    quick_limit: Option<usize>,
    /// 顶层未知键（`term validate` 用来点名拼错/已废弃的键，C19）。
    #[serde(default, flatten)]
    extra: BTreeMap<String, toml::Value>,
}

/// `[term.*]` 里 CLI 真正读取的键。其余键（除已知的 `evidence_tokens`）一律由
/// `term validate` 点名——从前未知键静默进 `extra` 再静默忽略，拼错的
/// `require_field` 与已废弃的 `mode` 都不吭声。
pub const KNOWN_TERM_KEYS: &[&str] = &[
    "slug",
    "synonyms",
    "require_fields",
    "enforce_on",
    "origin",
    "definition",
    "skip_field",
    "is_entry",
    "is_recorded",
    "schema_version",
    "evidence_tokens",
];
/// 顶层已知键。
pub const KNOWN_ROOT_KEYS: &[&str] = &["term", "quick_limit"];

/// 术语注册表：每次执行热加载，不缓存到二进制（§2.1b「热加载」）。
#[derive(Debug, Default)]
pub struct TermsRegistry {
    pub terms: BTreeMap<String, Term>,
    pub quick_limit: usize,
    /// 文件里声明过的顶层未知键（含已废弃的 `mode` 所在表之外的键）。
    pub extra_root_keys: Vec<String>,
}

impl TermsRegistry {
    /// 热加载：**内置默认为底，文件项按表名覆盖**（C37）。从前该文件一旦存在就整体
    /// 取代内置默认——手写一个只含自己一条术语的文件，会把 `prune.require_fields`
    /// 驱动的机器覆盖（禁忌 2/4）静默清零，validate 一字不提。
    /// 覆盖是**整表**粒度：你写了 `[term.prune]` 就是那一整张表说了算，少了哪个字段
    /// 默认值不会替你补——这种"覆盖了但覆盖没了"由 `term validate` 点名。
    pub fn load(path: &Path) -> Result<TermsRegistry> {
        let mut reg = TermsRegistry::defaults();
        if !path.exists() {
            return Ok(reg);
        }
        let raw = std::fs::read_to_string(path)?;
        let root: Root = toml::from_str(&raw).map_err(|e| Error::Toml {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        for (slug, t) in root.term {
            reg.terms.insert(slug, t);
        }
        reg.quick_limit = root.quick_limit.unwrap_or(reg.quick_limit);
        reg.extra_root_keys = root.extra.keys().cloned().collect();
        Ok(reg)
    }

    pub fn get(&self, slug: &str) -> Option<&Term> {
        self.terms.get(slug)
    }

    /// 反证明文契约允许的结果标签（§11 解析鲁棒性：按标签定位，不写死列序）。
    /// 术语即配置：改 `terms.local.toml` 的 `[term.falsification].evidence_tokens` 即改校验规则。
    pub fn falsification_evidence_tokens(&self) -> Vec<String> {
        let from_cfg = self
            .terms
            .get("falsification")
            .and_then(|t| t.extra.get("evidence_tokens"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        if !from_cfg.is_empty() {
            return from_cfg;
        }
        vec![
            "真实结果".to_string(),
            "实测结果".to_string(),
            "实际结果".to_string(),
        ]
    }

    /// 剪枝必填字段（来自 prune.require_fields），驱动 required_fields 规则（§6b）。
    pub fn prune_require_fields(&self) -> Vec<String> {
        self.terms
            .get("prune")
            .map(|t| t.require_fields.clone())
            .unwrap_or_default()
    }

    pub fn prune_enforce_on(&self) -> Vec<String> {
        self.terms
            .get("prune")
            .map(|t| t.enforce_on.clone())
            .unwrap_or_default()
    }

    /// 内置默认（首批 fidus 术语，§10 Phase 1b）。`load` 用它作**底**再叠文件项。
    pub fn defaults() -> TermsRegistry {
        let raw = include_str!("../templates/terms.local.toml.tpl");
        let root: Root = toml::from_str(raw).unwrap_or_default();
        TermsRegistry {
            terms: root.term,
            quick_limit: root.quick_limit.unwrap_or(5),
            extra_root_keys: Vec::new(),
        }
    }
}
