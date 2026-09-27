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
2. `athena context` 拿项目状态（含**人话台账**：人的原话优先于你的推断，与台账冲突就按禁忌 10 问）
3. 动手前 `athena validate` 看自检报告

> **项目名从哪来**：`--project <p>` > config `default_project` > **当前目录名**，且解析结果须已 `init` 过。仓库目录名 ≠ 项目名时，上面两步（以及所有命令）都要显式带 `--project <p>`；照报错里那句"新项目先 `athena init <目录名>`"去做，会造出一个没人用的幽灵项目——那不是修复，是污染。

## 命令索引
```
athena init <project>        实例化状态骨架 + 把**当前生效的**入口模板（状态根 overlay 优先于二进制内置）复制到当前目录；目标已有内容不同的 AGENTS.md 会**整体拒绝**（退出码 1、连骨架都不建）：`--force` 覆盖 / `--no-agents` 只建骨架 / `--at` 换落地目录
athena new <slug>            用提案模板在 pool/ 创建（项目内唯一）；`--kind proposal|draft|plan` 可在建立时直接落轴一
athena deepen <slug> --to <kind>   轴一：kind = proposal|draft|plan（改 kind，文件不动；**单向不可降级**，回退只能改文件）
athena promote <slug>        轴二：pool→working / community→working（不带 from/to；working→finished 用 complete）
athena quick <slug> -c "…" [--promote]   小修复/配置更改：一行留痕，不走完整剪枝（**限非 pool 项**，同一 slug 累计有上限；`--promote` 留痕后顺带推一格）
athena complete <slug>       进行中→结束(done)；pending 反证是**硬拦截**（误判可 `resume` 撤回）
athena freeze <slug> --reason "…"   非 finished→结束(frozen)，毙掉须写原因（此命令无 -c 短选项）；已 finished 要改判须先 resume
athena resume <slug>         finished(done|frozen)→working（complete 与 freeze 的共同逆操作，要求重新剪枝）
athena community <slug>      放进 community/ 请人帮忙
athena write|append <path>   相对 ~/.Athena 的读写（裸状态目录路径 pool/working/… 依 --project 归位到 projects/<p>/…；写顶层请用显式全路径）
athena pitfall "…" [--global]   记坑（项目级/全局）；`--search "<词条>"` 只读跨源搜索——**二者互斥**，同给时 `--global` 被静默忽略
athena notify "…" | --clear  全局广播通知（写 ~/.Athena/notices.md，各项目 context/validate 顶部可见）；`--clear` 清空的是**跨项目整块广播板**，多 agent 共享下请逐行手删
athena context               输出 AI 上下文（协议摘要+树+术语+坑+人话台账）
athena validate              自检报告（标红问题，不替你决定；**不读仓库根入口**，故看不见本文件的副本漂移）
athena term list|new|validate   术语接口（**无 show**；validate 只校 toml 语法）
athena log                   状态库的 git 提交流水（协议动作史，**不是**项目代码史）
athena onerror               AI 故障处置：源码位置 + /tmp 报告 + 处置原则
```

> **你手上这份入口从哪来、怎么升级**：入口文本五处落点——① 源码仓 `AGENTS.md`（人读的）② `templates/AGENTS.md.tpl`（须与 ① 同文）③ 编译时内置进二进制的副本 ④ 状态根 overlay `~/.Athena/templates/AGENTS.md.tpl`（**运行时优先级最高**）⑤ 各项目根、你正在读的这份。`init` 复制的是 **④ 的当前内容**（overlay 存在就压过内置），且**从不覆盖已有文件**。所以 Athena 升级不会自动触达你：须先 `athena write templates/AGENTS.md.tpl --stdin` 刷新 ④，再 `athena init <p> --force` 重发 ⑤。`validate` 不读仓库根入口，没有任何机制会替你发现 ⑤ 已过期——这一步是你自己的义务。

## 出故障时（CLI 自身报错 / 数据损坏）
跑 `athena onerror` 打印处置手册。要点：你对 `~/.Athena` 有读写权，可自行修坏文档；
代码问题只读审阅源码仓（勿改运行中的二进制）；临时报告放 `/tmp/`；修好后 `athena pitfall` 记根因。

