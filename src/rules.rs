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

use std::path::Path;

use crate::document::Document;
use crate::term::TermsRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn tag(&self) -> &'static str {
        match self {
            Level::Info => "ℹ",
            Level::Warn => "⚠ 标红",
            Level::Error => "✖ 阻塞",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub code: &'static str,
    pub message: String,
}

/// 校验上下文：一个文件 + 它在哪个状态目录里（真值）+ 术语。
pub struct ValidationCtx<'a> {
    pub path: &'a Path,
    pub doc: &'a Document,
    /// 该文件实际所在的状态目录（pool/working/finished/community）。
    pub located_status: &'a str,
    pub terms: &'a TermsRegistry,
    /// 生效的反证模式（config 覆盖 terms）：warn | block。
    pub falsification_mode: &'a str,
}

/// 规则 trait：可插拔（§6b）。新增规则 = 加一个 impl + 注册一行。
pub trait Rule {
    fn id(&self) -> &'static str;
    fn check(&self, ctx: &ValidationCtx) -> Vec<Finding>;
}

pub fn registry() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(SlugMatchesFile),
        Box::new(StatusLocation),
        Box::new(FalsificationRecorded),
        Box::new(RequiredFields),
    ]
}

/// `slug` 必须与文件名一致（§9.1）。
struct SlugMatchesFile;
impl Rule for SlugMatchesFile {
    fn id(&self) -> &'static str {
        "slug_matches_file"
    }
    fn check(&self, ctx: &ValidationCtx) -> Vec<Finding> {
        let stem = ctx.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem != ctx.doc.fm.slug {
            vec![Finding {
                level: Level::Error,
                code: "SlugMismatch",
                message: format!(
                    "slug `{}` 与文件名 `{stem}` 不一致，寻址会失败",
                    ctx.doc.fm.slug
                ),
            }]
        } else {
            vec![]
        }
    }
}

/// `status` 字段应与所在目录一致（真值是目录，§6 rule4）。
struct StatusLocation;
impl Rule for StatusLocation {
    fn id(&self) -> &'static str {
        "status_location"
    }
    fn check(&self, ctx: &ValidationCtx) -> Vec<Finding> {
        if ctx.doc.fm.status != ctx.located_status {
            vec![Finding {
                level: Level::Warn,
                code: "StatusDrift",
                message: format!(
                    "frontmatter status=`{}` 与所在目录 `{}` 不一致；真值是目录，请修正字段",
                    ctx.doc.fm.status, ctx.located_status
                ),
            }]
        } else {
            vec![]
        }
    }
}

/// 反证实验留痕（§5.1e）：缺失/空 → warn（block 模式则 error）。
struct FalsificationRecorded;
impl Rule for FalsificationRecorded {
    fn id(&self) -> &'static str {
        "falsification_recorded"
    }
    fn check(&self, ctx: &ValidationCtx) -> Vec<Finding> {
        // 已结束/社区互助不强制反证；只在推进路径上检查。
        if matches!(ctx.located_status, "finished" | "community") {
            return vec![];
        }
        let level = if ctx.falsification_mode == "block" {
            Level::Error
        } else {
            Level::Warn
        };
        if !ctx.doc.has_heading("反证实验") {
            return vec![Finding {
                level,
                code: "MissingFalsificationRecord",
                message:
                    "缺 `## 反证实验` 章节（写出来即可，或 promote --skip-falsification 登记待测）"
                        .into(),
            }];
        }
        let section = section_slice(&ctx.doc.body, "反证实验").unwrap_or_default();
        // 待测登记是 `### 待测`（反证实验的同级子标题）或 frontmatter 的 pending 标记；
        // 二者都要在整篇正文/frontmatter 层面检测，section 切片会漏掉同级子标题。
        let pending = ctx.doc.body.contains("### 待测")
            || ctx.doc.body.contains("[pending]")
            || ctx.doc.fm.falsification.as_deref() == Some("pending");
        let real = table_has_result(&section, &ctx.terms.falsification_evidence_tokens());
        if !pending && !real {
            return vec![Finding {
                level,
                code: "FalsificationNoEvidence",
                message: "`反证实验` 章节既无真实结果、也无 `### 待测`/具象跳过理由 → 仍是猜想"
                    .into(),
            }];
        }
        if pending && ctx.located_status == "working" {
            // working 里的 pending 提醒 complete 前清算（§5.1f）。
            return vec![Finding {
                level: Level::Info,
                code: "PendingVerification",
                message: "存在 `pending` 待测项：complete 前必须实际执行并填真实结果".into(),
            }];
        }
        vec![]
    }
}

