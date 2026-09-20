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

use crate::store::Store;

/// 内置模板（编译进二进制，零外部依赖，§2.1）。
const BUILTIN: &[(&str, &str)] = &[
    (
        "proposal.md.tpl",
        include_str!("../templates/proposal.md.tpl"),
    ),
    ("draft.md.tpl", include_str!("../templates/draft.md.tpl")),
    ("plan.md.tpl", include_str!("../templates/plan.md.tpl")),
    (
        "falsification.md.tpl",
        include_str!("../templates/falsification.md.tpl"),
    ),
    ("AGENTS.md.tpl", include_str!("../templates/AGENTS.md.tpl")),
    ("terms.md.tpl", include_str!("../templates/terms.md.tpl")),
    (
        "terms.local.toml.tpl",
        include_str!("../templates/terms.local.toml.tpl"),
    ),
    (
        "config.toml.tpl",
        include_str!("../templates/config.toml.tpl"),
    ),
];

/// 按名取模板文本：`~/.Athena/templates/<name>` 存在则用用户的，否则用内置（§2.1 优先级）。
/// 改模板不需要改代码。
pub fn load(store: &Store, name: &str) -> String {
    let overlay = store.templates_dir().join(name);
    if let Ok(s) = std::fs::read_to_string(&overlay) {
        return s;
    }
    BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| (*t).to_string())
        .unwrap_or_default()
}

/// 把 overlay 之外的内置模板铺到 ~/.Athena/templates/（init 时）。已存在则不覆盖。
pub fn materialize_defaults(store: &Store) -> std::io::Result<()> {
    std::fs::create_dir_all(store.templates_dir())?;
    for (name, text) in BUILTIN {
        let p = store.templates_dir().join(name);
        if !p.exists() {
            std::fs::write(&p, text)?;
        }
    }
    Ok(())
}

/// 极简占位符渲染：替换 `{{key}}`。够用即可，不引重量级模板引擎（§6b「自研 mini」）。
pub fn render(text: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        if let Some(j) = rest[i..].find("}}") {
            let key = rest[i + 2..i + j].trim();
            match vars.get(key) {
                Some(v) => out.push_str(v),
                None => {
                    out.push_str("{{");
                    out.push_str(key);
                    out.push_str("}}");
                }
            }
            rest = &rest[i + j + 2..];
        } else {
            out.push_str("{{");
            rest = &rest[i + 2..];
        }
    }
    out.push_str(rest);
    out
}

/// kind → 模板文件名。
pub fn template_for_kind(kind: &str) -> &'static str {
    match kind {
        "draft" => "draft.md.tpl",
        "plan" => "plan.md.tpl",
        _ => "proposal.md.tpl",
    }
}

pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 供测试：确保至少内置模板可被解析路径覆盖机制命中。
#[allow(dead_code)]
pub fn exists_any(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}
