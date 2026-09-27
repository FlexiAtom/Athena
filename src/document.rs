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

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// 文档类型（轴一 · 它是什么，写在文档自己身上，§1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Proposal,
    Draft,
    Plan,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Proposal => "proposal",
            Kind::Draft => "draft",
            Kind::Plan => "plan",
        }
    }
}

/// 结束时的结果（同在 finished/ 用此字段区分，§1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Done,
    Frozen,
}

/// frontmatter 的机读结构（§2.1 统一骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub slug: String,
    pub kind: Kind,
    /// 推进状态的冗余快照，真值在目录（§1）。
    pub status: String,
    pub project: String,
    #[serde(rename = "updated-by")]
    pub updated_by: String,
    #[serde(rename = "updated-at")]
    pub updated_at: String,
    #[serde(rename = "content-hash")]
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// `falsification: pending`（§5.1f 待测标记）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falsification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// frontmatter 里**不属于协议固定键**的原始行（含注释与嵌套子结构），逐行原文保留。
    /// serde 不认的键过去被静默丢弃：任何状态动作（连 `write`/`append`）都会把整个
    /// frontmatter 重写成固定 7–10 键，人手工加的 `owner:`/`tags:` 一次动作就没。
    /// 这里按行透传，不解释语义——是"保真"，不是"支持自定义字段"。
    #[serde(skip, default)]
    pub extra: Vec<String>,
}

/// 协议固定的 frontmatter 键（`extra` 的反面即除此之外的顶层键）。
const KNOWN_FM_KEYS: &[&str] = &[
    "slug",
    "kind",
    "status",
    "project",
    "updated-by",
    "updated-at",
    "content-hash",
    "outcome",
    "falsification",
    "priority",
];

/// 摘出 frontmatter 中非固定键的顶层行，连带其缩进的子行/空行一起保留。
fn unknown_fm_lines(fm_str: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut keeping = false;
    for line in fm_str.lines() {
        let top_level = !line.starts_with([' ', '\t']) && !line.trim().is_empty();
        if top_level {
            let key = line.split(':').next().unwrap_or("").trim();
            keeping = !key.is_empty() && !KNOWN_FM_KEYS.contains(&key);
        }
        if keeping {
            out.push(line.to_string());
        }
    }
    out
}

/// 一个工作项 = 一个文件（§13.1）。frontmatter + Markdown 正文。
#[derive(Debug, Clone)]
pub struct Document {
    pub fm: Frontmatter,
    pub body: String,
}

/// 计算正文 content-hash（sha256 over body）。§13.2 第 2 层 CAS 的写前快照。
pub fn content_hash(body: &str) -> String {
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

impl Document {
    pub fn parse(path: &Path, raw: &str) -> Result<Document> {
        let (fm_str, body) = split_frontmatter(raw).ok_or_else(|| Error::Parse {
            path: path.display().to_string(),
            message: "缺少 YAML frontmatter 分隔线 `---`（§9.1）".into(),
        })?;
        let mut fm: Frontmatter = serde_yaml::from_str(fm_str).map_err(|e| Error::Parse {
            path: path.display().to_string(),
            message: format!("frontmatter 解析失败: {e}"),
        })?;
        fm.extra = unknown_fm_lines(fm_str);
        Ok(Document { fm, body })
    }

    /// 序列化为完整文件文本，并把 content-hash 更新为正文当前哈希。
    pub fn render(&mut self) -> String {
        self.fm.content_hash = content_hash(&self.body);
        let mut fm_yaml = serde_yaml::to_string(&self.fm)
            .unwrap_or_else(|_| "error".into())
            .trim_end()
            .to_string();
        // 非固定键原样回填（顺序保持在固定键之后）。
        if !self.fm.extra.is_empty() {
            fm_yaml.push('\n');
            fm_yaml.push_str(&self.fm.extra.join("\n"));
        }
        format!("---\n{fm_yaml}\n---\n{}", self.body)
    }

    pub fn read(path: &Path) -> Result<Document> {
        let raw = std::fs::read_to_string(path)?;
        Document::parse(path, &raw)
    }

    pub fn write(&mut self, path: &Path) -> Result<()> {
        let text = self.render();
        std::fs::write(path, text)?;
        Ok(())
    }

    /// 判断正文是否含某标题（规范化后前缀匹配，容忍 "## 2. 反证实验" / "### 反证实验（…）"）。
    pub fn has_heading(&self, heading: &str) -> bool {
        self.body.lines().any(|l| heading_matches(l, heading))
    }
}

/// 把一行 Markdown 标题规范化为可比较文本：去掉 `#`、去掉可选的编号前缀（"2. " / "2、" / "2 "）。
/// 非标题行返回 None。
pub fn heading_text(line: &str) -> Option<String> {
    if !line.starts_with('#') {
        return None;
    }
    let mut t = line.trim_start_matches('#').trim_start();
    // 去编号前缀：若干数字 + 可选分隔符。
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &t[digits.len()..];
        let trimmed = rest.trim_start_matches(['.', '、', ')', '）', ':', ' ', '-']);
        if trimmed.len() < rest.len() {
            t = trimmed.trim_start();
        }
    }
    Some(t.to_string())
}