/// 术语驱动的必填章节（§6b）：require_fields 里的标题子串缺失即提示。
struct RequiredFields;
impl Rule for RequiredFields {
    fn id(&self) -> &'static str {
        "required_fields"
    }
    fn check(&self, ctx: &ValidationCtx) -> Vec<Finding> {
        // 只对"进行中"的实质推进强制完整剪枝章节。
        if ctx.located_status == "finished" || ctx.located_status == "community" {
            return vec![];
        }
        ctx.terms
            .prune_require_fields()
            .into_iter()
            .filter(|h| !ctx.doc.has_heading(h))
            .map(|h| Finding {
                level: Level::Warn,
                code: "MissingPruneSection",
                message: format!("缺剪枝章节 `{h}`（术语驱动，改 terms.local.toml 即改规则）"),
            })
            .collect()
    }
}

/// 取某标题（规范化后匹配）到下一个同级/更高级标题之间的正文。
fn section_slice(body: &str, heading: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let start = lines
        .iter()
        .position(|l| crate::document::heading_matches(l, heading))?;
    let start_level = crate::document::heading_level(lines[start]);
    let mut out = Vec::new();
    for l in &lines[start + 1..] {
        if l.starts_with('#') {
            let lvl = crate::document::heading_level(l);
            if lvl <= start_level {
                break;
            }
        }
        out.push(*l);
    }
    Some(out.join("\n"))
}

/// 反证明文契约：表格里必须出现某个"结果标签"表头列（`tokens` 之一），且其下至少一个数据格非空。
/// 按标签定位列，不认列序号——避免写死第 4 列导致 3 列简表假阴性（§11 解析鲁棒性）。
/// 无任一识别标签 → 判为未满足契约（不再做"非空格即可"的宽松兜底）。
fn table_has_result(section: &str, tokens: &[String]) -> bool {
    let is_sep = |t: &str| t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    let is_tok = |c: &&str| tokens.iter().any(|t| t == c);
    let rows: Vec<Vec<&str>> = section
        .lines()
        .map(|l| l.trim())
        .filter(|t| t.starts_with('|') && !is_sep(t))
        .map(|t| t.trim_matches('|').split('|').map(|c| c.trim()).collect())
        .collect();
    if let Some((hdr, col)) = rows
        .iter()
        .enumerate()
        .find_map(|(i, cells)| cells.iter().position(is_tok).map(|c| (i, c)))
    {
        return rows.iter().skip(hdr + 1).any(|cells| {
            cells
                .get(col)
                .map(|v| !v.is_empty() && !is_tok(v))
                .unwrap_or(false)
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks() -> Vec<String> {
        ["真实结果", "实测结果", "实际结果"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn empty_table_has_no_result() {
        let s = "| 假设 | 实验 | 预期 | 真实结果 |\n|---|---|---|---|\n|  |  |  |  |";
        assert!(!table_has_result(s, &toks()));
    }

    #[test]
    fn filled_result_detected() {
        let s = "| 假设 | 实验 | 预期 | 真实结果 |\n|---|---|---|---|\n| A | `find` | x | 42 XML 0 命中 |";
        assert!(table_has_result(s, &toks()));
    }

    #[test]
    fn three_column_result_detected() {
        // 3 列简表，结果列在最后：早期按第 4 列判会漏（本次修复要覆盖的假阴性）。
        let s = "| 假设 | 依据 | 真实结果 |\n|---|---|---|\n| 覆盖非本意 | git stat | 成立，275→40 行 |";
        assert!(table_has_result(s, &toks()));
    }

    #[test]
    fn three_column_empty_not_result() {
        let s = "| 假设 | 依据 | 真实结果 |\n|---|---|---|\n|  |  |  |";
        assert!(!table_has_result(s, &toks()));
    }

    #[test]
    fn no_result_label_is_not_evidence() {
        // 明文契约：没有约定标签列，即使别处有字也不算证据（不再宽松兜底）。
        let s = "| 假设 | 依据 | 结论 |\n|---|---|---|\n| A | x | 成立 |";
        assert!(!table_has_result(s, &toks()));
    }

    #[test]
    fn section_slice_stops_at_sibling_heading() {
        let body = "### 反证实验\ntable\n### 待测\n- [pending] x\n## 下一节\nsecret";
        let sec = section_slice(body, "反证实验").unwrap();
        // 反证实验(level3) 的切片止于同级 `### 待测`，不含其后内容；更不含 ## 下一节。
        assert!(sec.contains("table"));
        assert!(!sec.contains("[pending]"));
        assert!(!sec.contains("secret"));
    }
}
