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
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::context;
use crate::document::{content_hash, Document, Frontmatter, Kind, Outcome};
use crate::error::{Error, Result};
use crate::items::{find_item, is_item_doc_rel, list_items, list_unparseable, Item};
use crate::rules::{self, Finding, Level, ValidationCtx};
use crate::store::{Store, STATUSES};
use crate::templates;
use crate::term::TermsRegistry;
use crate::vcs::Git;

const PROTO_VERSION: &str = "0.1.0";

/// 全局通知广播板的表头（首次 notify 或 init 时落地）。
const NOTICE_HEADER: &str = "# Athena · 全局通知（广播板）\n\n\
<!-- 单一全局信道：一次写入、各项目 `athena context`/`validate` 顶部可见。\n\
     非工单系统——不追踪逐项目已读（§3 否决、§13.6 不预支复杂度）。\n\
     一条一项：`- <时间> · <提醒>`，正文多行时后续行缩进两格作为该条续行（一起渲染）。\n\
     处理完用 `athena notify --clear` 或手动删行清理。 -->\n\n";

// ============================================================================
// 项目 / actor 解析
// ============================================================================

fn resolve_project(store: &Store, flag: Option<&str>) -> Result<String> {
    // 空串/纯空白等同未设：否则 `--project ""` 的存在性检查会被 `projects/` 顶层
    // 目录空洞满足，写动作随即落进 `projects/pool/…` 造出幻影树。
    let flag = flag.filter(|s| !s.trim().is_empty());
    let name = if let Some(p) = flag {
        p.trim().to_string()
    } else {
        let cfg = Config::load(store)?;
        match cfg.default_project.filter(|s| !s.trim().is_empty()) {
            Some(dp) => dp.trim().to_string(),
            // 回退到当前目录名（常见：在项目仓库里执行）。
            None => {
                let cwd = std::env::current_dir()?;
                cwd.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| Error::Transition {
                        message: "无法确定项目：请用 --project 指定，或先 `athena init`".into(),
                    })?
            }
        }
    };
    validate_project_name(&name)?;
    // 解析出的项目必须已存在（项目目录只由 `athena init` 创建）。否则从 `~` 等
    // 非项目根运行时会把 cwd 名当项目，静默造出"幽灵项目"：context 显示全空却标
    // (git-tracked)（假阴性），new 会凭空建 projects/<假名>/ 污染状态树。显式 --project
    // 指向不存在项目同样拦下（多半是拼写错或漏 init）。
    if !store.project_dir(&name).is_dir() {
        return Err(Error::Transition {
            message: format!(
                "项目 `{name}` 不存在（缺 ~/.Athena/projects/{name}）。\n\
                 多半是在非项目目录运行：请在项目仓库根执行、或用 --project 指定；新项目先 `athena init {name}`。"
            ),
        });
    }
    Ok(name)
}

/// 项目名即 `projects/<name>` 目录名，参与每一次状态读写定位。
/// 含路径分隔符或 `..` 的名字会把整个项目的读写面指到别处（甚至状态根之外），
/// 所以三条解析路径（`--project` / config / cwd 回落）共用这道词法校验。
fn validate_project_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        return Ok(());
    }
    Err(Error::BadPath {
        what: "项目名".into(),
        value: name.into(),
        message: "只允许字母、数字与 . _ -，且不得以 `.` 开头。\n\
                  = 若这是当前目录名（cwd 回落），请用 `--project <名>` 显式指定，或改 config 的 default_project。"
            .into(),
    })
}

/// slug 即 `<status>/<slug>.md` 的文件名。`new` 是唯一"凭参数造文件"的入口，
/// 所以词法校验必须走在写盘之前：含 `/` 会落进子目录并对 context/validate
/// 双双隐身（目录扫描只看一层），含 `..` 则直接把状态文件写到状态根之外。
fn validate_slug(slug: &str) -> Result<()> {
    let ok = !slug.is_empty()
        && !slug.starts_with('.')
        && !slug.starts_with('-')
        && !slug.ends_with(".md")
        && slug
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        return Ok(());
    }
    Err(Error::BadPath {
        what: "slug".into(),
        value: slug.into(),
        message: "只允许字母、数字与 . _ -；不得以 `.`/`-` 开头、不得含 `/`、`..`、空白，也不得以 `.md` 结尾。\n\
                  = slug 直接成为状态文件名，越界或落进子目录的项对 context/validate 永久隐身。"
            .into(),
    })
}

fn actor() -> String {
    // session-id 语义（§13.2 第 3 层）：取环境变量，否则进程号占位。
    std::env::var("ATHENA_SESSION")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("pid-{}", std::process::id()))
}

fn require_initialized(store: &Store) -> Result<()> {
    if !store.root.join("projects").exists() {
        return Err(Error::Transition {
            message: format!(
                "状态库尚未初始化（{} 不存在）。先运行 `athena init <project>`。",
                store.root.display()
            ),
        });
    }
    // 审计层属于"已初始化"的一部分：`.git` 一旦丢失，写动作会寄生进宿主仓库、
    // `log` 会打印别人的历史，而命令照样 rc=0（C22）。放在任何写盘之前拒。
    Git::require_repo(&store.root)?;
    Ok(())
}

// ============================================================================
// init
// ============================================================================

/// `init` 建出的项目必备结构。缺任何一项都会让"项目已就绪"这个证据失真：
/// 旧判据只看四个状态目录，于是 `write projects/<名>/…` 顺手建出的半套树报"自洽"
/// exit 0，直到下一次 `new` 才 IO 错。
fn project_skeleton_gaps(store: &Store, project: &str) -> Vec<String> {
    let dir = store.project_dir(project);
    let mut gaps = Vec::new();
    if !dir.is_dir() {
        return vec!["<整个项目目录>".into()];
    }
    for st in STATUSES {
        if !dir.join(st).is_dir() {
            gaps.push(format!("{st}/"));
        }
    }
    for f in ["meta.md", "pitfalls.md"] {
        if !dir.join(f).is_file() {
            gaps.push(f.into());
        }
    }
    gaps
}

pub fn init(
    store: &Store,
    project: &str,
    agents_file: &str,
    at: &Path,
    force: bool,
    no_agents: bool,
) -> Result<()> {
    // 防呆（§1.1 安装语义）：先判入口文件，避免"半初始化 + 静默覆盖"。
    // **全部前置检查排在任何写盘之前**（C39）：从前 `read_to_string(&target).ok()` 把
    // "读不出来"（非法 UTF-8、权限）吞成"文件不存在"，于是既有的手改入口不加 --force 也被
    // 静默覆盖，或骨架建完后才报 IO 错、留下一套半成品。
    let overlay_entry = templates::overlay_present(store, "AGENTS.md.tpl");
    let rendered = render_entry(store, PROTO_VERSION)?;
    let target = at.join(agents_file);
    // --no-agents 承诺"完全不触碰目标仓库"，因此连读都不读。
    let mut existing: Option<String> = None;
    if !no_agents {
        match std::fs::read(&target) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => existing = Some(s),
                Err(_) if force => {}
                Err(_) => {
                    return Err(Error::Conflict {
                        message: format!(
                            "`{}` 读不出（不是合法 UTF-8），无法与模板比对 → 拒绝：不覆盖、也不建任何骨架。\n\
                             = 确认要丢弃它：加 `--force`；只想补状态骨架：加 `--no-agents`。",
                            target.display()
                        ),
                    })
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Error::Conflict {
                    message: format!(
                        "`{}` 存在但读不出来（{e}）→ 拒绝：不覆盖、也不建任何骨架（免留半套项目）。\n\
                         = 先修好该文件的可读性；只补状态骨架加 `--no-agents`；确认要整份替换加 `--force`。",
                        target.display()
                    ),
                })
            }
        }
    }
    let write_entry = match &existing {
        None => !no_agents,
        Some(cur) if *cur == rendered => false, // 幂等：已是最新，不重写
        Some(_) if force => true,               // 显式覆盖
        Some(_) => {
            return Err(Error::Conflict {
                message: format!(
                    "`{}` 已存在且与 Athena 模板不同，拒绝覆盖。\n\
                     = 想覆盖并初始化：加 `--force`；只建状态骨架、不碰该文件：加 `--no-agents`。\n\
                     = 本次未创建任何状态、未改动任何文件（§1.1：接口可进仓库，但不静默改用户仓库）。",
                    target.display()
                ),
            });
        }
    };

    std::fs::create_dir_all(store.root.join("pitfalls"))?;
    std::fs::create_dir_all(store.root.join("templates"))?;
    for status in STATUSES {
        std::fs::create_dir_all(store.status_dir(project, status))?;
    }
    std::fs::create_dir_all(store.root.join(".locks"))?;

    // 写入默认协议文件（已存在不覆盖，§2.1 覆盖机制）。
    write_if_absent(
        &store.config_toml(),
        &templates::load(store, "config.toml.tpl")?,
    )?;
    write_if_absent(
        &store.terms_md(),
        &templates::load(store, "terms.md.tpl")?,
    )?;
    write_if_absent(
        &store.terms_toml(),
        &templates::load(store, "terms.local.toml.tpl")?,
    )?;
    write_if_absent(
        &store.global_pitfalls(),
        "# 全局被坑（跨项目通用）\n\
         \n\
         - 原理可行 ≠ 原理已被实证；没跑反证实验的提案不离开池（§5.1a）。\n\
         - 在 Niri 上靠平台 API 定位自己：空值 + 恒返回 (0,0)，success 为真（fidus 教训，§5.1b）。\n\
         - AI 声称完成但没跑测试 → 必须实测结果才能 complete。\n",
    )?;
    write_if_absent(&store.notices_md(), NOTICE_HEADER)?;
    // 人话台账（C23）：入口把它列为最高优先级证据，但从前 init **从不创建**这两份文件，
    // 于是 `context` 的台账段静默缺席——"没有文件"与"这人没说过话"被糊成同一件事。
    write_if_absent(
        &store.global_voice(),
        "# 全局人话台账（跨项目约定 · 只记人的原话逐字）\n\n\
         <!-- 格式：- <时间> [类别] 「原话」 关于:<slug|->\n\
              类别 ∈ 指令/授权/裁决/约定/否决；[定] 标只由人给，AI 不得自加。\n\
              AI 的转述与推断不入台账；原话不可得须标「注:转述」，不得伪装成逐字。\n\
              只追加、禁整写覆盖（状态根多 agent 共享）：`athena append voice.md --stdin`。 -->\n",
    )?;
    write_if_absent(
        &store.project_voice(project),
        &format!(
            "# {project} · 项目人话台账（只记人的原话逐字）\n\n\
             <!-- 同上格式。追加：`athena append projects/{project}/voice.md --stdin`（无 --stdin 时空内容会被拒）。 -->\n"
        ),
    )?;
    templates::materialize_defaults(store)?;

    let meta = format!(
        "---\nproject: {project}\nathena-version: {PROTO_VERSION}\n---\n\n# {project}\n\n\
         <!-- 项目是什么。AI/人可直接编辑（§1.1）。 -->\n"
    );
    write_if_absent(&store.project_dir(project).join("meta.md"), &meta)?;
    write_if_absent(
        &store.project_pitfalls(project),
        &format!("# {project} · 项目级被坑\n\n- （每条对应一次真实踩坑）\n"),
    )?;

    Git::init(&store.root)?;

    // 复制协议入口文件到项目仓库根（复制而非软链，§1.1「安装语义」）。
    // 仅在需要时写：不存在→建 / --force→覆盖 / 内容已同或 --no-agents→跳过。
    let entry_note = if write_entry {
        std::fs::create_dir_all(
            target
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        std::fs::write(&target, &rendered)?;
        format!("协议入口文件已复制到：{}", target.display())
    } else if no_agents {
        "已按 --no-agents 跳过入口文件（未触碰目标仓库）".to_string()
    } else {
        format!("入口文件已是最新，未改动：{}", target.display())
    };

    // 只提交 init 真正落地的路径，绝不 `add -A` 扫全树：否则会卷进并发会话
    // 或上一轮遗留在**其它项目**目录里的脏改动（事故：`init pixel-raider`
    // 把 athena/pool/ 里一个无 frontmatter 的残片提交到 pixel-raider 名下）。
    let touched: Vec<PathBuf> = vec![
        srel(store, &store.config_toml()),
        srel(store, &store.terms_md()),
        srel(store, &store.terms_toml()),
        srel(store, &store.global_pitfalls()),
        srel(store, &store.global_voice()),
        srel(store, &store.notices_md()),
        srel(store, &store.templates_dir()),
        srel(store, &store.project_dir(project)),
    ];
    Git::commit_paths(
        &store.root,
        &touched,
        &format!("init: 项目 {project} 骨架"),
        &actor(),
    )?;
    // ④ 缺失时如实说出本次入口文本从哪来（C38）：从前静默用二进制内置，
    // 手改过 ④ 又误删的人只会发现"我的 overlay 不知何时不生效了"。
    let overlay_note = if overlay_entry {
        String::new()
    } else {
        "· 生效模板 ④ 缺失：本次入口文本取自**二进制内置**，已铺出 ~/.Athena/templates/AGENTS.md.tpl（此后改 ④ 优先于内置）。\n"
            .to_string()
    };
    println!(
        "已初始化 ~/.Athena 与项目 `{project}`。\n\
         {overlay_note}\
         {entry_note}（复制非软链，可安全提交；不含任何工作状态）。\n\
         下一步：在项目里让 AI 读本入口，再 `athena context` / `athena validate`。"
    );
    Ok(())
}

