# Windows 应用存储调研与扫描库

本文是知识库规则的证据与边界说明，范围限于用户目录及其中的 AppData。调研只读：没有迁移、删除真实用户文件，也没有关闭进程或句柄。

## 证据来源与边界

证据来自本机卸载注册表（HKCU/HKLM，含 32 位）的产品名、版本、安装位置，用户根目录的点目录，Local/Roaming 中的开发、AI 和桌面应用目录，并核对厂商文档和官方源码。`scripts/research-local.ps1` 默认只调查当前用户目录。尚无整个用户树的完整统计；权限拒绝、对象变化和不可读路径必须留在报告中，不能把未读大小写成 0 或声称完整覆盖。

原始安装信息、路径、链接目标和统计仅放在被 Git 忽略的 `.build/research-c/`。公开文档使用 `%USERPROFILE%`、`%APPDATA%`、`%LOCALAPPDATA%` 等模板，不含用户名、会话名、令牌、聊天或配置正文。

下列数值是 2026-10-24 **采样时的递归逻辑字节总和**，不是“可释放磁盘空间”。不跟随 reparse point；硬链接按目录项计数，未计 ADS、目录元数据和簇分配。运行中的应用仍会改变大小；同一上层总量与其子目录不可重复相加。

## 本机最值得处理的目录

| 应用是什么 | 本机目标 | 递归逻辑大小 | 应迁移什么／边界 |
| --- | --- | ---: | --- |
| VS Code 编辑器 | `%USERPROFILE%\.vscode\extensions` | 4.41 GiB | 已安装扩展；属于可重装程序内容，完整保留扩展索引，退出扩展宿主 |
| Grok CLI 编程工具 | `%USERPROFILE%\.grok\sessions` | 3.59 GiB | 会话历史；不是临时缓存，整体保留 |
| Rust 编译器管理器 | `%USERPROFILE%\.rustup\toolchains` | 3.00 GiB | 编译器和标准库；是工具链，停止 Cargo/rustup 后预检 |
| VS Code 应用数据 | `%APPDATA%\Code` | 2.34 GiB（父级） | 不整体视作缓存；拆分下列扩展下载缓存、日志等 |
| Codex 本地运行时 | `%USERPROFILE%\.cache\codex-runtimes` | 1.75 GiB | 包含 primary-runtime 和安装暂存；版本绑定尚需确认，仅列人工复核 |
| 企业微信客户端组件 | `%APPDATA%\Tencent\WXWork` | 1.29 GiB（父级） | 内含组件、日志、更新、OCR/小程序等，不是单一聊天附件目录 |
| Codex 编程工具 | `%USERPROFILE%\.codex\sessions` | 1.08 GiB | 会话记录，适合单独迁移；不夹带根目录 SQLite 和认证状态 |
| Obsidian 笔记应用数据 | `%APPDATA%\obsidian` | 1.06 GiB（父级） | 缓存、数据库、应用更新包混合，拆分具体叶目录 |
| VS Code 扩展下载 | `%APPDATA%\Code\CachedExtensionVSIXs` | 1.03 GiB | 已下载的 VSIX 包缓存；与已安装 extensions 分开 |
| Kimi Code 编程工具 | `%USERPROFILE%\.kimi-code\sessions` | 742.8 MiB | 会话状态、正文、任务、计划等；完整保留 |
| Claude Code 编程工具 | `%USERPROFILE%\.claude\projects` | 733.8 MiB | 会话、项目记忆和工具输出；不因 skills 的链接而整体阻止 |
| Go 模块缓存 | `%USERPROFILE%\go\pkg\mod` | 597.8 MiB | 本机还有旧位置残留；当前 GOPATH 在 D 盘，不能把旧缓存当当前配置 |
| npm 包管理器 | `%LOCALAPPDATA%\npm-cache` | 550.1 MiB | `_cacache`/`_npx` 等缓存；排除全局 npm 程序 |
| VS Code C/C++ 扩展 | `%LOCALAPPDATA%\Microsoft\vscode-cpptools` | 498.9 MiB | IntelliSense/PCH 缓存，有官方路径设置 |
| Codex 插件版本目录 | `%USERPROFILE%\.codex\plugins` | 492.9 MiB（父级） | 其中 cache 含 latest 版本连接；选择无连接分支或单独处理链接关系 |
| 企业微信日志 | `%APPDATA%\Tencent\WXWork\Log` | 361.4 MiB | 本机证据已确认日志目录，停客户端后可进入预检；不得据此推断其他组件可重建 |
| Grok 下载暂存 | `%USERPROFILE%\.grok\downloads` | 300.9 MiB | 本机观察；退出升级器后复核 |
| Claude 恢复快照 | `%USERPROFILE%\.claude\file-history` | 43.8 MiB | 文件回退所需的历史数据，不能按垃圾删除 |

