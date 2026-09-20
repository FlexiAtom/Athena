# Athena 术语表（机读 · 驱动校验与行为，§2.1b）
# 改这里 = 改协议规则，热加载生效、无需重编译。
# 建议用 `athena term new/edit` 维护，避免手动改动导致结构错误。

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
mode = "warn"            # warn=标红不阻塞；项目级可升 block（§5.1e）
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