fn render_entry(store: &Store, version: &str) -> Result<String> {
    let tpl = templates::load(store, "AGENTS.md.tpl")?;
    let mut vars = BTreeMap::new();
    vars.insert("version".to_string(), version.to_string());
    // 替换 frontmatter 中的版本占位（模板已含固定 version，这里兜底替换 athena-version 行）。
    let out = templates::render(&tpl, &vars);
    Ok(out.replace(
        "athena-version: 0.1.0",
        &format!("athena-version: {version}"),
    ))
}

fn write_if_absent(path: &Path, text: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

// ============================================================================
// new
// ============================================================================

pub fn new_item(store: &Store, project: &str, slug: &str, kind: &str, reuse: bool) -> Result<()> {
    require_initialized(store)?;
    validate_slug(slug)?;
    // 唯一性：任意进行中状态目录不得同名（§5.2a）。
    if let Some(it) = find_item(store, project, slug)? {
        if it.status != "finished" {
            return Err(Error::Slug {
                slug: slug.into(),
                message: format!(
                    "已存在于 {}（{}），项目内必须唯一",
                    it.status,
                    it.doc.fm.kind.as_str()
                ),
            });
        }
        // 同名的 finished 项不是"警告后照建"的理由：新项落 pool/ 后同一 slug 在两个
        // 状态目录各有一份，而 find_item 对 2+ 命中一律 exit 1 → 该 slug 的
        // promote/complete/resume/freeze/deepen 全部瘫掉，validate 却只列一行且 exit 0。
        if !reuse {
            return Err(Error::Slug {
                slug: slug.into(),
                message: format!(
                    "同名历史项已存在于 {}（kind: {}）。\n                     = 想留新项：换个 slug；\n                     = 确要同名（新项落 pool/，旧项仅作历史）：加 `--reuse-finished`。\n                     = 想把旧项捞回在途：`athena resume {slug}`，别再新建。",
                    srel(store, &it.path).display(),
                    it.doc.fm.kind.as_str()
                ),
            });
        }
        println!("⚠ 同名 finished 项保留为历史，新项将在 pool/ 创建（此后按 slug 的动作只看这一份）。");
    }
    let tpl_name = templates::template_for_kind(kind);
    let mut vars = BTreeMap::new();
    vars.insert("slug".into(), slug.into());
    vars.insert("kind".into(), kind.into());
    vars.insert("status".into(), "pool".into());
    vars.insert("project".into(), project.into());
    vars.insert("actor".into(), actor());
    vars.insert("now".into(), templates::now_iso());
    let rendered = templates::render(&templates::load(store, tpl_name)?, &vars);
    let mut doc = Document::parse(
        &store.status_dir(project, "pool").join(format!("{slug}.md")),
        &rendered,
    )?;
    doc.fm.slug = slug.into();
    doc.fm.project = project.into();
    doc.fm.status = "pool".into();
    let path = store.status_dir(project, "pool").join(format!("{slug}.md"));
    doc.write(&path)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &path)],
        &format!("new: pool/{slug} ({kind})"),
        &actor(),
    )?;
    println!("已创建 pool/{slug}.md（kind: {kind}）。想推进先写剪枝章节 + 反证留痕，再 `athena promote {slug}`。");
    Ok(())
}

// ============================================================================
// 校验
// ============================================================================

/// 反证模式的**单一权威**是 `config.toml [behavior].falsification_mode`（C35/C7）。
/// 从前它压死 `terms.local.toml` 的同名开关，且 config 语法坏时被 `.ok()` 吞掉、静默
/// 回落到 terms 的值——生效模式随"哪份文件坏了"翻转。现在：解析失败直接报错（受闸动作
/// 不执行），取值走白名单（拼错的 `nonsense` 从前按 warn 放行、只在首行原样打印）。
fn effective_falsification_mode(cfg: &Config) -> Result<String> {
    let mode = cfg.behavior.falsification_mode.as_deref().unwrap_or("warn");
    if !matches!(mode, "warn" | "block") {
        return Err(Error::Conflict {
            message: format!(
                "config.toml 的 falsification_mode = \"{mode}\" 不是合法取值。\n                     = 只认 warn（标红不阻塞）| block（拒绝推进）。\n                     = 唯一真值在 ~/.Athena/config.toml [behavior]；terms.local.toml 里的 mode 已废弃（`athena term validate` 会点名）。"
            ),
        });
    }
    Ok(mode.to_string())
}

/// 入口行数预算（C2/C26）。从前恒返回 None——"仓库根入口不在 ~/.Athena 里"是真限制，
/// 但代价是 `max_entry_lines` 成了写在模板里的假保障（入口 83→100 行无人报警）。
/// 现在量两份读得到的：**④ 生效模板**（`init` 铺出去的就是它）与**当前目录的入口副本**
/// （⑤，你正在读的这份）。达到 `max_entry_lines` 即报；换名落地的副本仍要人工盯。
fn entry_budget_warning(cfg: &Config, store: &Store) -> Vec<String> {
    let Some(max) = cfg.behavior.max_entry_lines else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut candidates = vec![store.root.join("templates/AGENTS.md.tpl")];
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("AGENTS.md"));
    }
    for p in candidates {
        if !p.is_file() {
            continue;
        }
        let key = p.canonicalize().unwrap_or_else(|_| p.clone());
        if !seen.insert(key) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&p) else {
            continue;
        };
        let n = raw.lines().count();
        if n >= max {
            out.push(format!(
                "{n} 行 ≥ 预算 {max}：{}（§1.1 常驻上下文必须短；要加行先剪行）",
                p.display()
            ));
        }
    }
    out
}

fn check_item(
    path: &Path,
    doc: &Document,
    located: &str,
    terms: &TermsRegistry,
    mode: &str,
) -> Vec<Finding> {
    let ctx = ValidationCtx {
        path,
        doc,
        located_status: located,
        terms,
        falsification_mode: mode,
    };
    rules::registry()
        .iter()
        .flat_map(|r| r.check(&ctx))
        .collect()
}

pub fn validate(store: &Store, project: &str) -> Result<bool> {
    require_initialized(store)?;
    // 术语/配置先解析（§9.1 解析优先）。
    let terms = TermsRegistry::load(&store.terms_toml())?;
    let cfg = Config::load(store)?;
    let mode = effective_falsification_mode(&cfg)?;
    let items = list_items(store, project)?;
    let mut has_error = false;
    let mut total = 0usize;
    // 退出码三态写在首行（C21）：脚本只看 `0` 会把"有 ⚠"误读成"干净"。
    println!(
        "athena validate · 项目 {project} · 反证模式={mode}\n\
         · 退出码：0=报告无 Error 级发现（可与任意多条 ⚠ 共存）｜1=命令自身失败｜2=报告含 Error 级发现"
    );
    // 项目骨架完整性：`write projects/<名>/…` 也会顺手建出目录，只建一层的项目
    // 对"四目录在不在"的旧判据完全自洽，直到下一次 `new` 才 IO 错。
    let gaps = project_skeleton_gaps(store, project);
    if !gaps.is_empty() {
        has_error = true;
        total += gaps.len();
        println!("\n── 项目骨架 projects/{project}");
        for g in &gaps {
            println!(
                "  {} [ProjectSkeleton] 缺 {g}（`athena init {project}` 会补齐既有缺失，不覆盖已有文件）",
                Level::Error.tag()
            );
        }
    }
    // 全局通知：跨项目广播，属信息横幅而非缺陷，不计入 total / 不影响自洽判定。
    if let Some(section) = context::notices_section(store) {
        println!("{}", section.trim_end());
    }
    for it in &items {
        let findings = check_item(&it.path, &it.doc, it.status, &terms, &mode);
        if findings.is_empty() {
            continue;
        }
        total += findings.len();
        println!("\n── {}/{}.md", it.status, it.doc.fm.slug);
        for f in findings {
            if f.level == Level::Error {
                has_error = true;
            }
            println!("  {} [{}] {}", f.level.tag(), f.code, f.message);
        }
    }
    // 兑现"解析失败项由 validate 单独报告"的契约（§9.1）：这些文件在 list_items/context 里隐身，
    // 若此处不扫，坏条目就既不报错也不显示（write 静默吞下 + 读侧静默跳过 的合谋）。
    for (path, err) in list_unparseable(store, project) {
        has_error = true;
        total += 1;
        let rel = path
            .strip_prefix(&store.root)
            .unwrap_or(path.as_path())
            .display();
        println!("\n── {rel}");
        println!(
            "  {} [Unparseable] 无法解析，已从 context/list_items 隐身，请补全 frontmatter：{err}",
            Level::Error.tag()
        );
    }
    // 入口文件膨胀检查（§1.1）。
    let budget = entry_budget_warning(&cfg, store);
    if !budget.is_empty() {
        println!("\n── 协议入口预算");
        for w in &budget {
            println!("  {} [EntryBudget] {w}", Level::Warn.tag());
        }
        total += budget.len();
    }
    if total == 0 {
        println!("✓ 无缺失项，状态库自洽。");
    } else {
        println!("\n共 {total} 条提示。这是自检报告：由你（AI）决定补不补，工具不代改（§9.1）。");
    }
    Ok(!has_error)
}

// ============================================================================
// promote / complete / freeze / community / resume / deepen / quick
// ============================================================================

pub fn promote(store: &Store, project: &str, slug: &str, skip: Option<&str>) -> Result<()> {
    require_initialized(store)?;
    let terms = TermsRegistry::load(&store.terms_toml())?;
    let mode = effective_falsification_mode(&Config::load(store)?)?;
    let it = need_item(store, project, slug)?;

    let target = match it.status {
        "pool" => "working",
        "community" => "working",
        "working" => {
            return Err(Error::Transition {
                message: "已在 working/；结束请用 `athena complete`".into(),
            })
        }
        "finished" => {
            return Err(Error::Transition {
                message: "已结束；若要重新推进用 `athena resume`（要求重新剪枝）".into(),
            })
        }
        _ => unreachable!(),
    };

    // 前置校验（§6：promote 先校验再移动）。
    let mut doc = it.doc.clone();
    doc.fm.status = target.to_string();
    if let Some(reason) = skip {
        if reason.trim().is_empty() || reason.chars().count() < 6 {
            return Err(Error::Transition {
                message: "--skip-falsification 需要具象理由（如“CI 无显示服务器…”）".into(),
            });
        }
        register_pending_in_doc(&mut doc, reason);
    }
    let findings = check_item(&it.path, &doc, target, &terms, &mode);
    for f in &findings {
        println!("  {} [{}] {}", f.level.tag(), f.code, f.message);
    }
    if findings.iter().any(|f| f.level == Level::Error) {
        return Err(Error::Transition {
            message:
                "反证/block 模式检出 Error，未移动。补齐证据或换 warn 模式后再 promote（§5.1e）。"
                    .into(),
        });
    }

    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, target)?;
    // 校验全过、文件也挪了，才写台账：此前登记先落盘，被拒时留下"已登记未推进"的
    // 假登记与脏状态库（git status 见 M pending.md）。
    if let Some(reason) = skip {
        append_pending_ledger(store, project, slug, reason);
    }
    let mut touched = vec![srel(store, &src), srel(store, &new_path)];
    if skip.is_some() {
        touched.push(srel(store, &store.project_dir(project).join("pending.md")));
    }
    Git::commit_paths(
        &store.root,
        &touched,
        &format!("promote: {slug} → {target}"),
        &actor(),
    )?;
    println!("✓ {slug}: {} → {target}", it.status);
    Ok(())
}

