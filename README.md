# Athena

> 面向 AI Agent 的**工作协议 CLI** —— 以提示词工程为原理、由 AI 驱动的状态机。
> A prompt-engineering-based, AI-driven working-protocol state machine.

![license](https://img.shields.io/badge/license-AGPL--3.0-blue)

Athena 把"一个想法如何被推进到落地"固化成一套**可被 AI 读取、可审计、可跨机传承**的协议。状态根在 `~/.Athena`（一个 git 仓库），CLI 只是脚手架——**它不阻止，只记录与提示**。若哪天要靠"拦住 AI"才能维持流程，那是协议文本没写好，该改协议而不是加固 CLI。

完整设计见 [`athena.md`](./athena.md)；给 AI 读的协议入口见 [`AGENTS.md`](./AGENTS.md)。

## 核心模型：两条正交的轴

别把"它是什么"和"推到哪一步"搞混：

| 轴 | 含义 | 取值 | 怎么改 |
|---|---|---|---|
| **文档类型**（它是什么） | 写在文件 `kind` 里 | 提案 proposal → 草案 draft → 方案 plan | `deepen`（原地深化，文件不动） |
| **推进状态**（推到哪一步） | 由**目录**表达 | pool → working → finished / community | `promote`（挪目录，剪枝是门票） |

一个工作项 = 一个文件（frontmatter + Markdown 正文）。写深了走轴一，推进一步走轴二。

## 安装

```bash
cargo build --release
# 二进制在 target/release/athena
```

## 快速上手

```bash
athena init myproj          # 实例化状态骨架 + 复制协议入口文件到项目根
athena new idea-slug        # 用提案模板在 pool/ 创建
# ……写剪枝章节 + 反证留痕……
athena promote idea-slug    # 推进一格（validate 前置，剪枝/反证是门票）
athena context              # 拉取 AI 上下文（协议摘要 + 树 + 术语 + 坑）
athena validate             # 自检报告（标红问题，不替你决定）
```

## 命令一览

| 命令 | 作用 |
|---|---|
| `init <project>` | 实例化状态骨架 + 复制协议入口文件（`--agents-file` 改名，`--force` 覆盖，`--no-agents` 只建骨架） |
| `new <slug>` | 用提案模板在 `pool/` 创建 |
| `deepen <slug> --to draft\|plan` | 轴一：原地深化文档类型（单向，不可降级） |
| `promote <slug>` | 轴二：按当前位置推进一格（先 validate 再移动） |
| `complete <slug>` | 进行中 → 结束(done)；pending 反证须先清算 |
| `freeze <slug> -c "…"` | 任意 → 结束(frozen)：毙掉需写原因 |
| `community <slug>` | 放进 `community/` 请人帮忙 |
| `resume <slug>` | finished(frozen) → working（要求重新剪枝） |
| `quick <slug> -c "…"` | 小修复/配置更改：一行留痕，不走完整剪枝（快速通道） |
| `write\|append <path>` | 相对 `~/.Athena` 的路径白名单安全读写 |
| `pitfall "…" [--global]` | 记坑（项目级 / 全局） |
| `context` | 输出 AI 上下文 |
| `validate` | 自检报告（只报告不改文件） |
| `log` | git 历史 |
| `term list\|new\|show\|validate` | 术语接口（术语即配置：改 `terms.local.toml` 即改校验规则） |

## 反证：强制留痕，而非硬门禁

原理实机验证的核心是**主动证伪自己的前提**。Athena 把反证定为"强制留痕"：写出来即可，缺失只标红不阻塞 `promote`；但 `complete` 前 pending 必须清算（这是唯一保留的硬拦截）。无法实机时用 `promote --skip-falsification="<具象理由>"` 登记一条待清算项。

反证证据是**明文契约**：正文反证表里必须出现约定的"真实结果"列且有非空格——`validate` 按**表头标签**识别、不认列位置。允许哪些标签由 `terms.local.toml` 的 `[term.falsification].evidence_tokens` 决定。

## 状态根结构

```
~/.Athena/
├── config.toml            # 全局配置（反证模式、quick 上限、上下文预算…）
├── terms.local.toml       # 机读术语（含 require_fields / evidence_tokens）
├── terms.md               # 人读术语
├── pitfalls/global.md      # 全局坑
├── templates/             # 可覆盖的文档/术语/入口模板
└── projects/<name>/
    ├── pool/  working/  finished/  community/   # 目录即状态
    └── pitfalls.md
```

`~/.Athena` 同时是一个 git 仓库：每次写操作**只提交该动作触达的文件**（精确审计归因），git 只做审计与跨机传承载体，不做并发控制。

## 许可

Copyright © 2026 FlexiAtom

本项目以 AGPL-3.0-or-later 授权，详见 [`LICENSE`](./LICENSE)。