其他实测：Cargo registry 222.9 MiB、Cargo git 198.4 MiB、NuGet packages 91.3 MiB、Edge 默认配置 Cache 99.2 MiB。uv/Gradle 的已测默认目录为空；不以目录存在推断它正在使用。Hugging Face 的当前环境线索指向 D 盘，旧 C 盘路径仍应按真实内容识别。沙箱会重定向 LOCALAPPDATA，调研使用真实用户根下的路径，不把沙箱路径当作桌面应用配置。

## `.claude`、`.codex` 的具体链接

以下根目录本身均为真实目录。已查到的子级 reparse point：

| 链接路径（相对用户目录） | 类型 | 目标 | 影响 |
| --- | --- | --- | --- |
| `.claude\debug\latest` | 文件符号链接 | `.claude\debug\<某会话>.txt`，实际存储为绝对路径 | 不是“已迁移的目录”；只影响 debug 分支的整体处理 |
| `.claude\skills\character-artist` | Junction | `.agents\skills\character-artist` | 共享技能连接，保留 |
| `.claude\skills\hard-surface` | Junction | `.agents\skills\hard-surface` | 同上 |
| `.claude\skills\reference-to-3d` | Junction | `.agents\skills\reference-to-3d` | 同上 |
| `.codex\plugins\cache\openai-bundled\chrome\latest` | Junction | 同级具体版本目录 `26.1007.71326` | 版本别名连接，保留其关系 |

应禁止把含未处理链接的父目录直接整体迁移；应允许其**没有链接的独立兄弟分支**单独预检，例如 projects、file-history、sessions、attachments。UI 与 CLI 必须给出 `link_details: [{path,target}]`，未知目标明确为 null；不能把文件链接、目录连接和整个根目录状态混为一谈。安全预检会拒绝含链接的迁移根，不对链接目标递归计量。

## 为什么目录名不能作为充分证据

`cache`、`target`、`temp`、`data`、`.xxx` 都可能是用户项目或不可再生的数据。采用“已知位置/项目根 + 应用特征文件或目录 + 明确的目标用途”的组合：先识别应用数据归属，再产生迁移候选。安装注册表、可执行文件、包清单和客户端显示的数据路径是进一步的独立证据。只发现应用数据不能声称可执行程序仍已安装，也不能据此认定数据可丢弃。

运行时已实施路径锚定和特征证据门槛，不自动收集所有点目录、任意 updater、任意 `.vs` 或任意 `target`。迁移时选择盘符，只对该盘的注册/当前用户配置目录及带 NTUSER.DAT 的普通 Users 子目录应用规则，包含 AppData；Win32 回退也只遍历这些用户目录。MFT 后端读取卷级名称索引后按用户路径过滤，不访问系统目录内容。没有用户目录的盘明确提示，可手动添加具体路径。恢复仍按所选盘找连接。Cargo 项目必须有 Cargo.toml；vcpkg checkout 必须有 .vcpkg-root 和 vcpkg.exe；Obsidian 插件必须有库配置；浏览器缓存必须位于有 Preferences 的具体 profile 下。标志只查元数据，不读取配置、认证或会话内容。

**待补齐的适配器：** 自动读取卸载注册表/包清单并关联版本；解析应用配置中的自定义根；按官方优先级解析环境变量。当前调研脚本读取了注册表，但产品扫描器尚未自动采集这项安装证据。JSON 的 relocation_env 是调研线索，尚不是已实现的重定位解析器。未验证的运行时、AI 新工具和模糊名称仅进入人工复核清单。

## 重点排除和特殊存储