/// 状态动作代为清算反证登记时，把原登记**搬进正文留痕**，返回是否真有登记被抹掉。
/// 这些动作删的是"证据义务"本身；不搬走原文就等于无痕绕道（禁忌 3 的机器面）。
fn strike_pending_registration(
    store: &Store,
    doc: &mut Document,
    project: &str,
    slug: &str,
    action: &str,
) -> bool {
    let had = doc.fm.falsification.as_deref() == Some("pending");
    let reason = pending_reason(store, project, slug);
    let had_marker = !rules::pending_markers(doc.fm.falsification.as_deref(), &doc.body).is_empty();
    doc.fm.falsification = None;
    doc.body = strip_pending_block(&doc.body);
    if had || had_marker {
        let note = match reason.as_deref() {
            Some(r) => format!("- 反证登记未清算即{action}（绕过，非清算）· 原理由：{r}"),
            None => format!("- 反证登记未清算即{action}（绕过，非清算）· 正文待测标记随本动作移除"),
        };
        doc.body = append_to_section(&doc.body, "决策", &note);
    }
    clear_pending_entry(store, project, slug);
    had || had_marker
}

/// 把"未清算即<action>"那行反过来变成一条正式待测登记（`resume` 用）。
fn revive_pending_registration(store: &Store, doc: &mut Document, project: &str, slug: &str) -> bool {
    let line = doc.body.lines().find(|l| l.contains("反证登记未清算即")).map(String::from);
    let Some(line) = line else {
        return false;
    };
    if doc.fm.falsification.as_deref() == Some("pending") {
        return false;
    }
    let reason = match line.split_once("原理由：") {
        Some((_, rest)) => rest.trim().to_string(),
        None => "曾以状态动作绕过、从未实机执行".to_string(),
    };
    doc.fm.falsification = Some("pending".into());
    append_pending_ledger(store, project, slug, &reason);
    true
}

fn report_bypass(action: &str, slug: &str, struck: bool) {
    if struck {
        println!(
            "⚠ {action} 代你抹掉了 {slug} 未清算的待测反证登记——这是**绕过**，不是清算。\n             = 原登记已搬进正文「决策」章节留痕；complete 前仍须真跑，或按禁忌 6 在决策里写明为何不再追。"
        );
    }
}

/// 从 pending.md 读回某 slug 的登记理由（首列精确匹配，不做子串搜索）。
fn pending_reason(store: &Store, project: &str, slug: &str) -> Option<String> {
    let text = std::fs::read_to_string(store.project_dir(project).join("pending.md")).ok()?;
    for line in text.lines() {
        if let Some((head, rest)) = ledger_split(line) {
            if head == slug {
                return rest.split_once("理由：").map(|(_, r)| r.trim().to_string());
            }
        }
    }
    None
}

/// 台账行的结构：`- [ ] <slug> · <正文>` → (slug, 正文)。
fn ledger_split(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("- [x] "))?;
    let (head, tail) = rest.split_once(" · ")?;
    Some((head.trim(), tail))
}

/// 删掉正文里的 `### 待测` 小节（连标题），止于下一个同级或更高级标题。
fn strip_pending_block(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let level_of = |l: &str| l.chars().take_while(|ch| *ch == '#').count();
    let Some(start) = lines.iter().position(|l| l.trim_start().starts_with("### 待测")) else {
        return body.to_string();
    };
    let base = level_of(lines[start]);
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if l.trim_start().starts_with('#') && level_of(l) <= base {
            end = i;
            break;
        }
    }
    let mut out = lines[..start].to_vec();
    out.extend_from_slice(&lines[end..]);
    out.join("\n") + "\n"
}

/// 仅改内存中的文档：置 frontmatter 标记 + 插入 `### 待测` 小节。
/// 台账行不在这里写（校验没过就不该留下假登记）。
fn register_pending_in_doc(doc: &mut Document, reason: &str) {
    doc.fm.falsification = Some("pending".into());
    let block = format!(
        "\n### 待测（pending verification）\n\
         - 理由：{reason}\n\
         - 登记时间：{}\n\
         - 状态：pending\n",
        templates::today()
    );
    // 插入到 `### 反证实验` 章节末尾。
    doc.body = insert_after_section(doc.body.trim_end(), "反证实验", &block);
}

/// 只写台账那一行（与"改文档"分开，供 promote 校验通过后落笔、resume 恢复登记复用）。
fn append_pending_ledger(store: &Store, project: &str, slug: &str, reason: &str) {
    let pend = store.project_dir(project).join("pending.md");
    let line = format!(
        "- [ ] {slug} · 跳过反证 · 理由：{reason} · {}",
        templates::today()
    );
    if let Err(e) = append_to_markdown_file(&pend, &line) {
        println!("⚠ 待测台账没写进去（{e}）——请手工补一行 pending.md。");
    }
}

pub fn complete(store: &Store, project: &str, slug: &str) -> Result<()> {
    require_initialized(store)?;
    let it = need_item(store, project, slug)?;
    if it.status != "working" {
        return Err(Error::Transition {
            message: format!("complete 需从 working/ 出发，当前 {}", it.status),
        });
    }
    // 唯一保留的硬阻塞：pending 反证未清算（§5.1f / §6 rule3）。三处判据一次列全：
    // 只给一条指引，照着做完仍会被另外两处拦住。
    let hits = rules::pending_markers(it.doc.fm.falsification.as_deref(), &it.doc.body);
    if !hits.is_empty() {
        let listed = hits
            .iter()
            .enumerate()
            .map(|(i, h)| format!("  {}. {}", i + 1, h))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Transition {
            message: format!(
                "{slug} 仍有未清算的待测反证，命中共 {} 处：\n{}\n                 = 三处全部清掉才能 complete；`resume` 救不了（它只从 finished/ 出发）。\n                 = 若这是被 freeze/community 绕过留下的，见正文「决策」章节里「反证登记未清算即」那行的原理由。",
                hits.len(),
                listed
            ),
        });
    }
    // C8：`enforce_on = ["promote","complete"]` 里的 complete 从未落地——剪枝章节校验只在
    // promote 侧跑过。现在按声明的触发点真跑：发现全部打印，Error 级才拦（本类是 Warn，
    // Athena 不阻止、只记录与提示；`term validate` 会把没实现的触发点点名）。
    let terms = TermsRegistry::load(&store.terms_toml())?;
    if terms.prune_enforce_on().iter().any(|a| a == "complete") {
        let missing = rules::missing_prune_fields(&terms, &it.doc);
        if missing.is_empty() {
            println!("· 剪枝章节校验（enforce_on 含 complete）：必填章节齐");
        } else {
            println!("── 剪枝章节校验（enforce_on 含 complete）");
            for m in &missing {
                println!(
                    "  {} [MissingPruneSection] 缺 `{m}`（不硬拦，但收口前该补的证据就这些）",
                    Level::Warn.tag()
                );
            }
        }
    }
    let mut doc = it.doc.clone();
    doc.fm.outcome = Some(Outcome::Done);
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "finished")?;
    clear_pending_entry(store, project, slug);
    let touched = vec![
        srel(store, &src),
        srel(store, &new_path),
        srel(store, &store.project_dir(project).join("pending.md")),
    ];
    Git::commit_paths(
        &store.root,
        &touched,
        &format!("complete: {slug} → finished(done)"),
        &actor(),
    )?;
    println!("✓ {slug}: working → finished (outcome: done)");
    Ok(())
}

pub fn freeze(store: &Store, project: &str, slug: &str, reason: &str) -> Result<()> {
    require_initialized(store)?;
    if reason.trim().is_empty() {
        return Err(Error::Transition {
            message: "毙掉必须写原因（不能无声消失，§5.2）".into(),
        });
    }
    let it = need_item(store, project, slug)?;
    if it.status == "finished" {
        return Err(Error::Transition {
            message: "已在 finished/".into(),
        });
    }
    let mut doc = it.doc.clone();
    doc.fm.outcome = Some(Outcome::Frozen);
    let struck = strike_pending_registration(store, &mut doc, project, slug, "freeze");
    doc.body = append_to_section(&doc.body, "决策", &format!("- 冻结原因：{reason}"));
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "finished")?;
    let touched = vec![
        srel(store, &src),
        srel(store, &new_path),
        srel(store, &store.project_dir(project).join("pending.md")),
    ];
    Git::commit_paths(
        &store.root,
        &touched,
        &format!("freeze: {slug} → finished(frozen)"),
        &actor(),
    )?;
    println!("✓ {slug}: {} → finished (outcome: frozen)", it.status);
    report_bypass("freeze", slug, struck);
    Ok(())
}

pub fn community(store: &Store, project: &str, slug: &str) -> Result<()> {
    require_initialized(store)?;
    let it = need_item(store, project, slug)?;
    if it.status == "community" {
        return Err(Error::Transition {
            message: "已在 community/".into(),
        });
    }
    // block 模式下 promote 会因未清算反证拒绝，而 community 此前不查——等于给入口
    // 宣布的唯一证据类硬闸留了一条更省事的逃生口。转社区同样是"在途"，一并受闸。
    if effective_falsification_mode(&Config::load(store)?)? == "block" {
        let hits = rules::pending_markers(it.doc.fm.falsification.as_deref(), &it.doc.body);
        if !hits.is_empty() {
            return Err(Error::Transition {
                message: format!(
                    "block 模式下不得把带未清算待测反证的 {slug} 转进 community（它是 promote 硬闸的绕道）。\n                     = 先实际执行并填真实结果；确实要先放出去，就在 config.toml 把 falsification_mode 换回 warn 并接受 ⚠ 提示。"
                ),
            });
        }
    }
    let mut doc = it.doc.clone();
    let struck = strike_pending_registration(store, &mut doc, project, slug, "community");
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "community")?;
    Git::commit_paths(
        &store.root,
        &[
            srel(store, &src),
            srel(store, &new_path),
            srel(store, &store.project_dir(project).join("pending.md")),
        ],
        &format!("community: {slug} → community"),
        &actor(),
    )?;
    println!(
        "✓ {slug}: {} → community（放出去请人帮忙，供人搬运，非机器同步）",
        it.status
    );
    report_bypass("community", slug, struck);
    Ok(())
}

pub fn resume(store: &Store, project: &str, slug: &str) -> Result<()> {
    require_initialized(store)?;
    let it = need_item(store, project, slug)?;
    if it.status != "finished" {
        return Err(Error::Transition {
            message: "resume 从 finished/ 出发".into(),
        });
    }
    let mut doc = it.doc.clone();
    doc.fm.outcome = None;
    // freeze/community 抹掉登记与台账行后，resume 只把文件挪回来，留下"从未真清算却
    // 查无登记"的不一致。现在按正文留痕把登记恢复。
    let revived = revive_pending_registration(store, &mut doc, project, slug);
    if !doc.has_heading("反证实验") {
        println!("⚠ 重新推进要求重新剪枝（不能无声复活，§5.2）—— 当前无反证章节，请补。");
    }
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "working")?;
    let mut touched = vec![srel(store, &src), srel(store, &new_path)];
    if revived {
        touched.push(srel(store, &store.project_dir(project).join("pending.md")));
    }
    Git::commit_paths(&store.root, &touched, &format!("resume: {slug} → working"), &actor())?;
    println!("✓ {slug}: finished → working（请重新完成剪枝）");
    if revived {
        println!("· 已恢复当年被状态动作**绕过**（而非清算）的待测登记：pending.md 与 frontmatter 都补回了。");
    }
    Ok(())
}

