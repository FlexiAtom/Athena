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

// v1 有意保留的架构接缝（Rule::id / Store::exists / Term 全字段 / registry getter），
// 供 §6b 可插拔与 §2.1b 术语驱动校验后续接入，不作为死代码删除。
#![allow(dead_code)]

mod commands;
mod config;
mod context;
mod document;
mod error;
mod items;
mod rules;
mod store;
mod templates;
mod term;
mod vcs;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use store::Store;

/// Athena —— 面向 AI Agent 的工作协议（协议的执行边界）。
/// 目录即状态，文档即记忆；靠说服生效，不靠强制。
#[derive(Parser)]
#[command(name = "athena", version, about, long_about = None)]
struct Cli {
    /// 目标项目；缺省时用 config 的 default_project 或当前目录名
    #[arg(long, global = true)]
    project: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 实例化 ~/.Athena 骨架 + 复制协议入口文件到项目根
    Init {
        project: String,
        #[arg(long, default_value = "AGENTS.md")]
        agents_file: String,
        /// 入口文件落地目录（默认为当前目录）
        #[arg(long)]
        at: Option<PathBuf>,
        /// 覆盖已存在的入口文件前必须显式指定（防呆，§1.1）
        #[arg(long)]
        force: bool,
        /// 只初始化状态骨架，完全不触碰目标仓库的入口文件
        #[arg(long)]
        no_agents: bool,
    },
    /// 用提案模板在 pool/ 创建（项目内唯一）
    New {
        slug: String,
        #[arg(long, default_value = "proposal")]
        kind: String,
    },
    /// 轴一：原地深化文档类型（改 kind，文件不动）
    Deepen {
        slug: String,
        #[arg(long)]
        to: String,
    },
    /// 轴二：按当前位置推进一格（池→进行中 / community→working）
    Promote {
        slug: String,
        /// 无法实机时登记待测：需具象理由
        #[arg(long = "skip-falsification")]
        skip: Option<String>,
    },
    /// 进行中→结束(done)：pending 反证须先清算
    Complete { slug: String },
    /// 任意→结束(frozen)：毙掉需写原因
    Freeze {
        slug: String,
        #[arg(long)]
        reason: String,
    },
    /// 放进 community/（请人帮忙处理）
    Community { slug: String },
    /// finished(frozen)→working（要求重新剪枝）
    Resume { slug: String },
    /// 小修复/配置更改：一行留痕，不走完整剪枝
    Quick {
        slug: String,
        #[arg(short = 'c', long)]
        comment: String,
        #[arg(long)]
        promote: bool,
    },
    /// 写入（相对 ~/.Athena）
    Write {
        path: String,
        #[arg(short, long)]
        content: Option<String>,
        #[arg(long)]
        stdin: bool,
        /// 允许写入空/纯空白内容（默认拒绝，防误清空状态文件）
        #[arg(long)]
        allow_empty: bool,
    },
    /// 追加（相对 ~/.Athena）
    Append {
        path: String,
        #[arg(short, long)]
        content: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
    /// 记一条坑
    Pitfall {
        text: String,
        #[arg(long)]
        global: bool,
    },
    /// 广播一条全局通知（写入 ~/.Athena/notices.md，各项目 context/validate 顶部可见）
    Notify {
        /// 通知文本（与 --clear 二选一）
        text: Option<String>,
        /// 清空全局通知广播板
        #[arg(long)]
        clear: bool,
    },
    /// 输出 AI 上下文（协议摘要 + 目录树 + 术语 + 坑）
    Context,
    /// 自检报告（解析校验 + 剪枝/反证完整性；只报告不改文件）
    Validate,
    /// git 历史
    Log {
        #[arg(short = 'n', default_value = "20")]
        count: usize,
    },
    /// AI 故障处置手册（源码位置 + /tmp 报告 + 处置原则）
    Onerror,
    /// 术语接口
    Term {
        #[command(subcommand)]
        sub: TermCmd,
    },
}

#[derive(Subcommand)]
enum TermCmd {
    /// 列出所有术语
    List,
    /// 新增术语骨架
    New {
        slug: String,
        #[arg(long)]
        origin: Option<String>,
    },
    /// 校验 terms.local.toml 语法
    Validate,
}

fn read_content(content: Option<String>, stdin: bool) -> anyhow::Result<String> {
    if let Some(c) = content {
        return Ok(c);
    }
    if stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    anyhow::bail!("需要 -c \"内容\" 或 --stdin")
}

fn main() {
    let cli = Cli::parse();
    let store = match Store::open() {
        Ok(s) => s,
        Err(e) => exit_err(e),
    };
    let resolve = |flag: Option<&str>| match commands::project_name(&store, flag) {
        Ok(p) => p,
        Err(e) => exit_err(e),
    };

    let r: anyhow::Result<()> = (|| {
        match &cli.cmd {
            Cmd::Init {
                project,
                agents_file,
                at,
                force,
                no_agents,
            } => {
                let at = at.clone().unwrap_or_else(|| PathBuf::from("."));
                commands::init(&store, project, agents_file, &at, *force, *no_agents)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::New { slug, kind } => {
                let p = resolve(cli.project.as_deref());
                commands::new_item(&store, &p, slug, kind).map_err(anyhow::Error::from)?;
            }
            Cmd::Deepen { slug, to } => {
                let p = resolve(cli.project.as_deref());
                commands::deepen(&store, &p, slug, to).map_err(anyhow::Error::from)?;
            }
            Cmd::Promote { slug, skip } => {
                let p = resolve(cli.project.as_deref());
                commands::promote(&store, &p, slug, skip.as_deref())
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::Complete { slug } => {
                let p = resolve(cli.project.as_deref());
                commands::complete(&store, &p, slug).map_err(anyhow::Error::from)?;
            }
            Cmd::Freeze { slug, reason } => {
                let p = resolve(cli.project.as_deref());
                commands::freeze(&store, &p, slug, reason).map_err(anyhow::Error::from)?;
            }
            Cmd::Community { slug } => {
                let p = resolve(cli.project.as_deref());
                commands::community(&store, &p, slug).map_err(anyhow::Error::from)?;
            }
            Cmd::Resume { slug } => {
                let p = resolve(cli.project.as_deref());
                commands::resume(&store, &p, slug).map_err(anyhow::Error::from)?;
            }
            Cmd::Quick {
                slug,
                comment,
                promote,
            } => {
                let p = resolve(cli.project.as_deref());
                commands::quick(&store, &p, slug, comment, *promote)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::Write {
                path,
                content,
                stdin,
                allow_empty,
            } => {
                let c = read_content(content.clone(), *stdin)?;
                commands::write_path(&store, path, &c, *allow_empty)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::Append {
                path,
                content,
                stdin,
            } => {
                let c = read_content(content.clone(), *stdin)?;
                commands::append_path(&store, path, &c).map_err(anyhow::Error::from)?;
            }
            Cmd::Pitfall { text, global } => {
                let p = resolve(cli.project.as_deref());
                commands::pitfall(&store, &p, text, *global).map_err(anyhow::Error::from)?;
            }
            Cmd::Notify { text, clear } => {
                commands::notify(&store, text.as_deref(), *clear).map_err(anyhow::Error::from)?;
            }
            Cmd::Context => {
                let p = resolve(cli.project.as_deref());
                commands::show_context(&store, &p).map_err(anyhow::Error::from)?;
            }
            Cmd::Validate => {
                let p = resolve(cli.project.as_deref());
                let ok = commands::validate(&store, &p).map_err(anyhow::Error::from)?;
                if !ok {
                    eprintln!("\n存在 Error 级问题（block 模式或结构错误）。");
                    std::process::exit(2);
                }
            }
            Cmd::Log { count } => {
                commands::show_log(&store, *count).map_err(anyhow::Error::from)?;
            }
            Cmd::Onerror => {
                commands::onerror(&store).map_err(anyhow::Error::from)?;
            }
            Cmd::Term { sub } => match sub {
                TermCmd::List => commands::term_list(&store).map_err(anyhow::Error::from)?,
                TermCmd::New { slug, origin } => {
                    commands::term_new(&store, slug, origin.as_deref())
                        .map_err(anyhow::Error::from)?
                }
                TermCmd::Validate => {
                    commands::term_validate(&store).map_err(anyhow::Error::from)?
                }
            },
        }
        Ok(())
    })();

    if let Err(e) = r {
        if let Some(crate_err) = e.downcast_ref::<error::Error>() {
            eprintln!("{crate_err}");
        } else {
            eprintln!("错误: {e}");
        }
        std::process::exit(1);
    }
}

fn exit_err(e: error::Error) -> ! {
    eprintln!("{e}");
    std::process::exit(1);
}
