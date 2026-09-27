# Athena 配置（§1）
# 状态库根固定为 ~/.Athena（可用环境变量 ATHENA_HOME 覆盖，主要供测试）。

# 当前默认项目；命令未指定 --project 时使用。留空则需在项目根或命令行显式指定。
default_project = ""

# Athena 源码仓库在本机的位置，供 `athena onerror` 打印（不烤进二进制，换机各自配）。
# 也可用环境变量 ATHENA_SOURCE 覆盖。留空则 onerror 会提示如何设置。
source_repo = ""

[behavior]
# 反证缺失时的模式：warn（标红不阻塞）| block（拒绝 promote）。
# ※ 此项**优先于** terms.local.toml 的 [term.falsification].mode；"项目级可覆盖"尚未实现（不存在 projects/<p>/terms.local.toml 解析）。
falsification_mode = "warn"
# 入口文件膨胀预算（§1.1）：**当前未生效**——仓库根入口不在 ~/.Athena 内，validate 的这项告警是死码（待 `agents check`，§1.1 实现进度）。
max_entry_lines = 150
max_entry_tokens = 2000
