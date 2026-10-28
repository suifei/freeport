# 应用目录知识库逐项清单

调研日期：2026-10-24，2026-10-28 补充整目录规则。由 `scripts/build-knowledge.py` 与运行时 JSON 同源生成；本机调查结论和适用限制见 [调研报告](application-storage-research.md)。

`candidate` 表示有证据可进入迁移预检，仍须经过权限、链接、句柄、空间和事务检查；没有默认勾选。`review` 会列出但标注“需确认”，同样不勾选，用户须先核对说明；`exclude` 排除自动发现。`app` 类型表示整个应用目录，对应原批处理的整目录迁移；与其下叶目录同时勾选时只迁移外层目录。用户历史、模型和扩展均须保留，不能按可丢弃缓存处理。

`home` 路径相对于真实用户目录；`project` 路径相对于经标志文件确认的项目/应用根。`*` 只匹配单层目录。markers 外层是“或”，每组内部是“且”；仅检查存在和路径，绝不读取配置、令牌或会话正文。

环境变量列为官方/本机的重定位线索，本版不会运行应用或执行规则内命令，也尚未自动解析各应用配置文件/环境变量以重定位根。

## Claude Code

识别证据：`settings.json` + `projects`；或 `.credentials.json` + `projects`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.claude/projects` | 会话记录、项目记忆及工具输出；属于用户历史，完整保留，退出所有会话后迁移。 | history | documented | candidate |
| `.claude/file-history` | 文件修改前的恢复快照；用于回退，不能当作可丢弃缓存。 | history | documented | candidate |
| `.claude/paste-cache` | 历史粘贴内容；删除可能影响历史提示词，迁移时完整保留。 | history | documented | candidate |
| `.claude/cache` | 版本说明等可重新获取的应用缓存。 | cache | documented | candidate |
| `.claude/plugins` | 已安装插件、市场副本和插件状态；检测嵌套链接后按无链接分支处理。 | extensions | documented | candidate |
| `.claude/debug` | 诊断日志；本机 latest 是绝对文件符号链接，整目录需链接处理能力，不能忽略。 | logs | documented | candidate |
| `.claude/skills` | 共享技能经常指向 .agents/skills；保留连接，不整体搬运。 | extensions | documented | review |

重定位线索：`CLAUDE_CONFIG_DIR`。

资料：[官方资料](https://code.claude.com/docs/en/claude-directory)。

## OpenAI Codex

识别证据：`config.toml` + `sessions`；或 `auth.json` + `sessions`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.codex/sessions` | 可恢复会话的完整记录；用户历史，禁止按临时文件清理。 | history | local | candidate |
| `.codex/archived_sessions` | 归档会话，迁移保留内容及 ACL。 | history | local | candidate |
| `.codex/attachments` | 会话附件，可能仍被历史引用。 | history | local | candidate |
| `.codex/generated_images` | 已生成图片，属于用户产物。 | history | local | candidate |
| `.codex/cache` | 应用缓存，具体子项随版本变化；退出应用后处理。 | cache | local | candidate |
| `.codex/plugins/cache` | 插件下载/版本目录；本机 chrome/latest 是版本目录连接，允许选择其他无链接分支。 | extensions | local | candidate |
| `.codex/.sandbox` | 沙箱状态和权限关系，不作为缓存自动迁移。 | runtime | local | exclude |
| `.codex/.sandbox-bin` | 沙箱执行文件，可能关联主机权限和版本绑定，需专门验证。 | runtime | local | review |
| `.codex` | 整个 Codex 用户目录；含沙箱状态与权限关系（.sandbox 排除自动迁移），整目录迁移前需人工确认。 | app | local | review |
| `AppData/Local/OpenAI/Codex` | 桌面端运行时与程序副本；升级与版本绑定尚未验证，需人工确认。 | runtime | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## Codex managed runtime

