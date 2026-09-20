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
    /// 反证留痕模式：warn（默认，标红不阻塞）| block（项目级可升，§5.1e/§6）。
    #[serde(default)]
    pub mode: Option<String>,
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
}

/// 术语注册表：每次执行热加载，不缓存到二进制（§2.1b「热加载」）。
#[derive(Debug, Default)]
pub struct TermsRegistry {
    pub terms: BTreeMap<String, Term>,
    pub quick_limit: usize,
}

impl TermsRegistry {
    /// 热加载。文件不存在时返回内置默认（保证 validate/context 在未初始化目录也不炸）。
    pub fn load(path: &Path) -> Result<TermsRegistry> {
        if !path.exists() {
            return Ok(TermsRegistry::defaults());
        }
        let raw = std::fs::read_to_string(path)?;
        let root: Root = toml::from_str(&raw).map_err(|e| Error::Toml {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Ok(TermsRegistry {
            terms: root.term,
            quick_limit: root.quick_limit.unwrap_or(5),
        })
    }

    pub fn get(&self, slug: &str) -> Option<&Term> {
        self.terms.get(slug)
    }

    /// 反证章节标题（术语 synonyms/falsification）。
    pub fn falsification_mode(&self) -> &str {
        self.terms
            .get("falsification")
            .and_then(|t| t.mode.as_deref())
            .unwrap_or("warn")
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

    /// 内置默认（首批 fidus 术语，§10 Phase 1b）。
    fn defaults() -> TermsRegistry {
        let raw = include_str!("../templates/terms.local.toml.tpl");
        let root: Root = toml::from_str(raw).unwrap_or_default();
        TermsRegistry {
            terms: root.term,
            quick_limit: root.quick_limit.unwrap_or(5),
        }
    }
}
