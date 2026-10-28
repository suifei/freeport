"""Compile the reviewed, data-only application knowledge into the shared rule pack."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RULES = []
LOCAL = "local-metadata:2026-10-24"
# category is derived from kind only, so the two fields can never disagree.
CATEGORY = {"cache": "cache", "logs": "logs", "history": "app_data", "mixed": "app_data", "app": "app_data",
            "extensions": "extensions", "models": "models", "toolchain": "toolchain", "runtime": "runtime"}


def application(prefix, name, owner, markers, sources, processes, env=(), scope="home", confidence="documented"):
    def target(key, relative, purpose, kind="cache", migration="candidate", level=None):
        suffix = "/".join(p for p in (owner, relative) if p)
        RULES.append(dict(id=f"{prefix}-{key}".lower().replace("_", "-"), name=f"{name}: {key}",
                          category=CATEGORY[kind],
                          suffix=suffix, processes=processes,
                          knowledge=dict(application=name, purpose=purpose, kind=kind,
                                         confidence=level or confidence, migration=migration,
                                         scope=scope, owner_suffix=owner, markers=markers,
                                         sources=sources, relocation_env=list(env))))
    return target


claude = application("claude", "Claude Code", ".claude", [["settings.json", "projects"], [".credentials.json", "projects"]],
                     ["https://code.claude.com/docs/en/claude-directory"], ["claude.exe", "node.exe"], ["CLAUDE_CONFIG_DIR"])
for leaf, purpose, kind in [
    ("projects", "会话记录、项目记忆及工具输出；属于用户历史，完整保留，退出所有会话后迁移。", "history"),
    ("file-history", "文件修改前的恢复快照；用于回退，不能当作可丢弃缓存。", "history"),
    ("paste-cache", "历史粘贴内容；删除可能影响历史提示词，迁移时完整保留。", "history"),
    ("cache", "版本说明等可重新获取的应用缓存。", "cache"),
    ("plugins", "已安装插件、市场副本和插件状态；检测嵌套链接后按无链接分支处理。", "extensions"),
    ("debug", "诊断日志；其中 latest 可能是指向日志文件的绝对符号链接，整目录迁移须连同处理该链接，不能忽略。", "logs"),
]:
    claude(leaf, leaf, purpose, kind)
claude("skills", "skills", "共享技能经常指向 .agents/skills；保留连接，不整体搬运。", "extensions", "review")

codex = application("codex", "OpenAI Codex", ".codex", [["config.toml", "sessions"], ["auth.json", "sessions"]],
                    ["https://developers.openai.com/codex/config-advanced", LOCAL], ["codex.exe", "chatgpt.exe"], ["CODEX_HOME"])
for leaf, purpose, kind in [
    ("sessions", "可恢复会话的完整记录；用户历史，禁止按临时文件清理。", "history"),
    ("archived_sessions", "归档会话，迁移保留内容及 ACL。", "history"),
    ("attachments", "会话附件，可能仍被历史引用。", "history"),
    ("generated_images", "已生成图片，属于用户产物。", "history"),
    ("cache", "应用缓存，具体子项随版本变化；退出应用后处理。", "cache"),
    ("plugins/cache", "插件下载/版本目录；其中 chrome/latest 可能是指向版本目录的连接，可改选不含链接的其他分支。", "extensions"),
]:
    codex(leaf.replace("/", "-"), leaf, purpose, kind, level="local")
codex("sandbox", ".sandbox", "沙箱状态和权限关系，不作为缓存自动迁移。", "runtime", "exclude", "local")
codex("sandbox-bin", ".sandbox-bin", "沙箱执行文件，可能关联主机权限和版本绑定，需专门验证。", "runtime", "review", "local")
codex("root", "", "整个 Codex 用户目录；含沙箱状态与权限关系（.sandbox 排除自动迁移），整目录迁移前需人工确认。", "app", "review", "local")
runtime = application("codex-runtime", "Codex managed runtime", ".cache/codex-runtimes", [["codex-primary-runtime"]], [LOCAL], ["codex.exe", "chatgpt.exe"], confidence="local")
runtime("root", "", "Codex 托管运行时（primary-runtime）与安装暂存；升级与版本绑定尚未验证，需人工确认。", "runtime", "review")

kimi = application("kimi", "Kimi Code", ".kimi-code", [["config.toml", "sessions"]],
                   ["https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/data-locations.html", LOCAL], ["kimi.exe", "kimi code.exe"], ["KIMI_CODE_HOME"])
for leaf, purpose, kind, level in [
    ("sessions", "会话正文、任务、计划与会话日志；完整保留，先退出所有 CLI/桌面后台。", "history", "documented"),
    ("logs", "诊断日志。", "logs", "documented"),
    ("cache", "应用缓存；格式随应用版本变化。", "cache", "local"),
    ("files", "附件文件，可能仍被历史引用。", "history", "local"),
    ("file-history", "文件恢复快照；不是可丢弃缓存。", "history", "local"),
    ("search-index", "搜索索引；先退出写入索引的进程。", "cache", "local"),
]:
    kimi(leaf, leaf, purpose, kind, level=level)
kimi("server", "server", "桌面服务运行时，需验证升级器及后台服务停机行为。", "runtime", "review", "local")
kimi("root", "", "整个 Kimi Code 用户目录（含服务运行时与凭据）；先退出 CLI、桌面及后台服务，需人工确认。", "app", "review", "local")

dsh = application("dsh", "DeepSeek Harness", ".dsh", [["profiles", "sessions"], [".credentials.yaml", "sessions"]],
                  ["https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md", LOCAL], ["dsh.exe", "dsh-desktop.exe", "node.exe"], ["DSH_HOME"], confidence="local")
for leaf, purpose, kind in [("sessions", "CLI 与桌面共享的会话历史。", "history"), ("attachments", "会话附件。", "history"), ("cache", "应用缓存。", "cache")]:
    dsh(leaf, leaf, purpose, kind)
for leaf in ["dsh-runtimes", "storages", "profiles"]:
    dsh(leaf, leaf, "运行时、插件存储与数据库的关系随版本变化，需人工确认后迁移。", "mixed", "review")
dsh("root", "", "整个 DeepSeek Harness 用户目录（含运行时与存储）；先退出 CLI 与桌面，需人工确认。", "app", "review")

grok = application("grok", "Grok CLI", ".grok", [["config.toml", "sessions"], ["auth.json", "sessions"]],
                   ["https://docs.x.ai/build/settings", "https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/17-sessions.md", LOCAL],
                   ["grok.exe"], ["GROK_HOME"])
for leaf, purpose, kind in [("sessions", "会话正文和恢复元数据，属于用户历史。", "history"), ("logs", "诊断日志。", "logs"), ("downloads", "下载暂存；先确认升级器未运行。", "cache"), ("marketplace-cache", "插件市场缓存。", "extensions")]:
    grok(leaf, leaf, purpose, kind, level="documented" if leaf == "sessions" else "local")
grok("runtime", "bin", "实际 CLI 程序及升级入口，不能当缓存。", "runtime", "review", "local")
grok("root", "", "整个 Grok CLI 用户目录（含程序与升级入口）；先退出全部会话，需人工确认。", "app", "review", "local")
grokbot = application("grokbot", "Grok Bot", ".grokbot", [[".grokbot-data-root-v1", "settings.json"]],
                      ["https://docs.x.ai/grok-bot/get-started", LOCAL], ["grok bot.exe"], confidence="local")
grokbot("attachment-staging", "attachment-staging", "附件暂存；公开文档未确认其可重建性，先保留人工复核。", "mixed", "review")

for prefix, name, folder, exe in [("vscode", "VS Code", "Code", "code.exe"), ("cursor", "Cursor", "Cursor", "cursor.exe"), ("obsidian", "Obsidian", "obsidian", "obsidian.exe")]:
    vscode_like = prefix != "obsidian"
    sources = ["https://code.visualstudio.com/docs/configure/command-line", LOCAL] if vscode_like else ["https://obsidian.md/help/data-storage", LOCAL]
    markers = [["Local State", "Preferences", "User/settings.json"], ["Local State", "Preferences", "User/globalStorage"]] if vscode_like else [["Local State", "Preferences", "obsidian.json"]]
    app = application(prefix, name, f"AppData/Roaming/{folder}", markers, sources, [exe], confidence="local")
    app("root", "", "整个应用数据目录（设置、扩展状态、会话、缓存）；整目录迁移，完全退出应用及后台后进行。", "app")
    leaves = ["Cache", "Code Cache", "GPUCache", "CachedData", "CachedExtensionVSIXs", "logs"] if vscode_like else ["Cache", "Code Cache", "GPUCache", "logs"]
    for leaf in leaves:
        app(leaf.lower().replace(" ", "-"), leaf, "应用特定缓存/日志；只处理确实存在的叶目录，保留 User、Cookies、会话和数据库。", "logs" if leaf == "logs" else "cache")
for prefix, name, folder, exe in [("vscode", "VS Code", ".vscode", "code.exe"), ("vscode-insiders", "VS Code Insiders", ".vscode-insiders", "code - insiders.exe"), ("cursor", "Cursor", ".cursor", "cursor.exe")]:
    app = application(prefix, name, folder, [["extensions/extensions.json"]], ["https://code.visualstudio.com/docs/configure/extensions/extension-marketplace", LOCAL], [exe])
    app("profile", "", "编辑器用户目录（扩展、argv.json、CLI 数据）；整目录迁移，完全退出编辑器及扩展宿主后进行。", "app")
    app("extensions", "extensions", "已安装扩展，不是缓存；完全退出编辑器及扩展宿主后迁移。", "extensions")
for prefix, name, folder, marker, exe in [("vscode-program", "VS Code", "Microsoft VS Code", "Code.exe", "code.exe"), ("cursor-program", "Cursor", "cursor", "Cursor.exe", "cursor.exe")]:
    app = application(prefix, name, f"AppData/Local/Programs/{folder}", [[marker, "resources/app"]], ["https://code.visualstudio.com/docs/setup/windows", LOCAL], [exe], confidence="local")
    app("root", "", "用户级安装的程序目录；升级器按原路径写入，经连接后仍可用。退出编辑器和升级器后迁移。", "app")
obsidian = application("obsidian-vault", "Obsidian vault", ".obsidian", [["app.json", "community-plugins.json"]], ["https://obsidian.md/help/configuration-folder"], ["obsidian.exe"], scope="project")
obsidian("plugins", "plugins", "插件代码及 data.json 设置，不是纯缓存；保留插件状态。", "extensions")

cargo = application("cargo", "Rust Cargo", ".cargo", [["registry", "bin"], ["registry", "config.toml"]],
                    ["https://doc.rust-lang.org/cargo/guide/cargo-home.html"], ["cargo.exe", "rustc.exe"], ["CARGO_HOME"])
for leaf in ["registry", "git"]:
    cargo(leaf, leaf, "下载依赖、索引及源码缓存；停止所有 cargo/rustc 后迁移，保留全目录关系。")
cargo("root", "", "整个 CARGO_HOME（bin、registry、git、配置）；PATH 中的 .cargo/bin 经连接后保持原路径。停止所有 cargo/rustc 后迁移。", "toolchain")
rustup = application("rustup", "Rust toolchains", ".rustup", [["settings.toml", "toolchains"]],
                     ["https://rust-lang.github.io/rustup/environment-variables.html"], ["rustup.exe", "rustc.exe", "cargo.exe"], ["RUSTUP_HOME"])
rustup("toolchains", "toolchains", "已安装编译器/标准库，属于工具链，不是构建缓存。", "toolchain")
rustup("root", "", "整个 RUSTUP_HOME（工具链、设置、下载）；停止 rustup、cargo、rustc 及 rust-analyzer 后迁移。", "toolchain")
for leaf in ["downloads", "tmp"]:
    rustup(leaf, leaf, "工具链下载/解压暂存，需停止 rustup 更新。")
rust_project = application("rust-project", "Cargo project", "", [["Cargo.toml"]], ["https://doc.rust-lang.org/cargo/reference/environment-variables.html"], ["cargo.exe", "rustc.exe"], ["CARGO_TARGET_DIR"], "project")
rust_project("target", "target", "项目构建产物，须同时存在 Cargo.toml；自定义 target-dir 另行解析。")
go = application("go", "Go build", "AppData/Local/go-build", [["README"], ["trim.txt"]], ["https://go.dev/cmd/go/#hdr-Build_and_test_caching"], ["go.exe", "gopls.exe"], ["GOCACHE"])
go("build", "", "Go 编译和测试缓存；设置了 GOCACHE 时以 go env GOCACHE 为准。")
gomod = application("go-mod", "Go modules", "go/pkg", [["mod/cache/download"]], ["https://go.dev/ref/mod#module-cache"], ["go.exe", "gopls.exe"], ["GOMODCACHE", "GOPATH"])
gomod("cache", "mod", "模块缓存含只读源码、下载及校验元数据；不是整个 GOPATH。")
vcpkg = application("vcpkg", "vcpkg", "AppData/Local/vcpkg", [["archives"]], ["https://learn.microsoft.com/en-us/vcpkg/users/binarycaching"], ["vcpkg.exe", "cl.exe", "msbuild.exe"], ["VCPKG_DEFAULT_BINARY_CACHE"])
vcpkg("archives", "archives", "已编译包的二进制归档缓存。")
vcpkg_project = application("vcpkg-root", "vcpkg checkout", "", [[".vcpkg-root", "vcpkg.exe"]], ["https://learn.microsoft.com/en-us/vcpkg/users/config-environment"], ["vcpkg.exe", "cl.exe"], ["VCPKG_ROOT"], "project")
for leaf in ["downloads", "buildtrees", "packages"]:
    vcpkg_project(leaf, leaf, "下载/构建中间产物/暂存包；只在证实为 vcpkg 根后纳入。")
cpptools = application("cpptools", "VS Code C/C++", "AppData/Local/Microsoft/vscode-cpptools", [["ipch"]], ["https://code.visualstudio.com/docs/cpp/faq-cpp"], ["code.exe", "cpptools.exe", "cpptools-srv.exe"])
cpptools("cache", "", "IntelliSense/PCH 缓存（ipch 及各工作区数据库）；可通过 C_Cpp.intelliSenseCachePath 更改位置。")

pip = application("pip", "Python pip", "AppData/Local/pip/Cache", [["http-v2"], ["wheels"], ["http"]], ["https://pip.pypa.io/en/stable/topics/caching/"], ["python.exe", "pip.exe"], ["PIP_CACHE_DIR"])
pip("cache", "", "HTTP 响应与已构建 wheel 缓存；不是 Python 安装或虚拟环境。")
npm = application("npm", "Node npm", "AppData/Local/npm-cache", [["_cacache"], ["_npx"]], ["https://docs.npmjs.com/cli/v11/commands/npm-cache"], ["node.exe"], ["NPM_CONFIG_CACHE"])
npm("cache", "", "内容寻址下载缓存及 npx 暂存；不包含全局安装程序。")
npm_old = application("npm-user", "Node npm", "npm-cache", [["_cacache"], ["_npx"]], ["https://docs.npmjs.com/cli/v11/commands/npm-cache", LOCAL], ["node.exe"], ["NPM_CONFIG_CACHE"])
npm_old("cache", "", "旧版 npm 配置遗留在用户目录下的缓存；按缓存结构识别。")
uv = application("uv", "Python uv", "AppData/Local/uv/cache", [["CACHEDIR.TAG"]], ["https://docs.astral.sh/uv/concepts/cache/"], ["uv.exe", "python.exe"], ["UV_CACHE_DIR"])
uv("cache", "", "Windows 默认使用硬链接；可能和环境共享数据。跨盘拆开共享关系不一定释放原盘空间，先专门复核。", "cache", "review")
pnpm = application("pnpm", "pnpm", "AppData/Local/pnpm/store", [["v3"], ["v10"], ["v11"]], ["https://pnpm.io/settings#storedir"], ["node.exe"])
pnpm("store", "", "内容寻址包仓库可能硬链接到项目，先确认边界及同盘限制。", "cache", "review")
bun = application("bun", "Bun", ".bun", [["bin/bun.exe", "install/cache"]], ["https://bun.sh/docs/pm/global-cache"], ["bun.exe"], ["BUN_INSTALL_CACHE_DIR"])
bun("cache", "install/cache", "包缓存可能与已安装包硬链接共享，先核验后迁移。", "cache", "review")
npm_global = application("npm-global", "Node npm", "AppData/Roaming/npm", [["node_modules", "npm.cmd"], ["node_modules", "npx.cmd"]], ["https://docs.npmjs.com/cli/v11/configuring-npm/folders", LOCAL], ["node.exe"], ["NPM_CONFIG_PREFIX"])
npm_global("packages", "", "npm 全局安装的命令行工具及其 node_modules；PATH 指向原路径，经连接后不变。停止所有 node 进程后迁移。", "toolchain")
python = application("python-manager", "Python install manager", "AppData/Local/Python", [["bin", "_cache"]], ["https://docs.python.org/3/using/windows.html", LOCAL], ["python.exe", "py.exe", "pythonw.exe"], confidence="local")
python("root", "", "Python 安装管理器的解释器与缓存；注册表与 PATH 仍指向原路径。虚拟环境可能硬编码解释器路径，需人工确认。", "toolchain", "review")
playwright = application("playwright", "Playwright", "AppData/Local/ms-playwright", [[".links"]], ["https://playwright.dev/docs/browsers#managing-browser-binaries"], ["node.exe", "chrome.exe", "msedge.exe", "firefox.exe"], ["PLAYWRIGHT_BROWSERS_PATH"])
playwright("browsers", "", "Playwright 下载的浏览器二进制，可重新下载。停止测试与浏览器进程后迁移。", "cache")
codex_desktop = application("codex-desktop", "OpenAI Codex", "AppData/Local/OpenAI/Codex", [["runtimes", "bin"]], [LOCAL], ["codex.exe", "chatgpt.exe"], confidence="local")
codex_desktop("root", "", "桌面端运行时与程序副本；升级与版本绑定尚未验证，需人工确认。", "runtime", "review")
nuget = application("nuget", "NuGet", ".nuget", [["packages"]], ["https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders"], ["dotnet.exe", "msbuild.exe", "devenv.exe"], ["NUGET_PACKAGES"])
nuget("packages", "packages", "全局展开包；构建可能直接引用其中内容，停止构建后迁移。")
gradle = application("gradle", "Gradle", ".gradle", [["caches", "daemon"], ["caches", "wrapper"]], ["https://docs.gradle.org/current/userguide/directory_layout.html"], ["java.exe", "gradle.exe"], ["GRADLE_USER_HOME"])
for leaf in ["caches", "wrapper/dists"]:
    gradle(leaf.replace("/", "-"), leaf, "依赖/构建缓存或 Gradle 分发包；先停止 Gradle daemon。")

hf = application("hf", "Hugging Face", ".cache/huggingface", [["hub"], ["xet"]], ["https://huggingface.co/docs/huggingface_hub/guides/manage-cache"], ["python.exe", "hf.exe"], ["HF_HOME", "HF_HUB_CACHE", "HF_XET_CACHE"])
for leaf in ["hub", "xet"]:
    hf(leaf, leaf, "模型缓存必须保留 blobs/refs/snapshots 关系；可能有内部符号链接，不能仅搬某个大权重文件。", "models")
modelscope = application("modelscope", "ModelScope", ".cache/modelscope", [["hub"], ["ast_indexer"]], ["https://github.com/modelscope/modelscope/blob/master/modelscope/hub/utils/utils.py"], ["python.exe"], ["MODELSCOPE_CACHE"])
modelscope("models", "", "模型及下载元数据；不能把 .modelscope/credentials 当模型目录。", "models")
torch = application("torch", "PyTorch Hub", ".cache/torch", [["hub"]], ["https://docs.pytorch.org/docs/stable/hub.html"], ["python.exe"], ["TORCH_HOME"])
torch("hub", "hub", "Hub 下载代码与 checkpoints；保留整体。", "models")
ollama = application("ollama", "Ollama", ".ollama", [["models/blobs", "models/manifests"]], ["https://docs.ollama.com/faq"], ["ollama.exe", "ollama app.exe"], ["OLLAMA_MODELS"])
ollama("models", "models", "共享权重 blobs 和 manifests 必须整体保留；停止模型服务和下载后迁移。", "models")
for prefix, name, owner in [("hy3d", "Hunyuan3D", ".cache/hy3dgen"), ("rembg", "rembg", ".u2net")]:
    app = application(prefix, name, owner, [["checkpoints"]] if prefix == "hy3d" else [["u2net.onnx"]], [LOCAL], ["python.exe"], confidence="local")
    app("models", "", "模型目录；需确认对应工具和模型文件格式，不能仅凭文件夹名判断。", "models", "review")

for prefix, name, owner, exe in [
    ("chrome", "Google Chrome", "AppData/Local/Google/Chrome/User Data/*", "chrome.exe"),
    ("edge", "Microsoft Edge", "AppData/Local/Microsoft/Edge/User Data/*", "msedge.exe"),
    ("brave", "Brave", "AppData/Local/BraveSoftware/Brave-Browser/User Data/*", "brave.exe"),
    ("chromium", "Chromium", "AppData/Local/Chromium/User Data/*", "chrome.exe")]:
    app = application(prefix, name, owner, [["Preferences"]], ["https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md"], [exe])
    for leaf in ["Cache", "Code Cache", "GPUCache"]:
        app(leaf.lower().replace(" ", "-"), leaf, "仅浏览器配置实例下的磁盘缓存；退出全部浏览器后台。排除 Cookies、Login Data、History 和会话数据库。")
    app("offline", "Service Worker/CacheStorage", "离线网站资源可能不可重新获取，不等同普通 HTTP 缓存。", "mixed", "review")
firefox = application("firefox", "Firefox", "AppData/Local/Mozilla/Firefox/Profiles/*", [["cache2/entries"]], ["https://firefox-source-docs.mozilla.org/netwerk/cache2/doc.html"], ["firefox.exe"])
firefox("cache", "cache2", "磁盘网络缓存；保留 Roaming 下书签、密码与配置数据库。")
wxwork = application("wxwork", "WeCom", "AppData/Roaming/Tencent/WXWork", [["Data"], ["config"]], ["https://work.weixin.qq.com/", LOCAL], ["wxwork.exe", "wxworkweb.exe"], confidence="unverified")
wxwork("data", "", "聊天、附件、账号与数据库混合。不同版本位置不同，须以客户端设置中显示的数据路径核实，暂不自动纳入。", "mixed", "review")
wxwork_log = application("wxwork", "WeCom", "AppData/Roaming/Tencent/WXWork", [["Log", "wwmapp"], ["Log", "WeChatOCR"]], [LOCAL], ["wxwork.exe", "wxworkweb.exe"], confidence="local")
wxwork_log("logs", "Log", "独立日志目录；退出企业微信全部后台后预检，保留其他组件和账号数据。", "logs")
for prefix, name, folder, markers, exes in [
    ("xwechat", "WeChat", "xwechat", [["config", "login"]], ["weixin.exe", "wechat.exe", "wechatappex.exe"]),
    ("wemeet", "Tencent Meeting", "WeMeet", [["Global", "config_info.ini"]], ["wemeetapp.exe"]),
    ("wemail", "WeMail", "WeMail", [["sqlite", "config"]], ["wemail.exe", "wemailnode_x64.exe"])]:
    app = application(prefix, name, f"AppData/Roaming/Tencent/{folder}", markers, [LOCAL], exes, confidence="local")
    app("data", "", "账号、配置、数据库与附件混合；须完全退出客户端（含托盘）后整目录迁移，需人工确认。", "mixed", "review")


def main():
    ids = [r["id"] for r in RULES]
    assert len(ids) == len(set(ids))
    # purpose is Simplified Chinese; the other interface languages come from purposes-i18n.json.
    translations = json.loads((ROOT / "rules/purposes-i18n.json").read_text(encoding="utf-8"))
    used = {r["knowledge"]["purpose"] for r in RULES}
    missing, stale = used - translations.keys(), translations.keys() - used
    assert not missing and not stale, f"purposes-i18n.json: missing {sorted(missing)}, stale {sorted(stale)}"
    for r in RULES:
        text = translations[r["knowledge"]["purpose"]]
        assert sorted(text) == ["de", "en", "ja", "ko", "zh-TW"], r["id"]
        r["knowledge"]["purposes"] = dict(sorted(text.items()))
    output = ROOT / "rules/savemycdrive-rules.json"
    output.write_text(json.dumps(dict(schema_version=2, revision=2026102901, rules=RULES), ensure_ascii=False, indent=2)+"\n", encoding="utf-8", newline="\n")
    terms = {
        "en": ["Folder", "Total size", "Application", "Status / link target", "Name-only legacy rules are disabled. Import a version 2 application knowledge pack."],
        "zh-CN": ["目录", "递归总大小", "所属应用", "状态 / 链接目标", "已禁用仅按名称匹配的旧规则。请导入第 2 版应用知识库。"],
        "zh-TW": ["目錄", "遞迴總大小", "所屬應用", "狀態 / 連結目標", "已停用僅依名稱比對的舊規則。請匯入第 2 版應用知識庫。"],
        "ja": ["フォルダー", "合計サイズ", "アプリ", "状態 / リンク先", "名前のみで一致する旧ルールは無効です。バージョン 2 のアプリ知識パックをインポートしてください。"],
        "ko": ["폴더", "전체 크기", "앱", "상태 / 링크 대상", "이름만 비교하는 이전 규칙은 비활성화되었습니다. 버전 2 앱 지식 팩을 가져오세요."],
        "de": ["Ordner", "Gesamtgröße", "Anwendung", "Status / Linkziel", "Alte Regeln ohne Anwendungsnachweis sind deaktiviert. Importieren Sie ein Anwendungspaket der Version 2."],
    }
    for tag, values in terms.items():
        path = ROOT / f"locales/{tag}.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data.update(zip(["column.folder", "column.total", "column.application", "column.status", "rules.legacy"], values))
        scope = {
            "en": ["Migration scans user folders, including AppData. Other paths can be added manually.", "No user profile was found on {0}. Add a specific folder if needed."],
            "zh-CN": ["迁移扫描用户目录（含 AppData）；其他位置可手动添加。", "{0} 未发现用户目录；需要时可手动添加具体路径。"],
            "zh-TW": ["遷移掃描使用者目錄（含 AppData）；其他位置可手動加入。", "{0} 未發現使用者目錄；需要時可手動加入指定路徑。"],
            "ja": ["移行はユーザーフォルダー（AppData を含む）を検索します。他の場所は手動で追加できます。", "{0} にユーザープロファイルがありません。必要に応じてフォルダーを追加してください。"],
            "ko": ["이동 검색은 AppData를 포함한 사용자 폴더를 대상으로 합니다. 다른 경로는 직접 추가할 수 있습니다.", "{0}에서 사용자 폴더를 찾지 못했습니다. 필요하면 폴더를 직접 추가하세요."],
            "de": ["Die Suche zur Migration umfasst Benutzerordner einschließlich AppData. Weitere Pfade können manuell hinzugefügt werden.", "Auf {0} wurde kein Benutzerprofil gefunden. Fügen Sie bei Bedarf einen Ordner hinzu."],
        }
        data.update(zip(["scan.scope", "scan.no_profiles"], scope[tag]))
        path.write_text(json.dumps(data, ensure_ascii=False, indent=2)+"\n", encoding="utf-8", newline="\n")
    lines = ["# 应用目录知识库逐项清单", "", "调研日期：2026-10-24，2026-10-28 补充整目录规则。由 `scripts/build-knowledge.py` 与运行时 JSON 同源生成；本机调查结论和适用限制见 [调研报告](application-storage-research.md)。", "", "`candidate` 表示有证据可进入迁移预检，仍须经过权限、链接、句柄、空间和事务检查；没有默认勾选。`review` 会列出但标注“需确认”，同样不勾选，用户须先核对说明；`exclude` 排除自动发现。`app` 类型表示整个应用目录，对应原批处理的整目录迁移；与其下叶目录同时勾选时只迁移外层目录。用户历史、模型和扩展均须保留，不能按可丢弃缓存处理。", "", "`home` 路径相对于真实用户目录；`project` 路径相对于经标志文件确认的项目/应用根。`*` 只匹配单层目录。markers 外层是“或”，每组内部是“且”；仅检查存在和路径，绝不读取配置、令牌或会话正文。", "", "环境变量列为官方/本机的重定位线索，本版不会运行应用或执行规则内命令，也尚未自动解析各应用配置文件/环境变量以重定位根。", ""]
    for app_name in dict.fromkeys(r["knowledge"]["application"] for r in RULES):
        entries = [r for r in RULES if r["knowledge"]["application"] == app_name]
        k = entries[0]["knowledge"]
        lines.extend([f"## {app_name}", "", "识别证据：" + "；或 ".join(" + ".join(f"`{x}`" for x in group) for group in k["markers"]) + "。", "", "| 目标（相对路径） | 内容与迁移边界 | 类型 | 证据等级 | 决策 |", "| --- | --- | --- | --- | --- |"])
        for r in entries:
            k = r["knowledge"]
            lines.append(f"| `{r['suffix']}` | {k['purpose']} | {k['kind']} | {k['confidence']} | {k['migration']} |")
        lines.extend(["", "重定位线索：" + ("、".join(f"`{x}`" for x in k["relocation_env"]) or "应用配置或启动参数；未做通用推断") + "。", "", "资料：" + "；".join("本机元数据观察（非厂商承诺）" if s == LOCAL else f"[官方资料]({s})" for s in k["sources"]) + "。", ""])
    (ROOT / "docs/application-storage-list.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"{len(RULES)} rules, {len(set(r['knowledge']['application'] for r in RULES))} application/storage families, {output.stat().st_size} bytes")


if __name__ == "__main__":
    main()