识别证据：`codex-primary-runtime`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cache/codex-runtimes` | 本机包含 primary-runtime 和安装暂存，约 1.75 GiB；升级/版本绑定尚需验证，暂不自动迁移。 | runtime | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## Kimi Code

识别证据：`config.toml` + `sessions`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.kimi-code/sessions` | 会话正文、任务、计划与会话日志；完整保留，先退出所有 CLI/桌面后台。 | history | documented | candidate |
| `.kimi-code/logs` | 诊断日志。 | logs | documented | candidate |
| `.kimi-code/cache` | 本机存在的缓存；格式由应用版本决定。 | cache | local | candidate |
| `.kimi-code/files` | 本机附件文件，可能被历史引用。 | history | local | candidate |
| `.kimi-code/file-history` | 本机文件恢复快照；不是可丢弃缓存。 | history | local | candidate |
| `.kimi-code/search-index` | 本机搜索索引；应退出索引写入进程后处理。 | cache | local | candidate |
| `.kimi-code/server` | 桌面服务运行时，需验证升级器及后台服务停机行为。 | runtime | local | review |
| `.kimi-code` | 整个 Kimi Code 用户目录（含服务运行时与凭据）；先退出 CLI、桌面及后台服务，需人工确认。 | app | local | review |

重定位线索：`KIMI_CODE_HOME`。

资料：[官方资料](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/data-locations.html)；本机元数据观察（非厂商承诺）。

## DeepSeek Harness

识别证据：`profiles` + `sessions`；或 `.credentials.yaml` + `sessions`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.dsh/sessions` | CLI 与桌面共享的会话历史。 | history | local | candidate |
| `.dsh/attachments` | 会话附件。 | history | local | candidate |
| `.dsh/cache` | 本机应用缓存。 | cache | local | candidate |
| `.dsh/dsh-runtimes` | 本机存在；需按具体版本确认运行时、插件存储和数据库关系后纳入。 | mixed | local | review |
| `.dsh/storages` | 本机存在；需按具体版本确认运行时、插件存储和数据库关系后纳入。 | mixed | local | review |
| `.dsh/profiles` | 本机存在；需按具体版本确认运行时、插件存储和数据库关系后纳入。 | mixed | local | review |
| `.dsh` | 整个 DeepSeek Harness 用户目录（含运行时与存储）；先退出 CLI 与桌面，需人工确认。 | app | local | review |

重定位线索：`DSH_HOME`。

资料：[官方资料](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)；本机元数据观察（非厂商承诺）。

## Grok CLI

识别证据：`config.toml` + `sessions`；或 `auth.json` + `sessions`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.grok/sessions` | 会话正文和恢复元数据，属于用户历史。 | history | documented | candidate |
| `.grok/logs` | 本机诊断日志。 | logs | local | candidate |
| `.grok/downloads` | 本机下载暂存，先确认升级器未运行。 | cache | local | candidate |
| `.grok/marketplace-cache` | 本机插件市场缓存。 | extensions | local | candidate |
| `.grok/bin` | 实际 CLI 程序及升级入口，不能当缓存。 | runtime | local | review |
| `.grok` | 整个 Grok CLI 用户目录（含程序与升级入口）；先退出全部会话，需人工确认。 | app | local | review |

重定位线索：`GROK_HOME`。

