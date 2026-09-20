use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::context;
use crate::document::{content_hash, Document, Frontmatter, Kind, Outcome};
use crate::error::{Error, Result};
use crate::items::{find_item, list_items, Item};
use crate::rules::{self, Finding, Level, ValidationCtx};
use crate::store::{Store, STATUSES};
use crate::templates;
use crate::term::TermsRegistry;
use crate::vcs::Git;

const PROTO_VERSION: &str = "0.1.0";

// ============================================================================
// 项目 / actor 解析
// ============================================================================

fn resolve_project(store: &Store, flag: Option<&str>) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p.to_string());
    }
    let cfg = Config::load(store)?;
    if let Some(dp) = cfg.default_project.filter(|s| !s.is_empty()) {
        return Ok(dp);
    }
    // 回退到当前目录名（常见：在项目仓库里执行）。
    let cwd = std::env::current_dir()?;
    cwd.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::Transition {
            message: "无法确定项目：请用 --project 指定，或先 `athena init`".into(),
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
    if store.root.join("projects").exists() {
        Ok(())
    } else {
        Err(Error::Transition {
            message: format!(
                "状态库尚未初始化（{} 不存在）。先运行 `athena init <project>`。",
                store.root.display()
            ),
        })
    }
}

// ============================================================================
// init
// ============================================================================

