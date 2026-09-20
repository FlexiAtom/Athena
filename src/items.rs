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

use std::path::PathBuf;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::store::{Store, STATUSES};

/// 一个已解析的工作项 + 它在哪个状态目录（真值）。
pub struct Item {
    pub path: PathBuf,
    pub status: &'static str,
    pub doc: Document,
}

/// 列出某项目全部状态目录中的工作项。跳过解析失败项（由 validate 单独报告）。
pub fn list_items(store: &Store, project: &str) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    for status in STATUSES {
        let dir = store.status_dir(project, status);
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name == "pitfalls.md" || name == "meta.md" || name == "pending.md" {
                continue;
            }
            if let Ok(doc) = Document::read(&path) {
                out.push(Item { path, status, doc });
            }
        }
    }
    Ok(out)
}

/// 按 slug 查找工作项（slug 在项目内唯一，§5.2a）。返回找到的项；多个则报错消歧。
pub fn find_item(store: &Store, project: &str, slug: &str) -> Result<Option<Item>> {
    let mut hits = Vec::new();
    for status in STATUSES {
        let path = store.status_dir(project, status).join(format!("{slug}.md"));
        if path.exists() {
            let doc = Document::read(&path)?;
            hits.push(Item { path, status, doc });
        }
    }
    match hits.len() {
        0 => Ok(None),
        1 => Ok(Some(hits.pop().unwrap())),
        n => Err(Error::Slug {
            slug: slug.into(),
            message: format!("在 {n} 个状态目录中重复存在，违反项目内唯一性，请手动消歧"),
        }),
    }
}