## 禁忌（协议门禁层：违反 → validate 标红提示，不拦截）
> 两条边界，别把"不设防"读过头：① 机器覆盖只到禁忌 2/3/4——其余几条违反后 validate **一字不提**，所以"没被标红"不等于合规；② 少数几处装了**真闸**，撞到就直接失败：`complete` 遇未清算 pending、`init` 遇内容不同的既有入口、路径越出状态根、slug 重名。失败是设计如此，不是状态库坏了（处置见 `athena onerror`）。
1. 不进池就开始写代码
2. 不剪枝就想 promote
3. 说"可行"必须贴**真实输出**；无实测用 `promote --skip-falsification="<具象理由>"` 登记待测
4. 发现更优方案必须做**成本对账**，维持现状要写可定位的拒绝理由
5. 不在状态根之外写状态文件（位置红线）——这一层**真拦**：`..` 与绝对路径越出 `~/.Athena` 会被 CLI 拒绝。但拦不住的仍在：**跨项目**写入（`projects/<别人的项目>/…`）与带路径分隔符的 slug 目前会照常落盘，validate 也不报——认清路径是你自己的义务，不是工具的。
6. **收口前须做全量审查**三问：是否仍有更优雅替代？是否仍有可继续剪掉的不合理设计？是否仍有逻辑问题？**未发现须明写"未发现"**；"有条件通过"必须列出全部条件，不接受"总体通过"
7. **审阅 ≠ 批准 ≠ 授权**：阅读通过 ≠ 文档批准，文档批准 ≠ 具体动作授权（含真实 mutation、破坏性实验、签名、发布、push）——AI 不得合并这三档，任何"下一步"须由调用方分档给出
8. **"继续"** 仅在 Agent 因意外中断后是合法恢复指令；其他语境须由调用方给出明确动作，不得把"继续"自行解释为实现、授权或阶段推进
9. **冻结须说明冻结对象**：设计 / 执行 / 结论——三种含义不同，不得只写"已冻结"
10. **拿不准必须问，不得自行解释后继续**：仅三种情况须停下要人明确——① 动作不可逆或对外（push / 签名 / 发布 / 删数据）；② 与人话台账已有原话冲突或疑似过期；③ 同一指令的两种读法会产出**不同东西**。其余按「放开来用」自决并留痕。问清之后得到的原话**补记入台账**

## 人话台账（voice）
`~/.Athena/voice.md`（全局约定）与 `~/.Athena/projects/<p>/voice.md`（项目）是**人的原话逐字记录**，`athena context` 已内置输出，优先级高于 AI 的一切推断。
- **只记原话**：AI 的转述、概括、"我理解为 X" 一律不入台账（那是 AI 自认，会抹平禁忌 7 的人/AI 界线）；AI 的理解写在自己的文档与决策日志里。
- **随手落一行**：听到 指令 / 授权 / 裁决 / 长期约定 / 被否掉的提议 就记，格式 `- <时间> [类别] 「原话」 关于:<slug|->`。写侧用 `athena append projects/<p>/voice.md --stdin`（**禁整写覆盖**，状态根多 agent 共享）；`[定]` 标只由人给，AI 不得自加。
- 原话不可得时必须标 `注:转述`，不得伪装成逐字。

## 放开来用（准入门槛≈0）
- **入池零门槛**：任何想法 / 观察 / 待办 / 临时念头都可以 `athena new <slug>` 落 pool。不必先想清楚、不必填反证、不必论证"值得做"——先落下来再说；不推就让它沉底，池不是债务。零门槛指**不必论证价值**，不是"必能落盘"：项目须已 `init`（见开场契约），slug 在项目内须唯一（重名直接拒）。
- **快速通道即记事本**：`athena quick <slug> -c "…"` 一行留痕，别当仪式。但它是这套 CLI 里少数**真会拒**的命令，两条门槛：pool 项不能 quick（池→working 必须完整剪枝），同一 slug 累计到 `quick_limit`（默认 5）就强制走剪枝。除此之外不必先问"我够格 quick 吗"。
- **可逆状态转换不是不可逆动作**：`promote` / `complete` / `freeze` / `community` / `resume` / 挪目录 都是**目录 git mv**，反向就是再挪一格（`resume` 既撤回 frozen 也撤回 done——误 complete 不必另立 slug）。**`deepen` 不在此列**：它原地改 `kind`、轴一**单向不可降级**，CLI 没有撤销手段，回退只能改文件或 `git revert`，所以别把它当无痕试错。禁忌 7 的"须明确授权"针对的是真实 mutation / 签名 / 发布 / push 这类**对外不可逆动作**——把每次状态推进都攒成请批，是把协议读成了镣铐，validate 拦不住但你会失去"随手用"的全部价值。红线只一条：**别把可逆的当不可逆来怕，也别把不可逆的当可逆来做**。
- **commit 属本地可逆，随手提交不必请批**：完成一个可验证的小步就提交（代码 / 文档 / 模板 / 测试同此），事后一句话报告即可。判据是"**怕的是没法回滚，不是怕误提交**"——误提交可 revert。攒着大批未提交改动、或每次提交都来要授权，同样是把协议读成镣铐。**push 不在此列**（对外不可逆，仍须明确授权）。

## 一句话立场
**Athena 不阻止，只记录与提示**（§1.3 不设防）。CLI 是脚手架不是镣铐；靠说服生效，不靠强制。若哪天要靠"拦住 AI"才能维持流程，那是协议文本没写好——去改协议，不是加固 CLI。

**"不设防"仅是流程层**，且**不等于"CLI 永不失败"**——「禁忌」节列的那几处真闸照常拦。不可逆 / 破坏性 / 对外动作（真实 mutation、签名、发布、push 等）仍受红线约束：须调用方**明确授权**（禁忌 7）；全量审查通过也不能替代授权。

> 完整规则、术语定义、pitfalls 按需 `athena context` 拉取，不在此常驻（防膨胀）。
