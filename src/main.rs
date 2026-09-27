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

use clap::{Parser, Subcommand, ValueEnum};
use store::Store;

/// `new --kind` 的取值白名单。此前是自由字符串：`"plan "`（带空格）静默收下，
/// frontmatter 写 `kind: "plan "` 而正文仍是提案骨架，clap 无从拦截。
#[derive(Clone, Copy, Debug, ValueEnum)]
enum KindArg {
    Proposal,
    Draft,
    Plan,
}

impl KindArg {
    fn as_str(self) -> &'static str {
        match self {
            KindArg::Proposal => "proposal",
            KindArg::Draft => "draft",
            KindArg::Plan => "plan",
        }
    }
}

/// Athena —— 面向 AI Agent 的工作协议（协议的执行边界）。
/// 目录即状态，文档即记忆；靠说服生效，不靠强制。
#[derive(Parser)]
#[command(name = "athena", version, about, long_about = None)]
struct Cli {
    /// 目标项目；缺省时用 config 的 default_project 或当前目录名。
    /// `notify`/`log`/`onerror`/`term` 整库或跨项目生效，没有项目作用域可选——同给会被**拒绝**而非静默忽略；
    /// `init` 只认位置参数，`--project <a> init <b>` 两个名字不同也拒。
    #[arg(long, global = true)]
    project: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 实例化 ~/.Athena 骨架 + 复制协议入口文件到项目根
    Init {
        /// 要新建/补骨架的项目名（区别于全局 `--project`：那个选的是已存在的项目）
        // 字段名即 clap 的参数 ID：早先叫 `project` 时与全局 `--project` 撞 ID，
        // 令顶层 `--project` 的值在 init 分支里恒为 None（"静默忽略"的真身）。
        #[arg(value_name = "PROJECT")]
        new_project: String,
        /// 入口文件名（默认 AGENTS.md）；换名不影响内容，只改落地文件名
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
    /// 用对应 kind 的模板在 pool/ 创建一项（同名在途项直接拒；与 finished 同名须 --reuse-finished）
    New {
        slug: String,
        /// 文档类型（同时决定正文骨架）：proposal 提案 | draft 草案 | plan 方案
        #[arg(long, value_enum, default_value = "proposal")]
        kind: KindArg,
        /// 同名项只在 finished/ 时才建（默认拒绝：同名两份会让该 slug 的状态动作全部 exit 1）
        #[arg(long)]
        reuse_finished: bool,
    },
    /// 轴一：原地深化文档类型（改 kind，文件不动）；单向不可降级，CLI 无回退命令
    Deepen {
        slug: String,
        /// 目标文档类型：proposal | draft | plan（只能沿此方向，不得降级）
        #[arg(long)]
        to: String,
    },
    /// 轴二：按当前位置推进一格（池→进行中 / community→working）
    Promote {
        slug: String,
        /// 无法实机时登记待测：需具象理由（不足 6 字会拒）
        #[arg(long = "skip-falsification")]
        skip: Option<String>,
    },
    /// 进行中→结束(done)：未清算的 pending 反证是硬拦截
    Complete { slug: String },
    /// 非 finished→结束(frozen)：毙掉须写原因；已是 finished 一律拒（改判先离开 finished/）
    Freeze {
        slug: String,
        /// 冻结原因，须点明冻结对象是设计/执行/结论（禁忌 9；CLI 只查非空）
        #[arg(long)]
        reason: String,
    },
    /// 放进 community/（请人帮忙处理）；起点不限，含 finished
    Community { slug: String },
    /// finished(done|frozen)→working：一律落 working，回不到 freeze 前的格子
    Resume { slug: String },
    /// 小修复/配置更改：一行留痕，不走完整剪枝（pool 项禁用；受 quick_limit 配额）
    Quick {
        slug: String,
        /// 这一行留痕的内容（原样进「决策日志」的 `- [ … ] quick:` 条目）
        #[arg(short = 'c', long)]
        comment: String,
        /// 留痕之后顺手再推一格；**非原子**——留痕已落盘并提交，推格失败配额照样耗掉
        #[arg(long)]
        promote: bool,
    },
    /// 写入（相对 ~/.Athena；裸状态目录路径 pool|working|finished|community 依 --project 归位到 projects/<project>/…，归位后按 item doc 校验）
    Write {
        path: String,
        /// 内容（与 --stdin 二选一）
        #[arg(short, long)]
        content: Option<String>,
        /// 从标准输入读内容（内容含 `$` 或反引号时用这条，别让 shell 展开）
        #[arg(long)]
        stdin: bool,
        /// 允许写入空/纯空白内容（默认拒绝，防误清空状态文件）
        #[arg(long)]
        allow_empty: bool,
    },
    /// 追加（相对 ~/.Athena；裸状态目录路径 pool|working|finished|community 依 --project 归位到 projects/<project>/…，归位后按 item doc 校验）
    Append {
        path: String,
        /// 内容（与 --stdin 二选一）；自动补行尾换行，多 agent 追加走 O_APPEND 不互相覆盖
        #[arg(short, long)]
        content: Option<String>,
        /// 从标准输入读内容
        #[arg(long)]
        stdin: bool,
        /// 允许追加空/纯空白内容（默认与 write 一样拒绝）
        #[arg(long)]
        allow_empty: bool,
    },
    /// 记一条坑（或加 --search 只读跨源搜索坑）
    Pitfall {
        /// 坑的正文（记为一条 `- …  <!-- 日期 · actor -->`）；以 `-` 开头时用 `-- "<文本>"`
        text: Option<String>,
        /// 落全局 pitfalls/global.md，而非当前项目的 pitfalls.md
        #[arg(long)]
        global: bool,
        /// 只读搜索：跨全局 + 所有项目的 pitfalls.md 匹配词条（大小写不敏感、多词 AND），不记录、不改状态
        #[arg(long)]
        search: Option<String>,
    },
    /// 广播一条全局通知（写入 ~/.Athena/notices.md，各项目 context/validate 顶部可见）
    Notify {
        /// 通知文本（与 --clear 二选一，同给会被拒）；多行正文会折成一条 + 缩进续行一起渲染；
        /// 以 `-` 开头时用 `athena notify -- "<文本>"`
        text: Option<String>,
        /// 清空全局通知广播板（跨项目整块，不只清本项目）
        #[arg(long)]
        clear: bool,
    },
    /// 输出 AI 上下文（协议摘要 + 目录树 + 全局通知 + 人话台账 + 术语 + 两级被坑）
    Context,
    /// 自检报告（解析校验 + 剪枝/反证完整性；只报告不改文件）。退出码 0=无 Error 级发现（可含任意 ⚠）｜1=命令自身失败｜2=报告含 Error 级发现
    Validate,
    /// 整库（不按 --project 过滤）的 git 提交主题流水：多为协议动作史，不是项目代码史
    Log {
        /// 打印最近多少条提交主题（默认 20）
        #[arg(short = 'n', default_value = "20")]
        count: usize,
    },
    /// AI 故障处置手册（源码位置 + /tmp 报告 + 处置原则）
    Onerror,
    /// 术语接口（无 show：单条定义去 terms.md 或 context 看）
    Term {
        #[command(subcommand)]
        sub: TermCmd,
    },
}

#[derive(Subcommand)]
enum TermCmd {
    /// 列出所有术语（首行含 quick_limit 与生效的反证模式）
    List,
    /// 新增术语骨架（只追加一条 [term.<slug>]，不动既有条目）
    New {
        slug: String,
        /// 术语来源（如某次真实踩坑或某份文档），写进骨架的 origin 字段
        #[arg(long)]
        origin: Option<String>,
    },
    /// 校验 terms.local.toml：语法 + 未知键/已废弃键/表名与 slug 不符等语义点名（不查定义是否说清）
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
        // `--project` 在整库/跨项目命令上没有作用对象：从前静默忽略，于是同一个参数
        // 在两族命令里语义相反（一条严格校验、一条无声丢弃），调用方无从知道哪种（C11）。
        if let Some(flag) = cli.project.as_deref().filter(|s| !s.trim().is_empty()) {
            if matches!(
                &cli.cmd,
                Cmd::Notify { .. } | Cmd::Log { .. } | Cmd::Onerror | Cmd::Term { .. }
            ) {
                anyhow::bail!(
                    "`--project {flag}` 对该命令无效：`notify`/`log`/`onerror`/`term` 整库或跨项目生效，没有项目作用域可选。\n\
                     = 去掉 --project 再跑（这是拒绝、不是警告：静默忽略会让人以为作用域已经收窄）。"
                );
            }
        }
        match &cli.cmd {
            Cmd::Init {
                new_project,
                agents_file,
                at,
                force,
                no_agents,
            } => {
                // `--project` 选的是"已存在的项目"，`init` 的位置参数是"要新建的项目"。
                // 两者同时出现却不同名时静默按位置参数走，会让调用方以为自己在给
                // 前者补骨架（实测另建出一个新项目）。这里拒绝，不做任何一方的解释。
                if let Some(flag) = cli.project.as_deref().filter(|s| !s.trim().is_empty()) {
                    if flag != new_project.as_str() {
                        anyhow::bail!(
                            "`init {new_project}` 与 `--project {flag}` 不一致：init 的项目名只认位置参数，\n\
                             --project 用于指向**已存在**的项目、对 init 无效。想给 {flag} 补骨架请去掉 --project。"
                        );
                    }
                }
                let at = at.clone().unwrap_or_else(|| PathBuf::from("."));
                commands::init(&store, new_project, agents_file, &at, *force, *no_agents)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::New {
                slug,
                kind,
                reuse_finished,
            } => {
                let p = resolve(cli.project.as_deref());
                commands::new_item(&store, &p, slug, kind.as_str(), *reuse_finished)
                    .map_err(anyhow::Error::from)?;
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
                let p = resolve(cli.project.as_deref());
                commands::write_path(&store, &p, path, &c, *allow_empty)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::Append {
                path,
                content,
                stdin,
                allow_empty,
            } => {
                let c = read_content(content.clone(), *stdin)?;
                let p = resolve(cli.project.as_deref());
                commands::append_path(&store, &p, path, &c, *allow_empty)
                    .map_err(anyhow::Error::from)?;
            }
            Cmd::Pitfall {
                text,
                global,
                search,
            } => {
                let p = resolve(cli.project.as_deref());
                if let Some(q) = search {
                    // 搜索本就跨全局 + 全部项目，--global 在这条路径上没有作用对象；
                    // 对 <文本> 已有同类提示，这里补齐，免得留下"半静音"的第三种组合（C9）。
                    if *global {
                        eprintln!("· 已同时给出 --global 与 --search：搜索已覆盖全局与所有项目，--global 不生效。");
                    }
                    if text.is_some() {
                        eprintln!("· 已同时给出 <文本> 与 --search：仅执行只读搜索，<文本> 被忽略、不记录。");
                    }
                    commands::pitfall_search(&store, &p, q).map_err(anyhow::Error::from)?;
                } else {
                    let text = text.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("pitfall 需要一个 <文本>，或改用 --search \"<词条>\"")
                    })?;
                    commands::pitfall(&store, &p, text, *global).map_err(anyhow::Error::from)?;
                }
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
