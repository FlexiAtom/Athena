# Athena

> 面向 AI Agent 的**工作协议 CLI** —— 以提示词工程为原理、由 AI 驱动的状态机。
> A prompt-engineering-based, AI-driven working-protocol state machine.

![license](https://img.shields.io/badge/license-AGPL--3.0-blue)

Athena 由 AI 驱动。**作为人，你只需两步**：编译它、在每个项目里跑一次 `init`；此后协议的一切推进都由 AI 读取项目根的 [`AGENTS.md`](./AGENTS.md) 来执行。命令一览、反证留痕、状态根结构等细节都写在 `AGENTS.md`（给 AI 读的入口）和 [`athena.md`](./athena.md)（完整设计）里，本 README 不重复。唯一例外是下面"两条正交的轴"——它是理解整个模型的前提，人也要懂，故原样摘来放在此处。

## 核心模型：两条正交的轴

别把"它是什么"和"推到哪一步"搞混。以下两表原样摘自 [`athena.md`](./athena.md)：

**轴一 · 文档类型（内容层级，写在文档自己身上）**

| 词 | 定义 | 说明 |
|---|---|---|
| **提案** | 我有个想法 | 只要说清动机、目标、约束、为什么值得做 |
| **草案** | 我打个草稿，这东西怎么实现 | 回答"怎么做"——技术路径、模块划分、关键决策 |
| **方案** | 我可以按照这个方案实施 | 回答"照着做就能成"——可执行到步骤级、有验收标准 |

**轴二 · 推进状态（位置，由目录表达）**

| 词 | 定义 | 目录 |
|---|---|---|
| **池** | 未推进的提案 | `pool/` |
| **进行中** | 我正在推进的提案/草案/方案 | `working/` |
| **结束** | 已经完成 | `finished/`（`status: done`） |
| **冻结** | 已经毙了 | `finished/`（`status: frozen`） |
| **社区互助** | 字面意思：放出去请社区帮忙处理 | `community/` |

一句话：**目录回答"推到哪一步"，文档自己回答"它是什么"**——两者正交。一个工作项从提案写到方案是**在同一文件里改 `kind`**（轴一，不挪目录）；从池推到结束才是**移动文件**（轴二）。

## 编译

```bash
cargo build --release
# 二进制在 target/release/athena
```

## 初始化

```bash
athena init <project>
```

在状态根 `~/.Athena` 实例化项目骨架，并把协议入口 `AGENTS.md` 复制到当前目录。装好这一步，剩下的交给 AI。

## 许可

AGPL-3.0
