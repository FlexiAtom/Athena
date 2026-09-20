use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// GitBackend：状态库既是协议存档也是审计层（§1.2）。
///
/// 每次 Athena 写操作自动 `git add -A && git commit`，获得免费历史/回溯/时间戳。
/// git 只做审计与跨机传承载体，不做并发控制（§13.6）。
pub struct Git;

const AUTHOR_NAME: &str = "Athena";
const AUTHOR_EMAIL: &str = "athena@local";

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", &format!("user.name={AUTHOR_NAME}")])
        .args(["-c", &format!("user.email={AUTHOR_EMAIL}")])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .map_err(|e| Error::Git {
            message: format!("无法执行 git: {e}"),
        })?;
    Ok(out)
}

fn check(out: &std::process::Output, what: &str) -> Result<()> {
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Git {
            message: format!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim()),
        })
    }
}

impl Git {
    pub fn init(root: &Path) -> Result<()> {
        if root.join(".git").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(root)?;
        check(&git(root, &["init", "-q"])?, "git init")
    }

    /// 暂存全部并提交（若无变更则跳过）。actor 记入 commit message 尾部。
    pub fn commit_all(root: &Path, summary: &str, actor: &str) -> Result<()> {
        check(&git(root, &["add", "-A"])?, "git add")?;
        // 无暂存变更时 diff --cached --quiet 返回 0，跳过提交避免空 commit。
        let dirty = git(root, &["diff", "--cached", "--quiet"])?.status.code() == Some(1);
        if !dirty {
            return Ok(());
        }
        let msg = format!("{summary}\n\nathena-actor: {actor}");
        check(&git(root, &["commit", "-q", "-m", &msg])?, "git commit")
    }

    /// 只暂存**指定路径**（相对 root）并提交——精确归因，避免 `add -A` 扫入无关挂起改动。
    /// paths 为空则退化为 commit_all（兜底，如 init 的骨架批量落地）。
    pub fn commit_paths(
        root: &Path,
        paths: &[PathBuf],
        summary: &str,
        actor: &str,
    ) -> Result<()> {
        if paths.is_empty() {
            return Self::commit_all(root, summary, actor);
        }
        let mut args: Vec<String> = vec!["add".into(), "-A".into(), "--".into()];
        for p in paths {
            args.push(p.to_string_lossy().into_owned());
        }
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        check(&git(root, &refs)?, "git add")?;
        let dirty = git(root, &["diff", "--cached", "--quiet"])?.status.code() == Some(1);
        if !dirty {
            return Ok(());
        }
        let msg = format!("{summary}\n\nathena-actor: {actor}");
        check(&git(root, &["commit", "-q", "-m", &msg])?, "git commit")
    }

    pub fn log(root: &Path, n: usize) -> Result<Vec<(String, String)>> {
        let out = git(root, &["log", &format!("-{n}"), "--pretty=format:%h\x1f%s"])?;
        check(&out, "git log")?;
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        let mut rows = Vec::new();
        for line in s.lines() {
            let mut it = line.split('\x1f');
            if let (Some(h), Some(subj)) = (it.next(), it.next()) {
                rows.push((h.to_string(), subj.to_string()));
            }
        }
        Ok(rows)
    }
}