| 对象 | 处理方式 | 原因／官方依据 |
| --- | --- | --- |
| MSVC、Visual Studio 安装目录和 Windows SDK | 安装器管理；不把整个目录作为缓存自动搬 | 编译器、组件注册和更新修复关系，不能等同编译缓存 |
| `%ProgramData%\Package Cache`、VS Installer Packages、`Windows\Installer` | 不自动 junction 迁移；按对应安装器政策处理 | [微软包缓存说明](https://learn.microsoft.com/en-us/visualstudio/install/disable-or-move-the-package-cache) |
| uv、pnpm、Bun store | 先复核外部硬链接关系 | Windows 硬链接只能在同一卷；跨盘复制缓存后，环境仍可能占着旧卷数据，不能承诺释放其全部占用。 [uv](https://docs.astral.sh/uv/concepts/cache/) / [pnpm](https://pnpm.io/settings#storedir) / [Bun](https://bun.sh/docs/pm/global-cache) |
| Hugging Face Hub | 整体保留 blobs、refs、snapshots 及内部链接；逐项预检 | [官方缓存布局](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)，只搬最大权重文件会破坏索引关系 |
| SQLite、LevelDB、浏览器 Local Storage、会话 DB | 不是普通缓存；应用停机并整体处理数据库/WAL 关系 | 不对正在写入的单个数据库或 WAL 做独立迁移 |
| 企业微信聊天、附件、同步和组件 | 客户端显示的数据路径为准；组件逐项验证 | 本机 WXWork 目录的大项含 Log、wwmapp、wmpf_Applet，不能根据 Data/config 猜测账号位置 |
| Obsidian `.obsidian/plugins` | 作为扩展数据保留 | 含插件配置 data.json；[官方配置目录说明](https://obsidian.md/help/configuration-folder) |
| Docker/WSL 虚拟磁盘、Android AVD | 优先原生重定位/导出；排除通用目录迁移 | [Docker 设置](https://docs.docker.com/desktop/settings-and-maintenance/settings/) / [Android 环境变量](https://developer.android.com/tools/variables)；含运行中磁盘和稀疏文件 |
| TEMP、Windows Temp、驱动 shader 缓存、Windows 更新缓存、回收站 | 不因名字自动产生迁移候选 | 混合活跃写入、系统服务和系统维护语义，不能一键选择整个临时目录 |
| `.ssh`、`.aws`、`.azure`、`.docker`、`.modelscope/credentials` 等 | 不作为缓存候选 | 可能存密钥、身份和配置；名称以点开头不是迁移依据 |

已发现但尚需版本级调查的本机应用还包括 Antigravity、Kiro、OpenCode、Gemini、Unsloth Studio、Zig、gopls、Blender/Godot 等；未凭名字加入自动候选。sccache 的 Windows 官方默认位置为 `%LOCALAPPDATA%\Mozilla\sccache`，不是旧代码写的 `%LOCALAPPDATA%\sccache`；待取得本机结构证据后启用。见 [sccache 官方存储说明](https://github.com/mozilla/sccache/blob/main/docs/Local.md)、[ccache 手册](https://ccache.dev/manual/latest.html)。

## 知识库和界面实现约定

逐项清单：[application-storage-list.md](application-storage-list.md)。运行时文件：[freeport-rules.json](../rules/freeport-rules.json)，schema_version=2，revision=2026102401。由 `scripts/build-knowledge.py` 同源生成清单与 JSON。CLI/GUI 共用 `rules::active/load/import/discover`；规则包不能执行命令、启动软件、更改事务策略或开启风险选项。schema 1 仍可读取，但没有应用证据的名称规则不再产生候选，迁移扫描保留旧文件并回退内置 v2 库，同时显示升级提示，避免旧包造成空白结果。恢复扫描独立于规则文件，即使旧库损坏也能找连接。

`knowledge` 记录 application、purpose、kind、confidence、migration、scope、owner_suffix、markers、sources、relocation_env。外部包通过原有 `fp rules --import FILE --state STORE` 或 GUI 导入；revision 必须递增。规则更新仅更新数据，是否可迁移仍由本机预检决定。

原生 TreeList 采用 **SysHeader32 + SysTreeView32 + Custom Draw**，保留原生三态复选框、键盘和懒展开，不新增 DLL。显示目录、递归总大小、应用、状态/链接目标；选择节点可查看具体连接路径和目标。每个已加载的兄弟组按数值 total 降序、同值按路径稳定排序；未知大小排后，不能按格式化的字符串排序。导航祖先只汇总不重叠候选，明确显示“仅汇总已识别候选目录”，不冒充整个 C 盘或用户根的实际总量。

一次计量同时产生目录总量和链接明细，避免每展开一层重新递归统计；每个候选保存最多 16,384 个目录总量、128 个链接明细，优先保留浅层目录。截断标志明确输出，未知子级大小不伪装为零。CLI 的 `scan --measure` 报告同样包含 directory_totals、link_details、details_truncated，兄弟组按数值总量降序；include/exclude 选择文件继续复用原有安全规划。**当前 total 是逻辑字节，不是 NTFS 唯一簇占用。** 分配空间与外部硬链接去重尚未实现，应作为独立度量补齐。