/// 标题行是否以 needle 开头（规范化后）。
pub fn heading_matches(line: &str, needle: &str) -> bool {
    heading_text(line)
        .map(|h| h.starts_with(needle))
        .unwrap_or(false)
}

/// 返回某标题行的 `#` 级别。
pub fn heading_level(line: &str) -> usize {
    line.chars().take_while(|c| *c == '#').count()
}

/// 拆分 `---\n <fm> \n---\n <body>`。
fn split_frontmatter(raw: &str) -> Option<(&str, String)> {
    let rest = raw.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\n")
        .or_else(|| rest.strip_prefix("\r\n"))?;
    let end = find_fm_end(rest)?;
    let (fm, body) = rest.split_at(end.0);
    // 跳过结束的 `---` 行
    let body_after = body.split_once('\n').map(|x| x.1).unwrap_or("");
    Some((fm.trim_end(), body_after.to_string()))
}

/// 找到作为 frontmatter 结束的 `---` 行，返回其起始偏移与其后换行结束偏移。
fn find_fm_end(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut pos = 0usize;
    for line in s.lines() {
        let line_len = line.len() + 1; // +换行
        let trimmed = line.trim_end();
        if trimmed == "---" || trimmed == "..." {
            return Some((pos, pos + line_len));
        }
        pos += line_len;
        let _ = bytes;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_normalizes_numbered_prefix() {
        assert!(heading_matches(
            "## 2. 原理实机验证（门槛）",
            "原理实机验证"
        ));
        assert!(heading_matches("### 反证实验（必须）", "反证实验"));
        assert!(heading_matches("## 自审裁枝", "自审裁枝"));
        assert!(!heading_matches("## 目标", "原理实机验证"));
        assert!(!heading_matches("正文一句话", "正文"));
        assert_eq!(heading_level("### x"), 3);
    }

    #[test]
    fn frontmatter_roundtrip_and_hash() {
        let mut doc = Document {
            fm: Frontmatter {
                slug: "foo".into(),
                kind: Kind::Proposal,
                status: "pool".into(),
                project: "demo".into(),
                updated_by: "s1".into(),
                updated_at: "t".into(),
                content_hash: "sha256:pending".into(),
                outcome: None,
                falsification: None,
                priority: None,
                extra: Vec::new(),
            },
            body: "\n# foo\n\n## 反证实验\n".into(),
        };
        let text = doc.render();
        // content-hash 应被重算且稳定。
        let h = text
            .lines()
            .find(|l| l.starts_with("content-hash:"))
            .unwrap()
            .to_string();
        assert!(h.contains("sha256:") && !h.contains("pending"), "{h}");
        let back = Document::parse(Path::new("foo.md"), &text).unwrap();
        assert_eq!(back.fm.slug, "foo");
        assert_eq!(back.fm.kind, Kind::Proposal);
        assert_eq!(back.fm.content_hash, content_hash(&back.body));
    }

    #[test]
    fn parse_missing_frontmatter_errors() {
        let e = Document::parse(Path::new("x.md"), "# no frontmatter");
        assert!(matches!(e, Err(Error::Parse { .. })));
    }

    /// 回归（C18）：serde 不认的顶层键过去被整个 frontmatter 重写静默吞掉。
    #[test]
    fn unknown_frontmatter_keys_survive_a_roundtrip() {
        let raw = "---\nslug: foo\nkind: proposal\nstatus: pool\nproject: demo\nupdated-by: s1\nupdated-at: t\ncontent-hash: sha256:pending\nowner: 张三\ntags:\n  - audit\n  - urgent\n# 手工备注\n---\n\n# foo\n\n## 反证实验\n";
        let mut doc = Document::parse(Path::new("foo.md"), raw).unwrap();
        let first = doc.render();
        for want in ["owner: 张三", "tags:", "  - audit", "  - urgent", "# 手工备注"] {
            assert!(first.contains(want), "自定义键 {want} 必须原样落地：\n{first}");
        }
        // 再读再写：不重复堆叠、不丢失。
        let mut again = Document::parse(Path::new("foo.md"), &first).unwrap();
        let second = again.render();
        assert_eq!(
            second.matches("owner: 张三").count(),
            1,
            "重复渲染不应堆叠：\n{second}"
        );
        assert_eq!(second, first, "render 应幂等");
        assert_eq!(again.fm.slug, "foo", "固定键照常解析");
    }
}
