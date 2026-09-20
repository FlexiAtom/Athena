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
                message: "缺 `## 反证实验` 章节（写出来即可，或 promote --skip-falsification 登记待测）".into(),
            }];
        }
        let section = section_slice(&ctx.doc.body, "反证实验").unwrap_or_default();
        // 待测登记是 `### 待测`（反证实验的同级子标题）或 frontmatter 的 pending 标记；
        // 二者都要在整篇正文/frontmatter 层面检测，section 切片会漏掉同级子标题。
        let pending = ctx.doc.body.contains("### 待测")
            || ctx.doc.body.contains("[pending]")
            || ctx.doc.fm.falsification.as_deref() == Some("pending");
        let real = table_has_result(&section);
        if !pending && !real {
            return vec![Finding {
                level,
                code: "FalsificationNoEvidence",
                message: "`反证实验` 章节既无真实结果、也无 `### 待测`/具象跳过理由 → 仍是猜想".into(),
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
    let start = lines.iter().position(|l| crate::document::heading_matches(l, heading))?;
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

/// Markdown 表格中是否存在某数据行的"真实结果"列（第 4 列）非空。
fn table_has_result(section: &str) -> bool {
    for line in section.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            continue;
        }
        // 跳过分隔行 |---|---|
        if t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(|c| c.trim()).collect();
        if cells.len() >= 4 {
            // 排除表头行（含"假设"/"真实结果"字样）
            let is_header = cells.iter().any(|c| *c == "假设" || *c == "真实结果");
            if !is_header && !cells[3].is_empty() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_has_no_result() {
        let s = "| 假设 | 实验 | 预期 | 真实结果 |\n|---|---|---|---|\n|  |  |  |  |";
        assert!(!table_has_result(s));
    }

    #[test]
    fn filled_result_detected() {
        let s = "| 假设 | 实验 | 预期 | 真实结果 |\n|---|---|---|---|\n| A | `find` | x | 42 XML 0 命中 |";
        assert!(table_has_result(s));
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
