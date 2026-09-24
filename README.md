# Athena

> 面向 AI Agent 的**工作协议 CLI** —— 以提示词工程为原理、由 AI 驱动的状态机。
> A prompt-engineering-based, AI-driven working-protocol state machine.

![license](https://img.shields.io/badge/license-AGPL--3.0-blue)

Athena 由 AI 驱动。**作为人，你只需两步**：编译它、在每个项目里跑一次 `init`；此后协议的一切推进都由 AI 读取项目根的 [`AGENTS.md`](./AGENTS.md) 来执行。命令一览、两条轴、反证留痕、状态根结构等细节都写在 `AGENTS.md`（给 AI 读的入口）和 [`athena.md`](./athena.md)（完整设计）里，本 README 不重复。

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