资料：[官方资料](https://docs.x.ai/build/settings)；[官方资料](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/17-sessions.md)；本机元数据观察（非厂商承诺）。

## Grok Bot

识别证据：`.grokbot-data-root-v1` + `settings.json`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.grokbot/attachment-staging` | 附件暂存；公开文档未确认其可重建性，先保留人工复核。 | mixed | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://docs.x.ai/grok-bot/get-started)；本机元数据观察（非厂商承诺）。

## VS Code

识别证据：`Local State` + `Preferences` + `User/settings.json`；或 `Local State` + `Preferences` + `User/globalStorage`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Code` | 整个应用数据目录（设置、扩展状态、会话、缓存）；对应原批处理的整目录迁移。完全退出应用及后台后迁移。 | app | local | candidate |
| `AppData/Roaming/Code/Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Code/Code Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Code/GPUCache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Code/CachedData` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Code/CachedExtensionVSIXs` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Code/logs` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | logs | local | candidate |
| `.vscode` | 编辑器用户目录（扩展、argv.json、CLI 数据）；整目录迁移，完全退出编辑器及扩展宿主后进行。 | app | documented | candidate |
| `.vscode/extensions` | 已安装扩展，不是缓存；完全退出编辑器及扩展宿主后迁移。 | extensions | documented | candidate |
| `AppData/Local/Programs/Microsoft VS Code` | 用户级安装的程序目录；升级器按原路径写入，经连接后仍可用。退出编辑器和升级器后迁移。 | app | local | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://code.visualstudio.com/docs/setup/windows)；本机元数据观察（非厂商承诺）。

## Cursor

识别证据：`Local State` + `Preferences` + `User/settings.json`；或 `Local State` + `Preferences` + `User/globalStorage`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Cursor` | 整个应用数据目录（设置、扩展状态、会话、缓存）；对应原批处理的整目录迁移。完全退出应用及后台后迁移。 | app | local | candidate |
| `AppData/Roaming/Cursor/Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Cursor/Code Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Cursor/GPUCache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Cursor/CachedData` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Cursor/CachedExtensionVSIXs` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/Cursor/logs` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | logs | local | candidate |
| `.cursor` | 编辑器用户目录（扩展、argv.json、CLI 数据）；整目录迁移，完全退出编辑器及扩展宿主后进行。 | app | documented | candidate |
| `.cursor/extensions` | 已安装扩展，不是缓存；完全退出编辑器及扩展宿主后迁移。 | extensions | documented | candidate |
| `AppData/Local/Programs/cursor` | 用户级安装的程序目录；升级器按原路径写入，经连接后仍可用。退出编辑器和升级器后迁移。 | app | local | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://code.visualstudio.com/docs/setup/windows)；本机元数据观察（非厂商承诺）。

## Obsidian

识别证据：`Local State` + `Preferences` + `obsidian.json`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/obsidian` | 整个应用数据目录（设置、扩展状态、会话、缓存）；对应原批处理的整目录迁移。完全退出应用及后台后迁移。 | app | local | candidate |
| `AppData/Roaming/obsidian/Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/obsidian/Code Cache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/obsidian/GPUCache` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | cache | local | candidate |
| `AppData/Roaming/obsidian/logs` | 应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。 | logs | local | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://obsidian.md/help/data-storage)；本机元数据观察（非厂商承诺）。

## VS Code Insiders

识别证据：`extensions/extensions.json`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.vscode-insiders` | 编辑器用户目录（扩展、argv.json、CLI 数据）；整目录迁移，完全退出编辑器及扩展宿主后进行。 | app | documented | candidate |
| `.vscode-insiders/extensions` | 已安装扩展，不是缓存；完全退出编辑器及扩展宿主后迁移。 | extensions | documented | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://code.visualstudio.com/docs/configure/extensions/extension-marketplace)；本机元数据观察（非厂商承诺）。

## Obsidian vault

识别证据：`app.json` + `community-plugins.json`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.obsidian/plugins` | 插件代码及 data.json 设置，不是纯缓存；保留插件状态。 | extensions | documented | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://obsidian.md/help/configuration-folder)。

## Rust Cargo

识别证据：`registry` + `bin`；或 `registry` + `config.toml`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cargo/registry` | 下载依赖、索引及源码缓存；停止所有 cargo/rustc 后迁移，保留全目录关系。 | cache | documented | candidate |
| `.cargo/git` | 下载依赖、索引及源码缓存；停止所有 cargo/rustc 后迁移，保留全目录关系。 | cache | documented | candidate |
| `.cargo` | 整个 CARGO_HOME（bin、registry、git、配置）；PATH 中的 .cargo/bin 经连接后保持原路径。停止所有 cargo/rustc 后迁移。 | toolchain | documented | candidate |

重定位线索：`CARGO_HOME`。

资料：[官方资料](https://doc.rust-lang.org/cargo/guide/cargo-home.html)。

## Rust toolchains

识别证据：`settings.toml` + `toolchains`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.rustup/toolchains` | 已安装编译器/标准库，属于工具链，不是构建缓存。 | toolchain | documented | candidate |
| `.rustup` | 整个 RUSTUP_HOME（工具链、设置、下载）；停止 rustup、cargo、rustc 及 rust-analyzer 后迁移。 | toolchain | documented | candidate |
| `.rustup/downloads` | 工具链下载/解压暂存，需停止 rustup 更新。 | cache | documented | candidate |
| `.rustup/tmp` | 工具链下载/解压暂存，需停止 rustup 更新。 | cache | documented | candidate |

重定位线索：`RUSTUP_HOME`。

资料：[官方资料](https://rust-lang.github.io/rustup/environment-variables.html)。

## Cargo project

识别证据：`Cargo.toml`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `target` | 项目构建产物，须同时存在 Cargo.toml；自定义 target-dir 另行解析。 | cache | documented | candidate |

重定位线索：`CARGO_TARGET_DIR`。

资料：[官方资料](https://doc.rust-lang.org/cargo/reference/environment-variables.html)。

## Go build

识别证据：`README`；或 `trim.txt`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/go-build` | Go 编译和测试缓存；以 go env GOCACHE 的本机配置为优先线索。 | cache | documented | candidate |

重定位线索：`GOCACHE`。

资料：[官方资料](https://go.dev/cmd/go/#hdr-Build_and_test_caching)。

## Go modules

识别证据：`mod/cache/download`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `go/pkg/mod` | 模块缓存含只读源码、下载及校验元数据；不是整个 GOPATH。 | cache | documented | candidate |

重定位线索：`GOMODCACHE`、`GOPATH`。

资料：[官方资料](https://go.dev/ref/mod#module-cache)。

## vcpkg

识别证据：`archives`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/vcpkg/archives` | 已编译包的二进制归档缓存。 | cache | documented | candidate |

重定位线索：`VCPKG_DEFAULT_BINARY_CACHE`。

资料：[官方资料](https://learn.microsoft.com/en-us/vcpkg/users/binarycaching)。

## vcpkg checkout

识别证据：`.vcpkg-root` + `vcpkg.exe`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `downloads` | 下载/构建中间产物/暂存包；只在证实为 vcpkg 根后纳入。 | cache | documented | candidate |
| `buildtrees` | 下载/构建中间产物/暂存包；只在证实为 vcpkg 根后纳入。 | cache | documented | candidate |
| `packages` | 下载/构建中间产物/暂存包；只在证实为 vcpkg 根后纳入。 | cache | documented | candidate |

重定位线索：`VCPKG_ROOT`。

资料：[官方资料](https://learn.microsoft.com/en-us/vcpkg/users/config-environment)。

## VS Code C/C++

识别证据：`ipch`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Microsoft/vscode-cpptools` | IntelliSense/PCH 缓存（ipch 及各工作区数据库）；可通过 C_Cpp.intelliSenseCachePath 更改位置。 | cache | documented | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://code.visualstudio.com/docs/cpp/faq-cpp)。

## Python pip

识别证据：`http-v2`；或 `wheels`；或 `http`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/pip/Cache` | HTTP 响应与已构建 wheel 缓存；不是 Python 安装或虚拟环境。 | cache | documented | candidate |

重定位线索：`PIP_CACHE_DIR`。

资料：[官方资料](https://pip.pypa.io/en/stable/topics/caching/)。

## Node npm

识别证据：`_cacache`；或 `_npx`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/npm-cache` | 内容寻址下载缓存及 npx 暂存；不包含全局安装程序。 | cache | documented | candidate |
| `npm-cache` | 本机历史配置遗留的用户根缓存；用缓存结构识别。 | cache | documented | candidate |
| `AppData/Roaming/npm` | npm 全局安装的命令行工具及其 node_modules；PATH 指向原路径，经连接后不变。停止所有 node 进程后迁移。 | toolchain | documented | candidate |

重定位线索：`NPM_CONFIG_PREFIX`。

资料：[官方资料](https://docs.npmjs.com/cli/v11/configuring-npm/folders)；本机元数据观察（非厂商承诺）。

## Python uv

识别证据：`CACHEDIR.TAG`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/uv/cache` | Windows 默认使用硬链接；可能和环境共享数据。跨盘拆开共享关系不一定释放原盘空间，先专门复核。 | cache | documented | review |

重定位线索：`UV_CACHE_DIR`。

资料：[官方资料](https://docs.astral.sh/uv/concepts/cache/)。

## pnpm

识别证据：`v3`；或 `v10`；或 `v11`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/pnpm/store` | 内容寻址包仓库可能硬链接到项目，先确认边界及同盘限制。 | cache | documented | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://pnpm.io/settings#storedir)。

## Bun

识别证据：`bin/bun.exe` + `install/cache`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.bun/install/cache` | 包缓存可能与已安装包硬链接共享，先核验后迁移。 | cache | documented | review |

重定位线索：`BUN_INSTALL_CACHE_DIR`。

资料：[官方资料](https://bun.sh/docs/pm/global-cache)。

## Python install manager

识别证据：`bin` + `_cache`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Python` | Python 安装管理器的解释器与缓存；注册表与 PATH 仍指向原路径。虚拟环境可能硬编码解释器路径，需人工确认。 | toolchain | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://docs.python.org/3/using/windows.html)；本机元数据观察（非厂商承诺）。

## Playwright

识别证据：`.links`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/ms-playwright` | Playwright 下载的浏览器二进制，可重新下载。停止测试与浏览器进程后迁移。 | cache | documented | candidate |

重定位线索：`PLAYWRIGHT_BROWSERS_PATH`。

资料：[官方资料](https://playwright.dev/docs/browsers#managing-browser-binaries)。

## NuGet

识别证据：`packages`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.nuget/packages` | 全局展开包；构建可能直接引用其中内容，停止构建后迁移。 | cache | documented | candidate |

重定位线索：`NUGET_PACKAGES`。

资料：[官方资料](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders)。

## Gradle

识别证据：`caches` + `daemon`；或 `caches` + `wrapper`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.gradle/caches` | 依赖/构建缓存或 Gradle 分发包；先停止 Gradle daemon。 | cache | documented | candidate |
| `.gradle/wrapper/dists` | 依赖/构建缓存或 Gradle 分发包；先停止 Gradle daemon。 | cache | documented | candidate |

重定位线索：`GRADLE_USER_HOME`。

资料：[官方资料](https://docs.gradle.org/current/userguide/directory_layout.html)。

## Hugging Face

识别证据：`hub`；或 `xet`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cache/huggingface/hub` | 模型缓存必须保留 blobs/refs/snapshots 关系；可能有内部符号链接，不能仅搬某个大权重文件。 | models | documented | candidate |
| `.cache/huggingface/xet` | 模型缓存必须保留 blobs/refs/snapshots 关系；可能有内部符号链接，不能仅搬某个大权重文件。 | models | documented | candidate |

重定位线索：`HF_HOME`、`HF_HUB_CACHE`、`HF_XET_CACHE`。

资料：[官方资料](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)。

## ModelScope

识别证据：`hub`；或 `ast_indexer`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cache/modelscope` | 模型及下载元数据；不能把 .modelscope/credentials 当模型目录。 | models | documented | candidate |

重定位线索：`MODELSCOPE_CACHE`。

资料：[官方资料](https://github.com/modelscope/modelscope/blob/master/modelscope/hub/utils/utils.py)。

## PyTorch Hub

识别证据：`hub`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cache/torch/hub` | Hub 下载代码与 checkpoints；保留整体。 | models | documented | candidate |

重定位线索：`TORCH_HOME`。

资料：[官方资料](https://docs.pytorch.org/docs/stable/hub.html)。

## Ollama

识别证据：`models/blobs` + `models/manifests`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.ollama/models` | 共享权重 blobs 和 manifests 必须整体保留；停止模型服务和下载后迁移。 | models | documented | candidate |

重定位线索：`OLLAMA_MODELS`。

资料：[官方资料](https://docs.ollama.com/faq)。

## Hunyuan3D

识别证据：`checkpoints`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.cache/hy3dgen` | 本机模型目录；需确认对应工具和模型文件格式后启用，禁止仅按通用文件夹名匹配。 | models | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## rembg

识别证据：`u2net.onnx`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `.u2net` | 本机模型目录；需确认对应工具和模型文件格式后启用，禁止仅按通用文件夹名匹配。 | models | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## Google Chrome

识别证据：`Preferences`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Google/Chrome/User Data/*/Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Google/Chrome/User Data/*/Code Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Google/Chrome/User Data/*/GPUCache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Google/Chrome/User Data/*/Service Worker/CacheStorage` | 离线网站资源可能不可重新获取，不等同普通 HTTP 缓存。 | mixed | documented | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md)。

## Microsoft Edge

识别证据：`Preferences`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Microsoft/Edge/User Data/*/Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Microsoft/Edge/User Data/*/Code Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Microsoft/Edge/User Data/*/GPUCache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Microsoft/Edge/User Data/*/Service Worker/CacheStorage` | 离线网站资源可能不可重新获取，不等同普通 HTTP 缓存。 | mixed | documented | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md)。

## Brave

识别证据：`Preferences`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/BraveSoftware/Brave-Browser/User Data/*/Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/BraveSoftware/Brave-Browser/User Data/*/Code Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/BraveSoftware/Brave-Browser/User Data/*/GPUCache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/BraveSoftware/Brave-Browser/User Data/*/Service Worker/CacheStorage` | 离线网站资源可能不可重新获取，不等同普通 HTTP 缓存。 | mixed | documented | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md)。

## Chromium

识别证据：`Preferences`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Chromium/User Data/*/Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Chromium/User Data/*/Code Cache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Chromium/User Data/*/GPUCache` | 仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。 | cache | documented | candidate |
| `AppData/Local/Chromium/User Data/*/Service Worker/CacheStorage` | 离线网站资源可能不可重新获取，不等同普通 HTTP 缓存。 | mixed | documented | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md)。

## Firefox

识别证据：`cache2/entries`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Local/Mozilla/Firefox/Profiles/*/cache2` | 磁盘网络缓存；保留 Roaming 下书签、密码与配置数据库。 | cache | documented | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：[官方资料](https://firefox-source-docs.mozilla.org/netwerk/cache2/doc.html)。

## WeCom

识别证据：`Data`；或 `config`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Tencent/WXWork` | 聊天、附件、账号与数据库混合。不同版本位置不同，须以本机客户端显示的数据路径核实，暂不自动纳入。 | mixed | unverified | review |
| `AppData/Roaming/Tencent/WXWork/Log` | 本机独立日志目录；退出企业微信全部后台后预检，保留其他组件和账号数据。 | logs | local | candidate |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## WeChat

识别证据：`config` + `login`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Tencent/xwechat` | 账号、配置、数据库与附件混合；须完全退出客户端（含托盘）后整目录迁移，需人工确认。 | mixed | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## Tencent Meeting

识别证据：`Global` + `config_info.ini`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Tencent/WeMeet` | 账号、配置、数据库与附件混合；须完全退出客户端（含托盘）后整目录迁移，需人工确认。 | mixed | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。

## WeMail

识别证据：`sqlite` + `config`。

| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |
| --- | --- | --- | --- | --- |
| `AppData/Roaming/Tencent/WeMail` | 账号、配置、数据库与附件混合；须完全退出客户端（含托盘）后整目录迁移，需人工确认。 | mixed | local | review |

重定位线索：应用配置或启动参数；未做通用推断。

资料：本机元数据观察（非厂商承诺）。