/// 轴一：原地深化文档类型（改 kind，文件不动，§5.2）。
pub fn deepen(store: &Store, project: &str, slug: &str, to: &str) -> Result<()> {
    require_initialized(store)?;
    let kind: Kind = match to {
        "draft" => Kind::Draft,
        "plan" => Kind::Plan,
        "proposal" => Kind::Proposal,
        _ => {
            return Err(Error::Transition {
                message: "--to 取 draft|plan|proposal".into(),
            })
        }
    };
    let it = need_item(store, project, slug)?;
    // 轴一不该由"已结束/已放出去"的项触发：那两项的 kind 是历史快照，改它等于改写历史，
    // 而 finished/community 里 validate 的剪枝检查全停，改完也没人复核。
    if matches!(it.status, "finished" | "community") {
        let back = if it.status == "finished" {
            format!("athena resume {slug}")
        } else {
            format!("athena promote {slug}")
        };
        return Err(Error::Transition {
            message: format!(
                "deepen 不作用于 {}——那里的 kind 是历史快照，而 finished/community 里剪枝检查全停，改完无人复核。\n                     = 要重开先回在途：`{back}`。",
                it.status
            ),
        });
    }
    let order = [Kind::Proposal, Kind::Draft, Kind::Plan];
    let from_i = order.iter().position(|k| *k == it.doc.fm.kind).unwrap_or(0);
    let to_i = order.iter().position(|k| *k == kind).unwrap();
    if to_i < from_i {
        return Err(Error::Transition {
            message: "不能反向降级文档类型（提案→草案→方案 单向深化）".into(),
        });
    }
    if to_i == from_i {
        // 同值过去也重写文件并新建一条状态库提交：看起来像做过事，实际什么都没变。
        println!(
            "· {slug} 已是 {}，kind 未变——不落盘、不提交（deepen 只单向，降级请改文件后 `git revert`）",
            kind.as_str()
        );
        return Ok(());
    }
    let mut doc = it.doc.clone();
    doc.fm.kind = kind;
    doc.fm.updated_at = templates::now_iso();
    doc.fm.updated_by = actor();
    doc.write(&it.path)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &it.path)],
        &format!("deepen: {slug} kind → {}", kind.as_str()),
        &actor(),
    )?;
    println!(
        "✓ {slug}: kind → {}（文件留在 {}/）",
        kind.as_str(),
        it.status
    );
    Ok(())
}

/// 快速通道：小修复/配置更改，一行留痕进 `## 决策日志`，不走完整剪枝（§5.4）。
pub fn quick(store: &Store, project: &str, slug: &str, msg: &str, do_promote: bool) -> Result<()> {
    require_initialized(store)?;
    let terms = TermsRegistry::load(&store.terms_toml())?;
    let it = need_item(store, project, slug)?;
    if it.status == "pool" {
        return Err(Error::Transition {
            message: "快速通道不得用于 pool→working（新提案从池进入必须完整剪枝，§5.4 防护1）"
                .into(),
        });
    }
    // 累计计数：超过 quick_limit 强制完整剪枝（防护2）。
    let count = count_quick_lines(&it.doc.body);
    if count >= terms.quick_limit {
        return Err(Error::Transition {
            message: format!(
                "{slug} 累计 quick 已达 {count}（上限 {}）——请走完整剪枝再推进（§5.4 防护2）",
                terms.quick_limit
            ),
        });
    }
    let mut doc = it.doc.clone();
    let line = format!("- [{}] quick: {msg} — {}", templates::now_iso(), actor());
    doc.body = append_to_section(&doc.body, "决策日志", &line);
    doc.fm.updated_at = templates::now_iso();
    doc.fm.updated_by = actor();
    doc.write(&it.path)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &it.path)],
        &format!("quick: {slug}"),
        &actor(),
    )?;
    println!(
        "✓ quick 留痕已记入 {slug} 的 ## 决策日志（第 {} 次，上限 {}）",
        count + 1,
        terms.quick_limit
    );
    if do_promote {
        promote(store, project, slug, None)?;
    }
    Ok(())
}

// ============================================================================
// write / append / pitfall
// ============================================================================

/// 写侧互斥锁（C47）：`.locks/` 从前只被 `init` 创建、全仓没有任何使用点——看着像并发
/// 保护，实际是假保障。现在写路径真的取锁：`O_EXCL` 占坑、写完删除，被占且没超过 TTL
/// 就明确拒绝（另一路写入进行中），超过 TTL 视为崩溃残留、抢占并说明。
/// 边界照 §13.2：`O_EXCL` 在 NFS/9p/virtiofs 上不保证强一致，跨边界并发仍可能互踩。
struct WriteLock {
    path: std::path::PathBuf,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

const LOCK_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn acquire_write_lock(store: &Store, rel: &str) -> Result<WriteLock> {
    let dir = store.root.join(".locks");
    std::fs::create_dir_all(&dir)?;
    let name: String = rel.chars().map(|c| if c == '/' { '_' } else { c }).collect();
    let path = dir.join(format!("{name}.lock"));
    for attempt in 0..3 {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{} {}", std::process::id(), actor());
                return Ok(WriteLock { path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(|t| t.elapsed().map(|el| el > LOCK_TTL).unwrap_or(false))
                    .unwrap_or(true);
                if stale {
                    println!(
                        "⚠ {name}：占锁文件超过 {} 秒未释放（疑似崩溃残留），抢占继续。",
                        LOCK_TTL.as_secs()
                    );
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                if attempt == 2 {
                    return Err(Error::Conflict {
                        message: format!(
                            "另一路写入正占着 {rel}（{}）。等它写完再试；确认那是死锁就删掉锁文件。\n                     = 锁目录：.locks/{name}.lock",
                            path.display()
                        ),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Conflict {
        message: format!("取锁失败：{}", path.display()),
    })
}

/// 原子整写（C27）：临时文件 + rename 替换，读侧不会看到写了一半的文件。
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("athena.tmp");
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 追加（C27）：非 item doc 走 `O_APPEND` 直写——从前是"整读→拼接→整写"，多 agent 共享
/// 状态根时后写的那份会覆盖先写的那份（丢别人的行）。item doc 必须整读整写重算
/// content-hash，那条只能靠 `.locks/` 互斥。
fn append_bytes(path: &Path, content: &str) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let needs_nl = match std::fs::metadata(path) {
        Ok(m) if m.len() > 0 => {
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::End(-1))?;
            let mut last = [0u8; 1];
            f.read_exact(&mut last)?;
            last[0] != b'\n'
        }
        _ => false,
    };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    if needs_nl {
        f.write_all(b"\n")?;
    }
    f.write_all(content.as_bytes())?;
    // 一次追加必须落成**完整行**：末字节补 \n，否则下一次追加会粘在同一行（C27 连带）。
    if !content.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(())
}

pub fn write_path(
    store: &Store,
    project: &str,
    rel: &str,
    content: &str,
    allow_empty: bool,
) -> Result<()> {
    require_initialized(store)?;
    // 裸状态目录路径（pool/… 等）依 --project 归位到 projects/<project>/…，
    // 防误写入状态根顶层产生幻影树；显式全路径（projects/…）保持原行为。
    let (rel, rerouted) = route_state_rel(rel, project)?;
    if rerouted {
        println!("· 相对状态目录路径已归位 → {rel}（依项目 {project}；欲写顶层请改用显式路径）");
    }
    ensure_project_target_exists(store, &rel)?;
    // 防呆：空/纯空白内容默认拒绝，避免误清空状态文件（§1.2 审计层之外的一道数据保护）；
    // 确要写空文件用 --allow-empty 显式放行。与 init --force 同类的"拒绝覆盖需显式"约定。
    let path = store.resolve_rel(&rel)?;
    let _lock = acquire_write_lock(store, &rel)?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    // 防呆：空/纯空白内容默认拒绝，避免误清空状态文件（§1.2 审计层之外的一道数据保护）；
    // 确要写空文件用 --allow-empty 显式放行。与 init --force 同类的"拒绝覆盖需显式"约定。
    if !allow_empty && content.trim().is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "拒绝用空内容写入 {rel}（会清空既有内容）。确需清空请加 --allow-empty（§1.2）"
            ),
        });
    }
    // 工作项文档：落盘前校验并归一化（拒绝静默吞下坏 frontmatter）；
    // 其它路径（pitfalls.md / terms / scratch 等）仍是原始状态文件网关（§1.1）。
    let final_content = if is_item_doc_rel(&rel) {
        let mut doc = Document::parse(&path, content)?;
        check_item_doc_placement(&rel, &doc.fm)?;
        doc.fm.updated_by = actor();
        doc.fm.updated_at = templates::now_iso();
        doc.render() // 重算 content-hash（§13.2 第 2 层）
    } else {
        content.to_string()
    };
    write_atomic(&path, &final_content)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &path)],
        &format!("write: {rel}"),
        &actor(),
    )?;
    println!("✓ 写入 {rel}");
    Ok(())
}

pub fn append_path(
    store: &Store,
    project: &str,
    rel: &str,
    content: &str,
    allow_empty: bool,
) -> Result<()> {
    require_initialized(store)?;
    let (rel, rerouted) = route_state_rel(rel, project)?;
    if rerouted {
        println!("· 相对状态目录路径已归位 → {rel}（依项目 {project}；欲写顶层请改用显式路径）");
    }
    ensure_project_target_exists(store, &rel)?;
    let path = store.resolve_rel(&rel)?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    if !allow_empty && content.trim().is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "拒绝用空内容追加到 {rel}（只会留下无意义提交，文件不存在时还会新建空文件）。确需如此请加 --allow-empty（§1.2）"
            ),
        });
    }
    // 行尾补一个换行：`-c "一行"` 不带换行时，下一次追加会**粘在同一行**（台账/散文的
    // 行语义就此失效）。O_APPEND 只保证不覆盖，不补分隔。
    let content = if content.ends_with('\n') {
        std::borrow::Cow::Borrowed(content)
    } else {
        std::borrow::Cow::Owned(format!("{content}\n"))
    };
    let content = content.as_ref();
    // 工作项文档：追加进正文后重算 hash 与 updated-*（避免 content-hash 悬空）；坏 frontmatter 拒绝。
    // 落点一致性也要查：append 能凭空建出 item doc，不查就成了绕过 `write` 那道闸的后门。
    if is_item_doc_rel(&rel) {
        // 只有"整读→改→整写"才需要互斥；O_APPEND 那条不占锁——占锁会把并发追加变成拒绝。
        let _lock = acquire_write_lock(store, &rel)?;
        let mut cur = std::fs::read_to_string(&path).unwrap_or_default();
        if !cur.is_empty() && !cur.ends_with('\n') {
            cur.push('\n');
        }
        cur.push_str(content);
        let mut doc = Document::parse(&path, &cur)?;
        check_item_doc_placement(&rel, &doc.fm)?;
        doc.fm.updated_by = actor();
        doc.fm.updated_at = templates::now_iso();
        write_atomic(&path, &doc.render())?;
    } else {
        // 台账/scratch/坑：O_APPEND 直写，不再"整读→拼接→整写"覆盖别人刚追加的行（C27）。
        append_bytes(&path, content)?;
    }
    Git::commit_paths(
        &store.root,
        &[srel(store, &path)],
        &format!("append: {rel}"),
        &actor(),
    )?;
    println!("✓ 追加到 {rel}");
    Ok(())
}

pub fn pitfall(store: &Store, project: &str, text: &str, global: bool) -> Result<()> {
    require_initialized(store)?;
    let rel_target = if global {
        store.global_pitfalls()
    } else {
        store.project_pitfalls(project)
    };
    let line = format!("- {text}  <!-- {} · {} -->", templates::today(), actor());
    append_to_markdown_file(&rel_target, &line)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &rel_target)],
        &format!(
            "pitfall({}): {}",
            if global { "global" } else { project },
            truncate(text, 40)
        ),
        &actor(),
    )?;
    println!(
        "✓ 记入 {}",
        if global {
            "全局被坑"
        } else {
            "项目级被坑"
        }
    );
    Ok(())
}

