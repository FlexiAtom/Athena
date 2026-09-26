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

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// 四个推进状态目录（轴二 · 位置即状态，§1）。
pub const STATUSES: [&str; 4] = ["pool", "working", "finished", "community"];

/// Store：对 `~/.Athena` 状态库的引用 + 路径解析 + 越界检查（§6b, §2.3）。
///
/// 路径防呆由 `resolve_rel` 实现：所有写入相对 `~/.Athena` 解析，拒绝绝对路径、
/// `..`、符号链接穿透——这是"帮 AI 别写错地方"的防呆，不是权限网关（§1.3）。
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// 状态根：`$ATHENA_HOME` 优先（供测试），否则 `~/.Athena`。
    pub fn open() -> Result<Self> {
        let root = match std::env::var_os("ATHENA_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => {
                let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
                    Error::Transition {
                        message: "无法确定 HOME（且未设置 ATHENA_HOME）".into(),
                    }
                })?;
                home.join(".Athena")
            }
        };
        let root = absolutize(&root);
        Ok(Store { root })
    }

    pub fn exists(&self) -> bool {
        self.root.join(".git").exists() || self.root.join("projects").exists()
    }

    /// 将相对 ~/.Athena 的路径解析为绝对路径，并做越界检查（防呆）。
    ///
    /// 拒绝：绝对路径、含 `..`/根前缀的路径。返回 canonicalize 后确认在 root 内的路径。
    /// 文件可能尚未创建，故对"最近的已存在祖先"做 canonicalize。
    pub fn resolve_rel(&self, rel: &str) -> Result<PathBuf> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(Error::PathEscape { path: rel.into() });
        }
        // 先做词法拒绝，拦截显式 `..` 与根。
        for comp in rel_path.components() {
            match comp {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(Error::PathEscape { path: rel.into() })
                }
                _ => {}
            }
        }
        let joined = self.root.join(rel_path);
        let canonical = self
            .canonicalize_existing_prefix(&joined)
            .map_err(|_| Error::PathEscape { path: rel.into() })?;
        let root_canon = self.canonicalize_existing_prefix(&self.root)?;
        if !canonical.starts_with(&root_canon) {
            return Err(Error::PathEscape { path: rel.into() });
        }
        Ok(canonical)
    }

    /// canonicalize 一个可能不存在的路径：向上找到最近的已存在祖先做 canonicalize，再拼回剩余段。
    fn canonicalize_existing_prefix(&self, p: &Path) -> Result<PathBuf> {
        let mut suffix: Vec<std::path::Component> = Vec::new();
        let mut cur = p;
        loop {
            if let Ok(c) = cur.canonicalize() {
                let mut out = c;
                for s in suffix.iter().rev() {
                    out.push(s.as_os_str());
                }
                return Ok(out);
            }
            match (cur.parent(), cur.file_name()) {
                (Some(parent), Some(name)) => {
                    suffix.push(Component::Normal(name));
                    cur = parent;
                }
                _ => {
                    return Err(Error::PathEscape {
                        path: p.display().to_string(),
                    })
                }
            }
        }
    }

    // ---- 常用目录助手 ----

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }
    pub fn project_dir(&self, project: &str) -> PathBuf {
        self.projects_dir().join(project)
    }
    pub fn status_dir(&self, project: &str, status: &str) -> PathBuf {
        self.project_dir(project).join(status)
    }
    pub fn templates_dir(&self) -> PathBuf {
        self.root.join("templates")
    }
    pub fn terms_toml(&self) -> PathBuf {
        self.root.join("terms.local.toml")
    }
    pub fn terms_md(&self) -> PathBuf {
        self.root.join("terms.md")
    }
    pub fn global_pitfalls(&self) -> PathBuf {
        self.root.join("pitfalls/global.md")
    }
    pub fn project_pitfalls(&self, project: &str) -> PathBuf {
        self.project_dir(project).join("pitfalls.md")
    }
    /// 人话台账：只存人的原话逐字，指令来源的最高优先级证据（见 §2.4）。
    pub fn project_voice(&self, project: &str) -> PathBuf {
        self.project_dir(project).join("voice.md")
    }
    pub fn global_voice(&self) -> PathBuf {
        self.root.join("voice.md")
    }
    pub fn config_toml(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    /// 全局通知广播板（单一真源，跨项目共享；见 `athena notify` / §13.6）。
    pub fn notices_md(&self) -> PathBuf {
        self.root.join("notices.md")
    }
}

/// 若给定路径已是绝对则原样 canonicalize-lite（拼接 cwd），否则尽力规范化。
fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("athena-store-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("projects/demo/pool")).unwrap();
        dir
    }

    #[test]
    fn resolve_rejects_escape_and_absolute() {
        let root = tmp_root();
        let store = Store { root: root.clone() };
        assert!(store.resolve_rel("../evil.md").is_err());
        assert!(store.resolve_rel("/etc/passwd").is_err());
        assert!(store.resolve_rel("projects/demo/pool/ok.md").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}