pub fn init(
    store: &Store,
    project: &str,
    agents_file: &str,
    at: &Path,
    force: bool,
    no_agents: bool,
) -> Result<()> {
    // 防呆（§1.1 安装语义）：先判入口文件，避免"半初始化 + 静默覆盖"。
    // 已存在且内容不同 → 除非 --force 否则整体拒绝，且不产生任何状态（原子、无副作用）。
    let rendered = render_entry(store, PROTO_VERSION);
    let target = at.join(agents_file);
    let existing = std::fs::read_to_string(&target).ok();
    let write_entry = match &existing {
        None => !no_agents,
        Some(cur) if *cur == rendered => false, // 幂等：已是最新，不重写
        Some(_) if no_agents => false,          // 存在且不同，但显式不动仓库
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
        &templates::load(store, "config.toml.tpl"),
    )?;
    write_if_absent(&store.terms_md(), &templates::load(store, "terms.md.tpl"))?;
    write_if_absent(
        &store.terms_toml(),
        &templates::load(store, "terms.local.toml.tpl"),
    )?;
    write_if_absent(
        &store.global_pitfalls(),
        "# 全局被坑（跨项目通用）\n\
         \n\
         - 原理可行 ≠ 原理已被实证；没跑反证实验的提案不离开池（§5.1a）。\n\
         - 在 Niri 上靠平台 API 定位自己：空值 + 恒返回 (0,0)，success 为真（fidus 教训，§5.1b）。\n\
         - AI 声称完成但没跑测试 → 必须实测结果才能 complete。\n",
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

    Git::commit_all(&store.root, &format!("init: 项目 {project} 骨架"), &actor())?;
    println!(
        "已初始化 ~/.Athena 与项目 `{project}`。\n\
         {entry_note}（复制非软链，可安全提交；不含任何工作状态）。\n\
         下一步：在项目里让 AI 读本入口，再 `athena context` / `athena validate`。"
    );
    Ok(())
}

fn render_entry(store: &Store, version: &str) -> String {
    let tpl = templates::load(store, "AGENTS.md.tpl");
    let mut vars = BTreeMap::new();
    vars.insert("version".to_string(), version.to_string());
    // 替换 frontmatter 中的版本占位（模板已含固定 version，这里兜底替换 athena-version 行）。
    let out = templates::render(&tpl, &vars);
    out.replace(
        "athena-version: 0.1.0",
        &format!("athena-version: {version}"),
    )
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

pub fn new_item(store: &Store, project: &str, slug: &str, kind: &str) -> Result<()> {
    require_initialized(store)?;
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
        println!("⚠ 存在同名历史项（finished），确认复用：新项仍将在 pool/ 创建。");
    }
    let tpl_name = templates::template_for_kind(kind);
    let mut vars = BTreeMap::new();
    vars.insert("slug".into(), slug.into());
    vars.insert("kind".into(), kind.into());
    vars.insert("status".into(), "pool".into());
    vars.insert("project".into(), project.into());
    vars.insert("actor".into(), actor());
    vars.insert("now".into(), templates::now_iso());
    let rendered = templates::render(&templates::load(store, tpl_name), &vars);
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

fn effective_falsification_mode(store: &Store, terms: &TermsRegistry) -> String {
    Config::load(store)
        .ok()
        .and_then(|c| c.behavior.falsification_mode)
        .unwrap_or_else(|| terms.falsification_mode().to_string())
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
    let mode = effective_falsification_mode(store, &terms);
    let items = list_items(store, project)?;
    let mut has_error = false;
    let mut total = 0usize;
    println!("athena validate · 项目 {project} · 反证模式={mode}");
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
    // 入口文件膨胀检查（§1.1）。
    if let Some(w) = entry_budget_warning(store, project) {
        println!("\n── 协议入口预算");
        println!("  ⚠ {w}");
    }
    if total == 0 {
        println!("✓ 无缺失项，状态库自洽。");
    } else {
        println!("\n共 {total} 条提示。这是自检报告：由你（AI）决定补不补，工具不代改（§9.1）。");
    }
    Ok(!has_error)
}

fn entry_budget_warning(store: &Store, _project: &str) -> Option<String> {
    let cfg = Config::load(store).ok()?;
    let max = cfg.behavior.max_entry_lines.unwrap_or(150);
    // 项目仓库根的 AGENTS.md 不在 ~/.Athena，无法在此读取；此项留待 `agents check`（Phase 2）。
    let _ = max;
    None
}

// ============================================================================
// promote / complete / freeze / community / resume / deepen / quick
// ============================================================================

pub fn promote(store: &Store, project: &str, slug: &str, skip: Option<&str>) -> Result<()> {
    require_initialized(store)?;
    let terms = TermsRegistry::load(&store.terms_toml())?;
    let mode = effective_falsification_mode(store, &terms);
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
        register_pending(store, &mut doc, project, slug, reason)?;
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

fn register_pending(
    store: &Store,
    doc: &mut Document,
    project: &str,
    slug: &str,
    reason: &str,
) -> Result<()> {
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
    // 追加到 pending.md（相对 ~/.Athena 的项目目录，§5.1f）。
    let pend = store.project_dir(project).join("pending.md");
    let line = format!(
        "- [ ] {slug} · 跳过反证 · 理由：{reason} · {}",
        templates::today()
    );
    append_to_markdown_file(&pend, &line)?;
    Ok(())
}

pub fn complete(store: &Store, project: &str, slug: &str) -> Result<()> {
    require_initialized(store)?;
    let it = need_item(store, project, slug)?;
    if it.status != "working" {
        return Err(Error::Transition {
            message: format!("complete 需从 working/ 出发，当前 {}", it.status),
        });
    }
    // 唯一保留的硬阻塞：pending 反证未清算（§5.1f / §6 rule3）。
    let has_pending = it.doc.fm.falsification.as_deref() == Some("pending")
        || it.doc.body.contains("[pending]")
        || it.doc.body.contains("### 待测");
    if has_pending {
        return Err(Error::Transition {
            message: format!(
                "{slug} 仍有未清算的 `pending` 反证 —— 实际执行并填真实结果、删除 ### 待测 段后再 complete。"
            ),
        });
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
    doc.fm.falsification = None;
    doc.body = append_to_section(&doc.body, "决策", &format!("- 冻结原因：{reason}"));
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
        &format!("freeze: {slug} → finished(frozen)"),
        &actor(),
    )?;
    println!("✓ {slug}: {} → finished (outcome: frozen)", it.status);
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
    let mut doc = it.doc.clone();
    doc.fm.falsification = None;
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "community")?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &src), srel(store, &new_path)],
        &format!("community: {slug} → community"),
        &actor(),
    )?;
    println!(
        "✓ {slug}: {} → community（放出去请人帮忙，供人搬运，非机器同步）",
        it.status
    );
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
    if !doc.has_heading("反证实验") {
        println!("⚠ 重新推进要求重新剪枝（不能无声复活，§5.2）—— 当前无反证章节，请补。");
    }
    let src = it.path.clone();
    let new_path = move_to_status(store, &mut doc, &it.path, project, slug, "working")?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &src), srel(store, &new_path)],
        &format!("resume: {slug} → working"),
        &actor(),
    )?;
    println!("✓ {slug}: finished → working（请重新完成剪枝）");
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
    let order = [Kind::Proposal, Kind::Draft, Kind::Plan];
    let from_i = order.iter().position(|k| *k == it.doc.fm.kind).unwrap_or(0);
    let to_i = order.iter().position(|k| *k == kind).unwrap();
    if to_i < from_i {
        return Err(Error::Transition {
            message: "不能反向降级文档类型（提案→草案→方案 单向深化）".into(),
        });
    }
    let mut doc = it.doc.clone();
    doc.fm.kind = kind;
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