/// 只读跨源搜索坑：扫 `pitfalls/global.md` + 每个项目的 `pitfalls.md`，返回匹配词条的行。
/// 大小写不敏感子串；多个词（按空白切分）取 AND。顺序：全局 → 当前项目 → 其余项目（字典序）。
/// 纯读、不写、不 commit、不改状态（与不设防一致，pitfall-search 提案）。
pub fn pitfall_search(store: &Store, project: &str, query: &str) -> Result<()> {
    require_initialized(store)?;
    let tokens: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    if tokens.is_empty() {
        println!("· 搜索词条为空，未匹配任何坑。");
        return Ok(());
    }

    // (来源标签, 路径)：先去重收集，其余项目字典序，全局与当前项目置顶。
    let mut sources: Vec<(String, PathBuf)> = vec![("global".into(), store.global_pitfalls())];
    let cur = (project.to_string(), store.project_pitfalls(project));
    let mut others: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(store.projects_dir()) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == project {
                continue;
            }
            others.push((name, entry.path().join("pitfalls.md")));
        }
    }
    others.sort();
    sources.push(cur);
    sources.extend(others);

    let mut total = 0usize;
    let mut scanned = 0usize; // 实际含 pitfalls.md 且读成功的来源数
    for (label, path) in &sources {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue; // 该项目/全局尚无 pitfalls.md，跳过（不计入已扫来源）
        };
        scanned += 1;
        let mut shown_header = false;
        for (i, raw) in content.lines().enumerate() {
            let line = raw.trim();
            // 只匹配坑条目（markdown 列表行）。标题、散文与 init 播种的占位示例都不算命中：
            // 从前搜"全局"会命中文件标题，报出一条根本不存在的坑（C9 假阳性）。
            if !is_pitfall_entry(line) {
                continue;
            }
            let low = line.to_lowercase();
            if tokens.iter().all(|t| low.contains(t)) {
                if !shown_header {
                    println!("── {label}");
                    shown_header = true;
                }
                let body = strip_pitfall_tail_comment(line.trim_start_matches(['-', '*']).trim_start());
                println!("  L{}: {}", i + 1, body);
                total += 1;
            }
        }
    }
    println!(
        "\n共 {total} 条命中（已扫 {scanned} 个含坑文件的来源；只匹配坑条目，标题/占位示例/散文不计；只读，未改动任何文件）。",
    );
    Ok(())
}

/// 坑条目判据：markdown 列表行，且不是占位示例。
fn is_pitfall_entry(line: &str) -> bool {
    let line = line.trim();
    (line.starts_with("- ") || line.starts_with("* ")) && !is_placeholder_entry(line)
}

/// 占位条目：init 播种的示例行（整条正文被括号包住，如 `- （每条对应一次真实踩坑）`）。
/// 它不是真实踩坑记录，计入命中就成了一条凭空出现的"已经记过"。
fn is_placeholder_entry(line: &str) -> bool {
    let body = strip_pitfall_tail_comment(
        line.trim_start_matches(['-', '*'])
            .trim_start(),
    );
    let chars: Vec<char> = body.chars().collect();
    match (chars.first(), chars.last()) {
        (Some('（'), Some('）')) | (Some('('), Some(')')) => true,
        _ => false,
    }
}

/// 剥掉坑行尾部的 `<!-- 日期 · actor -->` 注释，只留可读正文。
fn strip_pitfall_tail_comment(line: &str) -> &str {
    match line.find("<!--") {
        Some(idx) => line[..idx].trim_end(),
        None => line.trim_end(),
    }
}

// ============================================================================
// notify：全局通知广播（单一信道 ~/.Athena/notices.md · §13.6 不预支复杂度）
// ============================================================================

/// `athena notify "文本"` 追加一条全局广播；`--clear` 清空广播板。
///
/// 有意做成"逐项目无状态"：不追踪谁读过（§3 否决工单式回执），各项目 AI 通过
/// `context`/`validate` 顶部看到未清理的通知即可。通知非工作项，不进 list_items/validate 判定。
pub fn notify(store: &Store, text: Option<&str>, clear: bool) -> Result<()> {
    require_initialized(store)?;
    let path = store.notices_md();
    let has_text = text.is_some_and(|s| !s.trim().is_empty());
    // 文本与 --clear 同给：从前**文本被静默丢弃**、只清空广播板还报 rc=0，
    // 调用方以为广播成功了（C10）。与 pitfall 的互斥口径一致：二选一。
    if clear && has_text {
        return Err(Error::Conflict {
            message: "`notify <文本>` 与 `--clear` 只能二选一：同给时文本会被丢弃、只剩清空。\n\
                     = 要发通知就去掉 --clear；要清空就别带文本；两者都要请分两次跑。"
                .into(),
        });
    }
    if clear {
        if path.exists() {
            std::fs::write(&path, NOTICE_HEADER)?;
            Git::commit_paths(
                &store.root,
                &[srel(store, &path)],
                "notify: 清空全局通知",
                &actor(),
            )?;
            println!("✓ 已清空全局通知（notices.md）：这是**跨项目整块**广播板，不只清本项目的条目");
        } else {
            println!("· 无 notices.md，无需清空");
        }
        return Ok(());
    }
    let text = text.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        Error::Conflict {
            message: "需要通知文本，或用 `athena notify --clear` 清空广播板；文本以 `-` 开头时用 `notify -- \"…\"`".into(),
        }
    })?;
    if !path.exists() {
        std::fs::write(&path, NOTICE_HEADER)?;
    }
    // 多行正文从前只有首行能被渲染（顶部段过滤掉了不以 `- ` 开头的行），
    // 而 CLI 照样打印"顶部可见"。现在首行成条、其余行缩进两格作续行，两边口径一致（C40）。
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let stamp = templates::now_iso();
    let actor = actor();
    let mut block = format!("- {stamp} · {}  <!-- {actor} -->", lines[0]);
    for cont in &lines[1..] {
        block.push_str(&format!("\n  {cont}"));
    }
    append_to_markdown_file(&path, &block)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &path)],
        &format!("notify: {}", truncate(lines[0], 40)),
        &actor,
    )?;
    println!(
        "✓ 已广播全局通知（{} 行成 1 条）→ 各项目 `athena context` / `athena validate` 顶部可见",
        lines.len()
    );
    Ok(())
}

// ============================================================================
// context / log / term
// ============================================================================

pub fn show_context(store: &Store, project: &str) -> Result<()> {
    require_initialized(store)?;
    let terms = TermsRegistry::load(&store.terms_toml())?;
    print!("{}", context::build(store, project, &terms)?);
    Ok(())
}

pub fn show_log(store: &Store, n: usize) -> Result<()> {
    require_initialized(store)?;
    for (h, subj) in Git::log(&store.root, n)? {
        println!("{h} {subj}");
    }
    Ok(())
}

/// 故障处置手册（给 AI 的逃生说明）。`athena onerror` 打印。
///
/// 源码路径不烤进二进制：优先 `ATHENA_SOURCE` 环境变量，其次 config 的 `source_repo`，
/// 都没有则提示如何配置——换机/换宿主时各自指路，避免失效。
pub fn onerror(store: &Store) -> Result<()> {
    let source = std::env::var_os("ATHENA_SOURCE")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            Config::load(store)
                .ok()
                .and_then(|c| c.source_repo)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        });
    let source_line = match &source {
        Some(p) => format!("{}", p.display()),
        None => "（未配置）设置环境变量 ATHENA_SOURCE，或在 ~/.Athena/config.toml 写 source_repo = \"…\""
            .to_string(),
    };
    let report_dir = std::env::temp_dir();

    println!(
        "Athena 故障处置手册（给 AI）\n\
         ────────────────────────────\n\
         前提：你对状态根 {root} 有读写权（§前言）。故障时可自行处置，不必等我。\n\
         \n\
         1) 先诊断，别乱改：\n\
         \x20  athena validate   # 看是否有 [Unparseable] / 结构问题\n\
         \x20  athena context    # 核对协议摘要、目录树、术语、坑\n\
         \x20  athena log        # 回看最近动作，定位哪一步出的问题\n\
         \n\
         2) 分清问题在「数据」还是「代码」：\n\
         \x20  · 数据（某文档坏）：直接修 {root} 下那个 .md（补 frontmatter / 修 YAML），\n\
         \x20    或删掉重建（athena new）。git 是审计层，可回看与回溯。\n\
         \x20  · 代码（CLI 自身 panic/逻辑错）：只读审阅源码仓库：\n\
         \x20      {source}\n\
         \x20    看清根因，不要手改正在使用的二进制。\n\
         \n\
         3) 临时产物一律放 {tmp}/（别落状态根、别落源码仓）：\n\
         \x20  错误报告 / 复现脚本 / 临时导出…… 例：{tmp}/athena-error-<日期>.md\n\
         \n\
         4) 立场（§1.3）：Athena 不阻止，只记录与提示。\n\
         \x20  若流程要靠「拦住 AI」才能维持，那是协议文本没写好——改协议，别加固 CLI。\n\
         \n\
         5) 修好后：把根因写进避坑，防复犯：\n\
         \x20  athena pitfall \"<症状 + 根因 + 规避>\" [--global]",
        root = store.root.display(),
        source = source_line,
        tmp = report_dir.display(),
    );
    Ok(())
}

pub fn term_list(store: &Store) -> Result<()> {
    let path = store.terms_toml();
    let builtin = TermsRegistry::defaults();
    let terms = TermsRegistry::load(&path)?;
    let mode = effective_falsification_mode(&Config::load(store)?)?;
    let over = if path.exists() {
        format!("文件覆盖/新增，生效 {} 项", terms.terms.len())
    } else {
        format!("文件不存在，生效的是内置 {} 项", terms.terms.len())
    };
    println!(
        "术语（quick_limit={}，反证模式={mode}〔唯一真值：config.toml [behavior]〕，内置 {} 项为底 + {over}）",
        terms.quick_limit,
        builtin.terms.len()
    );
    for (slug, t) in &terms.terms {
        let syn = if t.synonyms.is_empty() {
            String::new()
        } else {
            format!("  ~ {}", t.synonyms.join(", "))
        };
        println!("  {slug}{syn}");
    }
    Ok(())
}

