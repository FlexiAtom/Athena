# Athena 术语表（机读 · 驱动校验与行为，§2.1b）
# 改这里 = 改协议规则，热加载生效、无需重编译。
# 建议用 `athena term new/edit` 维护，避免手动改动导致结构错误。
#
# 合并语义（C37）：内置默认（= 本模板内容）是**底**，你写的这份文件**按表名覆盖**同名表、
# 新表追加。删掉文件 = 全用内置默认；只写自己一条术语时其余内置术语照常生效（从前是整体
# 取代，手写一份短文件就把 require_fields 驱动的机器覆盖静默清零）。粒度是**整表**：写了
# [term.prune] 就是那一整张表说了算，未写的字段不会从内置补回来——这种"覆盖了但覆盖没了"
# 由 `athena term validate` 点名，不会无声通过。

quick_limit = 5          # 同一 slug 累计 quick 次数上限，超过强制完整剪枝（§5.4）

[term.pool]
slug = "pool"
is_entry = true
definition = "所有新想法的唯一入口；不在池里的东西不存在。"

[term.prune]
slug = "prune"
synonyms = ["自审", "裁枝", "pruning"]
# require_fields 直接列出正文标题子串，validate 按标题存在性校验（改术语即改校验）。
require_fields = ["自审裁枝", "原理实机验证", "反证实验", "更优雅", "成本对账"]
enforce_on = ["promote", "complete"]

[term.falsification]
slug = "falsification"
synonyms = ["反证", "证伪", "实证", "实机验证"]
is_recorded = true        # 反证必须留痕（可关闭的是门禁，不是记录义务）
# 反证模式开关**不在这里**（C35）：唯一真值是 config.toml 的 [behavior].falsification_mode。
# 从前两处都有同名键、config 压死 terms，而 config 语法坏时被静默吞掉回落到这里——
# 生效值随"哪份坏了"翻转。残留的 `mode = "..."` 键不会生效，`term validate` 会点名让你删。
skip_field = "skip_reason"
# 明文契约：反证表必须含以下"结果标签"列头之一，且其下至少一格非空。
# validate 按标签定位列（不认列序号），改这里即改规则（§6b/§11 解析鲁棒性）。
evidence_tokens = ["真实结果", "实测结果", "实际结果"]

[term.cost_review]
slug = "cost_review"
synonyms = ["成本对账"]
definition = "发现更优路径时量化比较，拒绝理由须可定位。沉没成本不作为留任理由。"

[term.gate]
slug = "gate"
origin = "fidus"
definition = "能力探测门：按能力而非平台身份决定可用性。"

[term.zero_trust]
slug = "zero_trust"
origin = "fidus"
definition = "不信任未经实机验证的结论，也不信任未跑测试的完成声明。"