pub fn write_path(store: &Store, rel: &str, content: &str) -> Result<()> {
    require_initialized(store)?;
    let path = store.resolve_rel(rel)?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&path, content)?;
    Git::commit_paths(
        &store.root,
        &[srel(store, &path)],
        &format!("write: {rel}"),
        &actor(),
    )?;
    println!("✓ 写入 {rel}");
    Ok(())
}

pub fn append_path(store: &Store, rel: &str, content: &str) -> Result<()> {
    require_initialized(store)?;
    let path = store.resolve_rel(rel)?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut cur = std::fs::read_to_string(&path).unwrap_or_default();
    if !cur.is_empty() && !cur.ends_with('\n') {
        cur.push('\n');
    }
    cur.push_str(content);
    std::fs::write(&path, cur)?;
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

pub fn term_list(store: &Store) -> Result<()> {
    let terms = TermsRegistry::load(&store.terms_toml())?;
    println!("术语（quick_limit={}）", terms.quick_limit);
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
    match TermsRegistry::load(&store.terms_toml()) {
        Ok(t) => {
            println!("✓ terms.local.toml 解析通过（{} 个术语）", t.terms.len());
            Ok(())
        }
        Err(e) => {
            println!("{e}");
            Err(e)
        }
    }
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
    doc.fm.updated_at = templates::now_iso();
    doc.fm.updated_by = actor();
    let new_path = store.status_dir(project, target).join(format!("{slug}.md"));
    // content-hash 在 render 时更新为新正文快照（§13.2 第 2 层）。
    let _ = content_hash(&doc.body);
    doc.write(&new_path)?;
    if old_path != new_path && old_path.exists() {
        std::fs::remove_file(old_path)?;
    }
    Ok(new_path)
}

fn append_to_markdown_file(path: &Path, line: &str) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut cur = std::fs::read_to_string(path).unwrap_or_default();
    if !cur.is_empty() && !cur.ends_with('\n') {
        cur.push('\n');
    }
    cur.push_str(line);
    if !line.ends_with('\n') {
        cur.push('\n');
    }
    std::fs::write(path, cur)?;
    Ok(())
}

fn clear_pending_entry(store: &Store, project: &str, slug: &str) {
    let pend = store.project_dir(project).join("pending.md");
    if let Ok(t) = std::fs::read_to_string(&pend) {
        let kept: Vec<&str> = t
            .lines()
            .filter(|l| !(l.contains("- [ ]") && l.contains(&format!("{slug} "))))
            .collect();
        let _ = std::fs::write(&pend, format!("{}\n", kept.join("\n")));
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

fn count_quick_lines(body: &str) -> usize {
    append_marker_count(body, "] quick:")
}

fn append_marker_count(body: &str, marker: &str) -> usize {
    body.lines().filter(|l| l.contains(marker)).count()
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