pub fn term_validate(store: &Store) -> Result<()> {
    use crate::term::{KNOWN_ROOT_KEYS, KNOWN_TERM_KEYS};
    let path = store.terms_toml();
    let builtin = TermsRegistry::defaults();
    // C19：从前文件缺失也报"✓ 解析通过"（校的其实是内置默认），把"你没配"说成"你配对了"。
    if !path.exists() {
        println!(
            "ℹ {} 不存在：当前生效的是**内置默认**（{} 个术语，quick_limit={}）。",
            path.display(),
            builtin.terms.len(),
            builtin.quick_limit
        );
        println!("  · 要自定义术语就跑 `athena term new <slug>`；文件写好后**按表名**叠在内置默认之上，不是取代它。");
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let value: toml::Value = toml::from_str(&raw).map_err(|e| Error::Toml {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let merged = TermsRegistry::load(&path)?;
    let mut problems: Vec<String> = Vec::new();
    if let Some(tbl) = value.as_table() {
        for k in tbl.keys() {
            if !KNOWN_ROOT_KEYS.contains(&k.as_str()) {
                problems.push(format!("顶层未知键 `{k}`：CLI 不读它，只会静默忽略（拼错了？）"));
            }
        }
    }
    match value.get("term").and_then(|v| v.as_table()) {
        None => problems.push("文件里没有 [term.*] 表：生效的全是内置默认".to_string()),
        Some(t) => {
            for (name, tv) in t {
                let Some(body) = tv.as_table() else {
                    problems.push(format!("[term.{name}] 不是表，CLI 读不到"));
                    continue;
                };
                for k in body.keys() {
                    if KNOWN_TERM_KEYS.contains(&k.as_str()) {
                        continue;
                    }
                    if k == "mode" {
                        problems.push(format!(
                            "[term.{name}].mode 已废弃：反证模式的唯一真值是 config.toml 的 [behavior].falsification_mode。这一行不会生效，请删掉"
                        ));
                    } else {
                        problems.push(format!("[term.{name}] 未知键 `{k}`：CLI 不读它，只会静默忽略（拼错了？）"));
                    }
                }
                if let Some(s) = body.get("slug").and_then(|v| v.as_str()) {
                    if s != name {
                        problems.push(format!(
                            "[term.{name}] 里 slug = \"{s}\" 与表名不符：CLI 按**表名**寻址，字段值只是给人看的"
                        ));
                    }
                }
                if let Some(arr) = body.get("enforce_on").and_then(|v| v.as_array()) {
                    for a in arr {
                        match a.as_str() {
                            None => problems.push(format!("[term.{name}].enforce_on 含非字符串项")),
                            Some(s) if !matches!(s, "promote" | "complete") => problems.push(format!(
                                "[term.{name}].enforce_on = \"{s}\" 没有对应触发点：CLI 只在 promote/complete 前跑剪枝章节校验"
                            )),
                            Some(_) => {}
                        }
                    }
                }
                // 整表覆盖的代价要说出口：少了 require_fields，禁忌 2/4 的机器覆盖就归零。
                if let Some(b) = builtin.terms.get(name) {
                    if !b.require_fields.is_empty() && !body.contains_key("require_fields") {
                        problems.push(format!(
                            "[term.{name}] 覆盖了内置整表却没写 require_fields：内置那 {} 项必填章节校验就此归零（写 [] 是明确关闭，整行不写是被覆盖掉）",
                            b.require_fields.len()
                        ));
                    }
                }
            }
        }
    }
    println!(
        "✓ {} 解析通过：内置 {} 项为底，文件覆盖/新增 {} 项，生效 {} 项（quick_limit={}）",
        path.display(),
        builtin.terms.len(),
        value.get("term").and_then(|v| v.as_table()).map(|t| t.len()).unwrap_or(0),
        merged.terms.len(),
        merged.quick_limit
    );
    for p in &problems {
        println!("  {} [TermConfig] {p}", Level::Warn.tag());
    }
    if problems.is_empty() {
        println!("  · 未知键 / 废弃键 / 非法枚举：无。");
    } else {
        println!("\n共 {} 条提示（不阻塞，但那条键根本没被读到）。", problems.len());
    }
    Ok(())
}

pub fn term_new(store: &Store, slug: &str, origin: Option<&str>) -> Result<()> {
    let toml_path = store.terms_toml();
    let mut raw = std::fs::read_to_string(&toml_path).unwrap_or_else(|_| "# terms\n".to_string());
    if raw.contains(&format!("[term.{slug}]")) {
        return Err(Error::Slug {
            slug: slug.into(),
            message: "术语已存在".into(),
        });
    }
    if !raw.ends_with('\n') {
        raw.push('\n');
    }
    let origin_line = origin
        .map(|o| format!("origin = \"{o}\"\n"))
        .unwrap_or_default();
    raw.push_str(&format!(
        "\n[term.{slug}]\nslug = \"{slug}\"\n{origin_line}synonyms = []\nrequire_fields = []\ndefinition = \"\"\n"
    ));
    std::fs::write(&toml_path, &raw)?;
    // 在 terms.md 追加人读章节。
    append_to_markdown_file(
        &store.terms_md(),
        &format!("\n## {slug}\n<!-- 定义与边界，AI 读。 -->\n"),
    )?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &toml_path), srel(store, &store.terms_md())],
        &format!("term new: {slug}"),
        &actor(),
    )?;
    println!("✓ 新增术语骨架 `{slug}`：编辑 terms.local.toml 的 [term.{slug}] 与 terms.md 后 `athena term validate`。");
    Ok(())
}

// ============================================================================
// 辅助
// ============================================================================

fn need_item(store: &Store, project: &str, slug: &str) -> Result<Item> {
    find_item(store, project, slug)?.ok_or_else(|| Error::Slug {
        slug: slug.into(),
        message: format!("在 {project} 的任何状态目录中都找不到"),
    })
}

/// 相对状态库根的路径，用于 `git add -- <path>` 精确暂存本次动作真正触碰的文件。
fn srel(store: &Store, p: &Path) -> PathBuf {
    p.strip_prefix(&store.root).unwrap_or(p).to_path_buf()
}

/// 把裸状态目录相对路径（pool/working/finished/community 开头）依当前项目归位到
/// `projects/<project>/…`，杜绝误写入状态根顶层（write-ignores-project 缺陷）。
/// 已限定路径（projects/…、templates/…、pitfalls/… 等）原样返回。
/// 返回 (归位后路径, 是否发生归位)。
/// 裸状态目录路径（`pool/x.md`）归位到 `projects/<project>/pool/x.md`。
///
/// 归位判定是**词法首段比对**，所以两种"看着像状态路径"的写法必须先拒：
/// ① 参数带首尾空白（`" pool/x.md"`）会静默不归位，把状态文件写进状态根顶层
/// 一个叫 `" pool"` 的目录，context/validate 一字不提；② 首段去掉尾随空白后才
/// 等于状态名（`pool /x.md`）同理。真正带 `..`/绝对路径的越界由 `resolve_rel` 拦。
fn route_state_rel(rel: &str, project: &str) -> Result<(String, bool)> {
    if rel != rel.trim() {
        return Err(Error::BadPath {
            what: "状态路径".into(),
            value: rel.into(),
            message: "首尾含空白会绕过项目归位，把文件写进状态根顶层的幻影目录。请去掉多余空白。"
                .into(),
        });
    }
    let trimmed = rel.trim_start_matches("./");
    let first = trimmed.split(['/', '\\']).next().unwrap_or("");
    if first != first.trim_end() && STATUSES.contains(&first.trim_end()) {
        return Err(Error::BadPath {
            what: "状态路径".into(),
            value: rel.into(),
            message: format!(
                "首段 `{first}` 含尾随空白，不等于任何状态目录（{}），因此不会被归位到 projects/{project}/ 下。",
                STATUSES.join("|")
            ),
        });
    }
    if STATUSES.contains(&first) {
        Ok((format!("projects/{project}/{trimmed}"), true))
    } else {
        Ok((rel.to_string(), false))
    }
}

/// 写 item doc 时查 frontmatter 与**落点路径**的一致性（`project:`/`status:`/`slug:`）。
/// 过去只查 frontmatter 能不能解析：三者任一种漂移都能 rc=0 落盘并自动提交，事后
/// `validate` 也只补标 status 与 slug 两类，`project` 漂移从头到尾无人报——于是
/// "写在 a 项目、声明自己是 b 项目"的文档既进不了 b 的 context，也不受 a 的清算。
fn check_item_doc_placement(rel: &str, fm: &Frontmatter) -> Result<()> {
    let seg: Vec<&str> = rel
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    // 调用方已用 is_item_doc_rel 确认形状为 projects/<p>/<status>/<name>.md。
    let (project, status, file) = (seg[1], seg[2], seg[3]);
    let stem = file.trim_end_matches(".md");
    let mut drift = Vec::new();
    if fm.project != project {
        drift.push(format!(
            "`project: {}` ≠ 路径所属项目 `{project}`（该文档既不进 {project} 的 context，也不受其清算）",
            fm.project
        ));
    }
    if fm.status != status {
        drift.push(format!("`status: {}` ≠ 所在目录 `{status}`（目录才是状态真值）", fm.status));
    }
    if fm.slug != stem {
        drift.push(format!("`slug: {}` ≠ 文件名 `{stem}`", fm.slug));
    }
    if !drift.is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "工作项与落点不一致：{rel}\n                 - {}\n                 = 改 frontmatter 或换路径，二者取其一。",
                drift.join("\n                 - ")
            ),
        });
    }
    Ok(())
}

/// `write`/`append` 只允许落到**已存在**的项目目录里。项目骨架由 `init` 建立；
/// 顺手 `create_dir_all` 会造出半套项目（validate 只看四目录在不在，故报"自洽"，
/// 直到下一次 `new` 才 IO 错），把"项目已就绪"的证据变成假阴性。
fn ensure_project_target_exists(store: &Store, rel: &str) -> Result<()> {
    let mut segs = rel.split('/');
    let project = match (segs.next(), segs.next()) {
        (Some("projects"), Some(p)) => p.to_string(),
        _ => return Ok(()),
    };
    if store.project_dir(&project).is_dir() {
        return Ok(());
    }
    Err(Error::BadPath {
        what: "目标项目".into(),
        value: project.clone(),
        message: format!(
            "`projects/{project}` 不存在，写入不会替你建项目骨架。\n\
             = 新项目先 `athena init {project}`；裸状态目录路径（pool/… 等）会自动归位到已解析的项目下，不必手写 projects/ 前缀。"
        ),
    })
}

/// 把工作项移动到目标状态目录：更新 frontmatter，写新文件，删旧文件。返回新文件路径。
fn move_to_status(
    store: &Store,
    doc: &mut Document,
    old_path: &Path,
    project: &str,
    slug: &str,
    target: &str,
) -> Result<PathBuf> {
    doc.fm.status = target.to_string();
    // outcome 只对 finished/ 有意义；留在 working/community 里会产出
    // "status: working 且 outcome: done" 的自相矛盾文档，而 validate 对它一字不提。
    if target != "finished" {
        doc.fm.outcome = None;
    }
    doc.fm.updated_at = templates::now_iso();
    doc.fm.updated_by = actor();
    let new_path = store.status_dir(project, target).join(format!("{slug}.md"));
    // 目标已存在 = 这一"移动"其实是**覆盖**：`--reuse-finished` 造出的同名项走完
    // complete 时，会把 finished/<slug>.md 那份历史无痕换掉。宁可停在这里。
    if new_path.exists() && new_path != old_path {
        return Err(Error::Conflict {
            message: format!(
                "目标已存在 {}，移动会覆盖那份历史。先给其中一份换名/换 slug，再推进。",
                srel(store, &new_path).display()
            ),
        });
    }
    // content-hash 在 render 时更新为新正文快照（§13.2 第 2 层）。
    let _ = content_hash(&doc.body);
    doc.write(&new_path)?;
    if old_path != new_path && old_path.exists() {
        std::fs::remove_file(old_path)?;
    }
    Ok(new_path)
}

/// 追加一条（或一整段）markdown 列表项：坑、通知都走这里。
/// 与 `append_path` 的散文分支同源，走 `O_APPEND` 直写而非"整读→拼接→整写"（C27）——
/// 这两个文件同样是多 agent 共享的，RMW 会把别人刚追加的行覆盖掉。
fn append_to_markdown_file(path: &Path, line: &str) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    append_bytes(path, line)
}

fn clear_pending_entry(store: &Store, project: &str, slug: &str) {
    // 台账行首列就是 slug，按结构精确比对后删除。此前是"slug 加空格"的子串搜索：
    // 别人的理由里只要提过这个 slug，那一行就被连坐删掉（实测三条删剩一条），
    // 被删那条的反证义务从此无痕消失。
    let pend = store.project_dir(project).join("pending.md");
    let Ok(text) = std::fs::read_to_string(&pend) else {
        return;
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| match ledger_split(l) {
            Some((head, _)) => head != slug,
            None => true,
        })
        .collect();
    if kept.len() == text.lines().count() {
        return;
    }
    let next = if kept.is_empty() {
        String::new()
    } else {
        format!("{}\n", kept.join("\n"))
    };
    if let Err(e) = std::fs::write(&pend, next) {
        println!("⚠ 待测台账没清掉（{e}）。");
    }
}

