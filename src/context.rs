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

use crate::error::Result;
use crate::items::list_items;
use crate::store::{Store, STATUSES};
use crate::term::TermsRegistry;

/// 协议摘要（§7 首段，必读，不依赖入口文件）。
const SUMMARY: &str = "\
# Athena Context
# 你是 Athena 工作协议的执行者。
# 两条正交的轴，别搞混：
#   ① 文档类型（它是什么）：提案=有个想法 / 草案=打草稿怎么实现 / 方案=可照着实施
#   ② 推进状态（推到哪一步）：池 / 进行中 / 结束 / 社区互助 —— 由目录表达
# 写深了 → deepen（改内容，文件不动）；推进一步 → promote（挪目录，剪枝是门票）。
# 禁忌：① 不进池就写代码 ② 不剪枝就想 promote ③ 说\"可行\"须贴真实输出
#      ④ 发现更优方案须做成本对账 ⑤ 不在 ~/.Athena 外写状态文件。
# 小改动走快速通道（athena quick，一行留痕）；大改动走完整剪枝。
# 违反协议 → validate 标红提示（自检报告，不是拦截）。";

/// 渲染 `athena context` 输出（§7）。
pub fn build(store: &Store, project: &str, terms: &TermsRegistry) -> Result<String> {
    let mut s = String::new();
    let now = crate::templates::now_iso();
    s.push_str(SUMMARY);
    s.push('\n');
    s.push_str(&format!("# Project: {project} | Generated: {now}\n"));
    s.push_str(&format!(
        "# Truth: ~/.Athena/projects/{project} (git-tracked)\n"
    ));

    // 全局通知广播板（单一信道 notices.md，一次写入全项目可见；§13.6 不预支复杂度）。
    if let Some(section) = notices_section(store) {
        s.push_str(&section);
    }

    let items = list_items(store, project)?;
    for status in STATUSES {
        let title = match status {
            "pool" => "池（pool/）",
            "working" => "进行中（working/）",
            "finished" => "结束（finished/）",
            _ => "社区互助（community/）",
        };
        let here: Vec<_> = items.iter().filter(|i| i.status == status).collect();
        s.push_str(&format!("\n## {title}\n"));
        if here.is_empty() {
            s.push_str("- （空）\n");
            continue;
        }
        for it in here {
            let outcome = match it.doc.fm.outcome {
                Some(crate::document::Outcome::Done) => ", outcome: done".to_string(),
                Some(crate::document::Outcome::Frozen) => ", outcome: frozen".to_string(),
                None => String::new(),
            };
            let prio = it
                .doc
                .fm
                .priority
                .as_ref()
                .map(|p| format!(", {p}"))
                .unwrap_or_default();
            let goal = first_goal_line(&it.doc.body);
            s.push_str(&format!(
                "- {} · kind: {}{}{prio}\n",
                it.doc.fm.slug,
                it.doc.fm.kind.as_str(),
                outcome,
            ));
            if let Some(g) = goal {
                s.push_str(&format!("    目标：{g}\n"));
            }
        }
    }

    // pending 汇总（§5.1f：complete 前须清算，默认展示）。
    let pend = store.project_dir(project).join("pending.md");
    if let Ok(t) = std::fs::read_to_string(&pend) {
        if t.contains("- [ ]") {
            s.push_str("\n## ⚠ 待清算的反证（pending）\n");
            for line in t.lines() {
                if line.contains("- [ ]") {
                    s.push_str(&format!("{line}\n"));
                }
            }
        }
    }

    // 两级被坑。
    push_file_section(&mut s, "## 项目级被坑", &store.project_pitfalls(project));
    push_file_section(&mut s, "## 全局被坑", &store.global_pitfalls());

    // 术语与规则。
    s.push_str("\n## 术语与规则（来自 terms.md）\n");
    if let Ok(t) = std::fs::read_to_string(store.terms_md()) {
        s.push_str(&t);
        if !t.ends_with('\n') {
            s.push('\n');
        }
    }
    let _ = terms;
    Ok(s)
}

fn first_goal_line(body: &str) -> Option<String> {
    let mut seen_goal = false;
    for l in body.lines() {
        let t = l.trim();
        if t.starts_with("## 目标") {
            seen_goal = true;
            continue;
        }
        if seen_goal {
            if t.starts_with('#') {
                break;
            }
            if !t.is_empty() && !t.starts_with("<!--") {
                return Some(t.chars().take(80).collect());
            }
        }
    }
    None
}

fn push_file_section(s: &mut String, header: &str, path: &std::path::Path) {
    if let Ok(t) = std::fs::read_to_string(path) {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            s.push_str(&format!("\n{header}\n"));
            s.push_str(trimmed);
            s.push('\n');
        }
    }
}

/// 从 `~/.Athena/notices.md` 抽取广播通知（以 `- ` 开头的行），渲染成顶部提醒段。
/// 无内容（文件缺失或无通知行）→ None。通知是跨项目广播而非工作项，不进 list_items/validate 判定。
pub fn notices_section(store: &Store) -> Option<String> {
    let text = std::fs::read_to_string(store.notices_md()).ok()?;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("- ") && l.chars().count() > 2)
        .collect();
    if lines.is_empty() {
        return None;
    }
    let mut s = String::from("\n## 📣 全局通知（来自 ~/.Athena/notices.md · 处理后请清理）\n");
    for l in lines {
        s.push_str(l);
        s.push('\n');
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("athena-notify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn notices_absent_or_header_only_yields_none() {
        let root = scratch_root("none");
        let store = Store { root: root.clone() };
        // 文件缺失
        assert!(notices_section(&store).is_none());
        // 仅有表头/注释、无通知行
        std::fs::write(store.notices_md(), "# 全局通知\n<!-- 注释 -->\n").unwrap();
        assert!(notices_section(&store).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn notices_renders_only_bullet_lines() {
        let root = scratch_root("some");
        let store = Store { root: root.clone() };
        std::fs::write(
            store.notices_md(),
            "# 全局通知\n- 2026-09-21 · AGENTS.md 已更新，请同步入口副本\n\n正文不应出现\n",
        )
        .unwrap();
        let section = notices_section(&store).expect("应有通知");
        assert!(section.contains("AGENTS.md 已更新"));
        assert!(!section.contains("正文不应出现"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
