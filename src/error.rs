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

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// 路径解析越出 ~/.Athena 之外（防呆，§2.3）。
    PathEscape {
        path: String,
    },
    /// YAML frontmatter 解析失败（§9.1：报错交编辑者修，绝不自动改）。
    Parse {
        path: String,
        message: String,
    },
    Toml {
        path: String,
        message: String,
    },
    /// slug 未找到或在多个状态目录中重复。
    Slug {
        slug: String,
        message: String,
    },
    /// 转换非法或前置未满足。
    Transition {
        message: String,
    },
    /// 目标文件已存在且内容不同，需显式 --force/--no-agents 才继续（防呆，§1.1 安装语义）。
    Conflict {
        message: String,
    },
    Git {
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO 错误: {e}"),
            Error::PathEscape { path } => write!(
                f,
                "路径越界: `{path}` 解析到 ~/.Athena 之外。\n\
                 = 状态内容不得离开 ~/.Athena（§1.1 位置红线）。请改用相对 ~/.Athena 的路径。"
            ),
            Error::Parse { path, message } => write!(
                f,
                "解析 {path} 失败\n= 原因: {message}\n\
                 = 这是人工/AI 直接编辑导致的解析错误，Athena 不会自动修改你的文件（§9.1）。\n\
                 = 请自行修复后重试。"
            ),
            Error::Toml { path, message } => {
                write!(f, "TOML {path} 解析失败\n= 原因: {message}")
            }
            Error::Slug { slug, message } => write!(f, "工作项 `{slug}`: {message}"),
            Error::Transition { message } => write!(f, "非法转换: {message}"),
            Error::Conflict { message } => write!(f, "拒绝覆盖: {message}"),
            Error::Git { message } => write!(f, "git 错误: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