/// 把 text 追加到某标题（如"决策"/"决策日志"）章节的末尾。
fn append_to_section(body: &str, heading: &str, text: &str) -> String {
    use crate::document::{heading_level, heading_matches};
    let lines: Vec<&str> = body.lines().collect();
    let idx = lines.iter().rposition(|l| heading_matches(l, heading));
    let text_nl = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    match idx {
        Some(start) => {
            let start_level = heading_level(lines[start]);
            let mut end = lines.len();
            for (i, l) in lines.iter().enumerate().skip(start + 1) {
                if l.starts_with('#') {
                    let lvl = heading_level(l);
                    if lvl <= start_level {
                        end = i;
                        break;
                    }
                }
            }
            // 去掉章节尾部空行后再插。
            let mut insert_at = end;
            while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            let mut out = lines[..insert_at].join("\n");
            out.push('\n');
            out.push_str(&text_nl);
            if insert_at < end {
                out.push_str(&lines[insert_at..end].join("\n"));
                out.push('\n');
            }
            if end < lines.len() {
                out.push_str(&lines[end..].join("\n"));
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            out
        }
        None => {
            // 无该章节：追加一个新章节到文末。
            let mut out = body.trim_end().to_string();
            out.push_str(&format!("\n\n## {heading}\n{text_nl}"));
            out
        }
    }
}

/// 在某标题章节末尾插入一整块（含标题，用于 ### 待测）。
fn insert_after_section(body: &str, heading: &str, block: &str) -> String {
    append_to_section(body, heading, block.trim_end())
}

/// 配额口径：**只数留痕行本身**（`- [<时间>] quick: … — <actor>`）。
/// 过去的口径是"含 `] quick:` 子串的行数"，于是出厂提案模板里那行示例注释先预占 1
/// （上限 5 实给 4），正文里任何散文/引述写过该子串也照占额度——而 CLI 无任何清零手段。
fn is_quick_ledger_line(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("<!--") {
        return false;
    }
    // `-` 与 `[` 之间的空白容忍：手工重排过的留痕仍算数（漏数等于放宽防护）。
    let after_dash = match t.strip_prefix('-') {
        Some(rest) => rest.trim_start(),
        None => return false,
    };
    after_dash.starts_with('[') && after_dash.contains("] quick:")
}

fn count_quick_lines(body: &str) -> usize {
    body.lines().filter(|l| is_quick_ledger_line(l)).count()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

pub fn project_name(store: &Store, flag: Option<&str>) -> Result<String> {
    resolve_project(store, flag)
}

// 让 Frontmatter 构造在需要时可复用（当前由模板渲染路径使用）。
#[allow(dead_code)]
fn _frontmatter_type_marker(_: Frontmatter) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：init 必须只提交自己落地的路径，绝不 `add -A` 卷进**其它项目**的脏改动。
    /// 事故出处：`init pixel-raider` 把遗留在 athena/pool/ 的无 frontmatter 残片
    /// 一并提交到 pixel-raider 名下（非路径隔离的 commit_all 副作用）。
    #[test]
    fn init_does_not_sweep_foreign_project_dirt() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-init-iso-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-init-iso-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };

        init(&store, "alpha", "AGENTS.md", &at, false, false).unwrap();
        // 模拟游离残片：init alpha 之后，在 alpha 池里写一个未跟踪、坏 frontmatter 的文件。
        let stray = root.join("projects/alpha/pool/stray-orphan.md");
        std::fs::write(&stray, "填入某提案真实证据\n").unwrap();

        // 初始化另一个项目——旧逻辑会把 alpha 的残片卷进 beta 的 init 提交。
        init(&store, "beta", "AGENTS.md", &at, false, false).unwrap();

        let out = std::process::Command::new("git")
            .args(["-C"])
            .arg(&root)
            .args(["show", "--name-only", "--pretty=format:", "HEAD"])
            .output()
            .expect("git show");
        let files = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            files.contains("projects/beta"),
            "beta 的 init 提交应包含自身骨架，实际：\n{files}"
        );
        assert!(
            !files.contains("projects/alpha"),
            "beta 的 init 提交不应卷走 alpha 的文件，实际：\n{files}"
        );
        assert!(
            !files.contains("stray-orphan"),
            "beta 的 init 提交不应卷走游离残片，实际：\n{files}"
        );
        // 残片应原样留在磁盘、仍未被跟踪（init 无权也不该动别的项目）。
        assert!(stray.exists(), "游离残片不应被 init 删除");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    #[test]
    fn route_state_rel_prefixes_bare_status_dirs_under_project() {
        for st in STATUSES {
            let (r, changed) = route_state_rel(&format!("{st}/x.md"), "demo").unwrap();
            assert!(changed, "{st}/ 裸路径应触发归位");
            assert_eq!(r, format!("projects/demo/{st}/x.md"));
        }
        // ./ 前缀也归位。
        let (r, changed) = route_state_rel("./pool/x.md", "demo").unwrap();
        assert!(changed);
        assert_eq!(r, "projects/demo/pool/x.md");
    }

    #[test]
    fn route_state_rel_leaves_qualified_and_nonstatus_paths() {
        // 已限定全路径、其它根目录：原样、不改位（向后兼容）。
        for rel in [
            "projects/demo/pool/x.md",
            "templates/AGENTS.md.tpl",
            "pitfalls/global.md",
            "notices.md",
            "projects/demo/pending.md",
        ] {
            let (r, changed) = route_state_rel(rel, "demo").unwrap();
            assert!(!changed, "{rel} 不应被归位");
            assert_eq!(r, rel);
        }
    }

    /// 回归（C28）：归位是词法首段比对，`" pool/x.md"` 此前绕过归位、把状态文件
    /// 写进状态根顶层一个叫 `" pool"` 的目录，且 rc=0、context/validate 一字不提。
    #[test]
    fn route_state_rel_rejects_whitespace_so_bare_dirs_cannot_escape_rerouting() {
        for rel in [" pool/x.md", "pool/x.md ", "\tpool/x.md"] {
            assert!(
                route_state_rel(rel, "demo").is_err(),
                "首尾空白的状态路径必须被拒，而不是静默不归位：{rel:?}"
            );
        }
        // 首段去掉尾随空白后才等于状态名：同样必须拒，不能默默落到顶层 `pool /`。
        assert!(route_state_rel("pool /x.md", "demo").is_err());
        // 与状态名无关的目录仍原样放行（不该被这道校验误伤）。
        assert!(route_state_rel("scratch/a.md", "demo").is_ok());
    }

    /// 回归（C20/C4）：slug 直接成为状态文件名。含 `..` 曾把 1.7 KB 文件写到状态根
    /// 之外（报错来自随后的 git add，坏文件不清理）；含 `/` 落进子目录后对
    /// context/validate 双双隐身（目录扫描只看一层）。
    #[test]
    fn validate_slug_rejects_traversal_separators_and_blanks() {
        for bad in [
            "",
            "..",
            "../../evil",
            "a/b",
            "a\\b",
            " pool",
            ".hidden",
            "-flag",
            "x.md",
            "中文 混空格",
        ] {
            assert!(validate_slug(bad).is_err(), "非法 slug 应被拒：{bad:?}");
        }
        for ok in ["plain", "a-b", "a_b", "a.v2", "带中文的slug", "12"] {
            assert!(validate_slug(ok).is_ok(), "合法 slug 不应被误伤：{ok:?}");
        }
    }

    /// 回归（C41/C5）：`--project ""` 的存在性检查曾被 `projects/` 顶层自身满足，
    /// 写动作随即落进 `projects/pool/…` 幻影树。空串与纯空白一律按未指定处理。
    #[test]
    fn resolve_project_treats_blank_flag_as_unset() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-blank-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-blank-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };

        for blank in ["", "   "] {
            assert!(
                resolve_project(&store, Some(blank)).is_err(),
                "空 project 不得被 `projects/` 顶层空洞放行"
            );
        }
        // 项目名本身也不得携带路径分隔符：否则整个项目的读写面指向别处。
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();
        for bad in ["../demo", "a/b", ".", "./", "demo/x"] {
            assert!(
                resolve_project(&store, Some(bad)).is_err(),
                "非法项目名应被拒：{bad:?}"
            );
        }
        // 含空白但去掉空白即合法的名字：按去空白后解析（不因传参习惯直接失败）。
        assert_eq!(resolve_project(&store, Some(" demo ")).unwrap(), "demo");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    /// 回归（C46/C5）：`write projects/<名>/…` 顺手建目录造出的半套项目，旧判据
    /// 报"无缺失项"exit 0，直到下一次 `new` 才 IO 错。写侧拒、读侧报 Error。
    #[test]
    fn write_rejects_unknown_project_and_validate_flags_half_skeleton() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-half-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-half-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();

        // 写入不存在的项目目录：必须拒，且不留下任何目录。
        let err = write_path(&store, "demo", "projects/ghost/readme.md", "x\n", false);
        assert!(err.is_err(), "write 不得顺手替人生成半套项目");
        assert!(
            !root.join("projects/ghost").exists(),
            "被拒的写入不应留下目录"
        );

        // 手工造半套树（模拟既有状态库），validate 必须报 Error 而非"自洽"。
        std::fs::create_dir_all(root.join("projects/halfonly/pool")).unwrap();
        std::fs::write(root.join("projects/halfonly/stray.md"), "y\n").unwrap();
        let gaps = project_skeleton_gaps(&store, "halfonly");
        assert!(
            gaps.contains(&"working/".to_string()) && gaps.contains(&"meta.md".to_string()),
            "半套项目的缺失项应被列出，实际：{gaps:?}"
        );
        assert!(project_skeleton_gaps(&store, "demo").is_empty(), "完整项目不应报缺失");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    #[test]
    fn write_path_reroutes_bare_status_path_into_project_and_validates_fm() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-wip-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-wip-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();

        // 裸 pool/ 路径 + 缺 frontmatter → 归位后按 item doc 校验拒绝，绝不落顶层。
        let bad = write_path(&store, "demo", "pool/bad.md", "no frontmatter\n", false);
        assert!(bad.is_err(), "归位后应触发一致 frontmatter 校验");
        assert!(
            !root.join("pool/bad.md").exists(),
            "顶层 pool/ 不应被污染（幻影树来源）"
        );

        // 裸 pool/ 路径 + 合法 frontmatter → 落到 projects/demo/pool/。
        let good = format!(
            "---\nslug: ok\nkind: proposal\nstatus: pool\nproject: demo\n\
             updated-by: pid-0\nupdated-at: 2026-09-22T00:00:00+08:00\ncontent-hash: sha256:{:064x}\n---\n\n# ok\n",
            0u64
        );
        write_path(&store, "demo", "pool/ok.md", &good, false).unwrap();
        assert!(
            root.join("projects/demo/pool/ok.md").exists(),
            "归位后应写入 projects/demo/pool/ok.md"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    #[test]
    fn strip_pitfall_tail_comment_removes_metadata() {
        assert_eq!(
            strip_pitfall_tail_comment("记个坑  <!-- 2026-09-24 · pid-1 -->"),
            "记个坑"
        );
        assert_eq!(strip_pitfall_tail_comment("无注释行"), "无注释行");
    }

    #[test]
    fn pitfall_search_matches_across_sources_readonly() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-psearch-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-psearch-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();
        init(&store, "other", "AGENTS.md", &at, false, false).unwrap();

        pitfall(&store, "demo", "push 会要密码，先不 push", true).unwrap(); // 全局
        pitfall(&store, "demo", "push 前须 rebase", false).unwrap(); // demo 项目
        pitfall(&store, "other", "与 push 无关的坑", false).unwrap(); // other 项目

        // 多词 AND：命中含 push 的三条
        pitfall_search(&store, "demo", "push").unwrap();
        // 空词条：短路返回 Ok，不落任何文件
        pitfall_search(&store, "demo", "   ").unwrap();

        // 纯读命令不应在状态根顶层制造幻影目录
        assert!(!root.join("pool").exists());
        assert!(!root.join("working").exists());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    #[test]
    fn resolve_project_rejects_ghost_project() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-rp-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-rp-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };

        // 未创建任何项目时，指向不存在项目的解析必须报错——否则从 ~ 运行会把
        // cwd 名/错拼当项目，静默造出幽灵项目（context 假空 + new 幻影树）。
        assert!(
            resolve_project(&store, Some("ghostproj")).is_err(),
            "不存在的项目必须被拒，不能静默放行"
        );

        // init 真实项目后，同名解析应成功。
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();
        assert_eq!(resolve_project(&store, Some("demo")).unwrap(), "demo");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    /// 回归（C3）：配额口径过去是"含 `] quick:` 子串的行数"——出厂模板那行示例注释
    /// 先预占 1（上限 5 实给 4），正文里引述/散文写过该子串也照占，且无从清零。
    #[test]
    fn quick_quota_counts_only_ledger_lines() {
        let body = "<!-- 快速通道（athena quick）的一行留痕由 CLI 自动追加在此 -->\n\
                    示例格式长这样：- [时间] quick: ... — <session>\n\
                    - [2026-01-01T00:00:00+08:00] quick: 真留痕 — pid-1\n\
                    <!-- - [x] quick: 注释里的不算 -->\n\
                    -   [2026-01-02T00:00:00+08:00] quick: 第二条 — pid-2\n";
        assert_eq!(count_quick_lines(body), 2, "只应数两条真留痕：{body:?}");
        // 缩进的留痕行仍是留痕。
        assert_eq!(count_quick_lines("  - [t] quick: x — a"), 1);
    }

    /// 回归（C33）：写 item doc 时 frontmatter 与落点三种漂移都能 rc=0 落盘，
    /// 事后 validate 只补标 status/slug，`project` 漂移无人报。
    #[test]
    fn write_checks_frontmatter_against_placement() {
        let good = batch_b_doc("body", false); // slug exp / project demo / status working
        assert!(check_item_doc_placement("projects/demo/working/exp.md", &good.fm).is_ok());
        // 三种漂移各拒一次，且文案点名是哪一处。
        let mut p = good.clone();
        p.fm.project = "other".into();
        let e = check_item_doc_placement("projects/demo/working/exp.md", &p.fm).unwrap_err();
        assert!(e.to_string().contains("project:"), "{e}");
        let mut s = good.clone();
        s.fm.status = "pool".into();
        assert!(check_item_doc_placement("projects/demo/working/exp.md", &s.fm).is_err());
        let mut n = good.clone();
        n.fm.slug = "别的".into();
        assert!(check_item_doc_placement("projects/demo/working/exp.md", &n.fm).is_err());
    }

    fn batch_b_doc(body: &str, pending: bool) -> Document {
        Document {
            fm: Frontmatter {
                slug: "exp".into(),
                kind: Kind::Proposal,
                status: "working".into(),
                project: "demo".into(),
                updated_by: "t".into(),
                updated_at: "t".into(),
                content_hash: "sha256:pending".into(),
                outcome: None,
                falsification: Some("pending".into()).filter(|_| pending),
                priority: None,
                extra: Vec::new(),
            },
            body: body.into(),
        }
    }

    fn batch_b_store(tag: &str) -> Store {
        let root = std::env::temp_dir().join(format!("athena-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("projects/demo")).unwrap();
        Store { root }
    }

    /// 回归（C31）：清理台账用的是"`slug` + 空格"子串搜索，别的条目只要**理由里提过
    /// 这个 slug** 就被连坐删掉（实测三条删剩一条），那条的反证义务无痕消失。
    #[test]
    fn clear_pending_entry_deletes_only_the_owning_line() {
        let store = batch_b_store("b31");
        let pend = store.project_dir("demo").join("pending.md");
        std::fs::write(
            &pend,
            "- [ ] exp · 跳过反证 · 理由：CI 无显示服务器 · 2026-01-01\n\
             - [ ] other · 跳过反证 · 理由：涉及 exp 的窗口，需复验 · 2026-01-01\n\
             - [ ] third · 跳过反证 · 理由：无关 · 2026-01-01\n",
        )
        .unwrap();

        clear_pending_entry(&store, "demo", "exp");

        let left = std::fs::read_to_string(&pend).unwrap();
        assert!(!left.contains("- [ ] exp ·"), "自己的行应被清掉：\n{left}");
        assert!(
            left.contains("- [ ] other ·") && left.contains("- [ ] third ·"),
            "提到该 slug 的别人条目不得被连坐删除：\n{left}"
        );
        let _ = std::fs::remove_dir_all(&store.root);
    }

    /// 回归（C16/C17/C45）：freeze/community 原先**静默**抹掉 frontmatter 的待测标记
    /// 并删掉台账行 —— 绕过却无痕。现在原理由搬进正文「决策」章节留痕，并回报绕过。
    #[test]
    fn status_actions_strike_pending_but_leave_an_audit_trail() {
        let store = batch_b_store("b16");
        let pend = store.project_dir("demo").join("pending.md");
        std::fs::write(&pend, "- [ ] exp · 跳过反证 · 理由：CI 无显示服务器 · 2026-01-01\n").unwrap();
        let mut doc = batch_b_doc(
            "## 反证实验\n### 待测\n- 理由：CI 无显示服务器\n- 状态：pending\n\n## 决策\n- 决定：先推进\n",
            true,
        );

        let struck = strike_pending_registration(&store, &mut doc, "demo", "exp", "freeze");

        assert!(struck, "确有登记时被抹掉必须回报，供调用方打印绕过提示");
        assert_eq!(doc.fm.falsification, None, "frontmatter 标记应被清除");
        assert!(
            !doc.body.contains("### 待测"),
            "待测小节应被移除：\n{}",
            doc.body
        );
        assert!(
            doc.body.contains("反证登记未清算即freeze") && doc.body.contains("原理由：CI 无显示服务器"),
            "原理由必须留在「决策」章节：\n{}",
            doc.body
        );
        assert!(
            doc.body.contains("- 决定：先推进"),
            "原有决策不得被覆盖：\n{}",
            doc.body
        );
        assert!(
            !std::fs::read_to_string(&pend).unwrap().contains("- [ ] exp ·"),
            "台账行应随之清掉"
        );
        let _ = std::fs::remove_dir_all(&store.root);
    }

    /// 回归（C44）：`resume` 只把文件挪回 working，登记早已被 freeze/community 抹净
    /// —— 于是"从未真跑过反证"的项带着查无登记的状态回来，complete 也不再拦它。
    /// 现在按正文留痕把登记恢复。
    #[test]
    fn resume_revives_a_registration_that_was_bypassed_not_settled() {
        let store = batch_b_store("b44");
        let pend = store.project_dir("demo").join("pending.md");
        std::fs::write(&pend, "- [ ] exp · 跳过反证 · 理由：CI 无显示服务器 · 2026-01-01\n").unwrap();
        let mut doc = batch_b_doc("## 反证实验\n### 待测\n- 状态：pending\n\n## 决策\n", true);
        strike_pending_registration(&store, &mut doc, "demo", "exp", "community");
        assert_eq!(doc.fm.falsification, None);

        let revived = revive_pending_registration(&store, &mut doc, "demo", "exp");

        assert!(revived, "有绕过留痕时必须恢复登记");
        assert_eq!(doc.fm.falsification.as_deref(), Some("pending"));
        let ledger = std::fs::read_to_string(&pend).unwrap();
        assert!(
            ledger.contains("- [ ] exp · 跳过反证 · 理由：CI 无显示服务器"),
            "台账应补回原理由：\n{ledger}"
        );
        // 已处于 pending 时不重复登记。
        assert!(!revive_pending_registration(&store, &mut doc, "demo", "exp"));
        assert_eq!(
            std::fs::read_to_string(&pend)
                .unwrap()
                .lines()
                .filter(|l| l.contains("- [ ] exp ·"))
                .count(),
            1,
            "重复调用不得堆叠台账行"
        );
        let _ = std::fs::remove_dir_all(&store.root);
    }

    /// 回归（C34）：`promote --skip-falsification` 先写文档与台账、再跑校验，被拒时
    /// 留下"已登记但未推进"的假登记（git status 见 M pending.md）。现在拆分：
    /// 只改内存文档，校验与挪动全过了才落台账。
    #[test]
    fn register_pending_touches_only_the_in_memory_doc() {
        let store = batch_b_store("b34");
        let pend = store.project_dir("demo").join("pending.md");
        let mut doc = batch_b_doc("## 反证实验\n", false);

        register_pending_in_doc(&mut doc, "CI 无显示服务器");

        assert_eq!(doc.fm.falsification.as_deref(), Some("pending"));
        assert!(doc.body.contains("### 待测"), "待测小节应插入：\n{}", doc.body);
        assert!(!pend.exists(), "登记阶段绝不提前写台账");
        // 台账写入是独立一步（挪动成功后才调用）。
        append_pending_ledger(&store, "demo", "exp", "CI 无显示服务器");
        assert!(std::fs::read_to_string(&pend)
            .unwrap()
            .contains("- [ ] exp · 跳过反证 · 理由：CI 无显示服务器"));
        let _ = std::fs::remove_dir_all(&store.root);
    }

    /// C23 + C39：init 补出台账文件；既有入口读不出时**整体拒绝**，不留半套项目。
    #[test]
    fn init_seeds_voice_and_refuses_unreadable_entry() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-f-voice-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-f-voice-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();
        // 人话台账从前由不创建，`context` 的台账段因此静默缺席。
        assert!(store.global_voice().is_file(), "init 须铺出全局 voice.md");
        assert!(
            store.project_voice("demo").is_file(),
            "init 须铺出项目 voice.md"
        );

        // 非 UTF-8 的既有入口：不加 --force 必须拒，且**不建任何骨架**。
        let _ = std::fs::remove_dir_all(&root);
        std::fs::write(at.join("AGENTS.md"), [0u8, 159, 146, 128]).unwrap();
        let err = init(&store, "demo", "AGENTS.md", &at, false, false);
        assert!(err.is_err(), "读不出的既有入口必须拒，不得静默覆盖");
        assert!(
            !root.join("projects/demo").exists(),
            "被拒的 init 不应留下半套项目：{err:?}",
        );
        // 显式 --force 才允许丢弃它。
        init(&store, "demo", "AGENTS.md", &at, true, false).unwrap();
        assert!(root.join("projects/demo/pool").is_dir());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    /// C22：状态根自身 `.git` 丢失后，写动作与 log 都必须拒绝——从前 git 向上借用宿主仓库，
    /// 状态提交落进别人的仓、log 打印宿主代码史，而命令照样 rc=0。
    #[test]
    fn missing_state_git_refuses_writes_and_log() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-f-nogit-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-f-nogit-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();
        std::fs::remove_dir_all(root.join(".git")).unwrap();

        let err = write_path(&store, "demo", "projects/demo/scratch.md", "x\n", false)
            .expect_err("失去 .git 的状态根不得再被写入（会寄生进宿主仓库）");
        assert!(
            format!("{err}").contains(".git"),
            "报错须点名缺失的 .git，实际：{err}"
        );
        assert!(
            std::fs::read_to_string(root.join("projects/demo/scratch.md")).is_err(),
            "拒绝必须发生在写盘之前"
        );
        assert!(show_log(&store, 5).is_err(), "log 不得打印宿主仓库历史");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    /// C10 + C40：notify 的文本与 --clear 互斥；多行正文折成一条 + 缩进续行一起渲染。
    #[test]
    fn notify_rejects_clear_with_text_and_folds_lines() {
        let tag = std::process::id();
        let root = std::env::temp_dir().join(format!("athena-f-notify-{tag}"));
        let at = std::env::temp_dir().join(format!("athena-f-notify-at-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).unwrap();
        let store = Store { root: root.clone() };
        init(&store, "demo", "AGENTS.md", &at, false, false).unwrap();

        let err = notify(&store, Some("要发的那条"), true).expect_err("同给必须拒");
        assert!(
            format!("{err}").contains("二选一"),
            "报错须要求二选一，实际：{err}"
        );
        assert!(
            !std::fs::read_to_string(store.notices_md())
                .unwrap()
                .contains("要发的那条"),
            "被拒的 notify 不得留下半条记录"
        );

        notify(&store, Some("第一行\n第二行细节"), false).unwrap();
        let raw = std::fs::read_to_string(store.notices_md()).unwrap();
        assert!(raw.contains("-  · 第一行") || raw.contains("· 第一行"), "{raw}");
        assert!(raw.contains("\n  第二行细节"), "续行须缩进两格：{raw}");
        let section = context::notices_section(&store).expect("应有通知段");
        assert!(
            section.contains("第二行细节"),
            "多行正文必须能在顶部被看到（否则'顶部可见'是假话）：{section}"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&at);
    }

    /// C9：搜索只认坑条目——标题、散文与 init 播种的占位示例都不算命中。
    #[test]
    fn pitfall_search_matches_only_real_entries() {
        assert!(is_pitfall_entry("- 真实踩到的一次坑"));
        assert!(is_pitfall_entry("  * 缩进的列表项也算"));
        assert!(
            !is_pitfall_entry("# 全局被坑（跨项目通用）"),
            "标题命中就是凭空报出一条不存在的坑"
        );
        assert!(!is_pitfall_entry("- （每条对应一次真实踩坑）"), "占位示例不是记录");
        assert!(!is_pitfall_entry("这段散文里含关键词"));
        assert!(!is_pitfall_entry(""));
    }
}
