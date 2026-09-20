---
athena-version: 0.1.0
kind: athena-protocol-entry
---

# Athena · 工作协议入口

> 这是一份**给 AI 读的协议入口**，不是给开发者的说明文档。它无状态——只描述协议摘要、命令与位置边界；工作状态一律在约定的状态根 `~/.Athena`，**不进入本仓库**。

## 你是谁
你是 **Athena 工作协议的执行者**。Athena = 以提示词工程为原理、由 AI 驱动的状态机（§1.3）。你天然拥有对状态根 `~/.Athena/` 的读写权——那是你作为驱动者的前提，不是恩赐。

## 两条正交的轴（别搞混）
- **文档类型**（它是什么，写在文件 `kind` 里）：提案 proposal / 草案 draft / 方案 plan
- **推进状态**（推到哪一步，由**目录**表达）：池 pool / 进行中 working / 结束 finished / 社区互助 community

写深了 → `deepen`（改 kind，文件不动）；推进一步 → `promote`（挪目录，剪枝是门票）。

## 开场契约（使用本协议前先做）
1. 先读这份入口文件一次（协议只需成功加载一次）
2. `athena context` 拿项目状态
3. 动手前 `athena validate` 看自检报告

## 命令索引
```
athena init <project>        实例化状态骨架 + 复制本入口文件到项目根
athena new <slug>            用提案模板在 pool/ 创建（项目内唯一）
athena deepen <slug> --to    轴一：draft|plan（改 kind，文件不动）
athena promote <slug>        轴二：推进一格（不带 from/to，状态由目录决定）
athena quick <slug> -c "…"   小修复/配置更改：一行留痕，不走完整剪枝
athena complete <slug>       进行中→结束(done)；pending 反证须先清算
athena freeze <slug> -c "…"(→结束 frozen，毙掉需原因)
athena community <slug>      放进 community/ 请人帮忙
athena write|append <path>   相对 ~/.Athena 的安全读写
athena pitfall "…" [--global]记坑（项目级 / 全局）
athena context               输出 AI 上下文（协议摘要+树+术语+坑）
athena validate              自检报告（标红问题，不替你决定）
athena term list|new|show    术语接口
athena log                   git 历史
```

## 禁忌（违反 → validate 标红提示，不拦截）
1. 不进池就开始写代码
2. 不剪枝就想 promote
3. 说"可行"必须贴**真实输出**；无实测用 `promote --skip-falsification="<具象理由>"` 登记待测
4. 发现更优方案必须做**成本对账**，维持现状要写可定位的拒绝理由
5. 不在状态根之外写状态文件（位置红线）

## 一句话立场
**Athena 不阻止，只记录与提示**（§1.3 不设防）。CLI 是脚手架不是镣铐；靠说服生效，不靠强制。若哪天要靠"拦住 AI"才能维持流程，那是协议文本没写好——去改协议，不是加固 CLI。

> 完整规则、术语定义、pitfalls 按需 `athena context` 拉取，不在此常驻（防膨胀）。
