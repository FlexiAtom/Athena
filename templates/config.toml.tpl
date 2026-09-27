# Athena 配置（§1）
# 状态库根固定为 ~/.Athena（可用环境变量 ATHENA_HOME 覆盖，主要供测试）。

# 当前默认项目；命令未指定 --project 时使用。留空则需在项目根或命令行显式指定。
default_project = ""

# Athena 源码仓库在本机的位置，供 `athena onerror` 打印（不烤进二进制，换机各自配）。
# 也可用环境变量 ATHENA_SOURCE 覆盖。留空则 onerror 会提示如何设置。
source_repo = ""

[behavior]
# 反证留痕缺失时的模式，取值**只认** warn（标红不阻塞）| block（拒绝推进）；拼错的值当场报错、
# 受闸动作不执行（C35/C7）。这里是**唯一真值**：terms.local.toml 的 [term.falsification].mode
# 已废弃，不再参与覆盖，也不会因某份文件坏了而翻转生效值。本文件语法坏同样直接报错（不回落）。
# 项目级覆盖（projects/<p>/terms.local.toml）从未实现，也不在计划里——要按项目分档就换 ATHENA_HOME。
falsification_mode = "warn"
# 入口文件膨胀预算（§1.1）：`validate` 量**生效模板**（~/.Athena/templates/AGENTS.md.tpl）与
# **当前目录的 AGENTS.md** 两份行数，达到该值即 ⚠（C2/C26：从前这项是死码，入口 83→100 行无人报警）。
# 出厂内置入口实测 100 行，故默认 110 = 再加 10 行就提醒你去剪枝而不是加行。
max_entry_lines = 110
