# Athena 配置（§1）
# 状态库根固定为 ~/.Athena（可用环境变量 ATHENA_HOME 覆盖，主要供测试）。

# 当前默认项目；命令未指定 --project 时使用。留空则需在项目根或命令行显式指定。
default_project = ""

[behavior]
# 反证缺失时的模式：warn（标红不阻塞）| block（拒绝 promote）。项目级可覆盖 terms.local.toml。
falsification_mode = "warn"
# 入口文件膨胀预算（§1.1）：超过则 validate 标黄提示精简/外置。
max_entry_lines = 150
max_entry_tokens = 2000
