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

/// 按名取模板文本：`~/.Athena/templates/<name>`（④ overlay）存在则用用户的，否则用内置（§2.1 优先级）。
/// 改模板不需要改代码。
///
/// ④ 读不出来一律报错，**不再静默回退内置**（C38）：非法 UTF-8 的 overlay 从前会被当作
/// "没有 overlay"，用户手改的模板坏一个字节就悄悄换成出厂文本，且落地时看不出来。
pub fn load(store: &Store, name: &str) -> crate::error::Result<String> {
    use crate::error::Error;
    let overlay = store.templates_dir().join(name);
    if overlay.exists() {
        let bytes = std::fs::read(&overlay)?;
        return String::from_utf8(bytes).map_err(|_| Error::Template {
            message: format!(
                "{name}（{}）不是合法 UTF-8，拒绝回退到二进制内置内容。\n\
                 = 修好它，或删除该文件让 init 重铺出厂内容（删除会连带丢掉你的改动）。\n\
                 = 非法 UTF-8 从前会被当成\"没有 overlay\"而静默铺出内置文本，那是假保障。",
                overlay.display()
            ),
        });
    }
    Ok(BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| (*t).to_string())
        .unwrap_or_default())
}

/// ④ 是否存在（init 用它决定是否要如实告知"本次用的是二进制内置模板"）。
pub fn overlay_present(store: &Store, name: &str) -> bool {
    store.templates_dir().join(name).is_file()
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
