use crate::platform::{key, within};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub category: String,
    pub source: PathBuf,
    pub recommended: bool,
    pub note: String,
    pub processes: Vec<String>,
}

pub struct Catalog {
    pub items: Vec<Candidate>,
    seen: BTreeSet<String>,
}
pub fn custom(path: &Path, name: &str, category: &str) -> Candidate {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(key(path).as_bytes());
    Candidate {
        id: format!("{:x}", digest)[..12].into(),
        name: name.into(),
        category: category.into(),
        source: path.to_path_buf(),
        recommended: false,
        note: crate::i18n::t("msg.b99bd3a311").into(),
        processes: vec![],
    }
}
impl Catalog {
    fn add(
        &mut self,
        path: PathBuf,
        name: &str,
        category: &str,
        recommended: bool,
        note: &str,
        processes: &[&str],
    ) {
        if fs::symlink_metadata(&path).is_err() || !self.seen.insert(key(&path)) {
            return;
        }
        // No overlapping roots: known precise subdirectories take precedence over discovery.
        if self
            .items
            .iter()
            .any(|c| within(&path, &c.source) || within(&c.source, &path))
        {
            return;
        }
        let digest = sha2::Sha256::digest(key(&path).as_bytes());
        use sha2::Digest;
        self.items.push(Candidate {
            id: format!("{:x}", digest)[..12].to_string(),
            name: name.into(),
            category: category.into(),
            source: path,
            recommended,
            note: note.into(),
            processes: processes.iter().map(|x| x.to_string()).collect(),
        });
    }
}

pub fn discover(home: &Path, extra: &[PathBuf]) -> Vec<Candidate> {
    let local = home.join("AppData/Local");
    let roaming = home.join("AppData/Roaming");
    let mut c = Catalog {
        items: vec![],
        seen: BTreeSet::new(),
    };
    let mut add =
        |base: &Path,
         rel: &str,
         name: &str,
         category: &str,
         auto: bool,
         note: &str,
         ps: &[&str]| c.add(base.join(rel), name, category, auto, note, ps);
    for (rel, name, ps) in [
        ("Cursor", "Cursor", &["cursor.exe"][..]),
        ("Code", "VS Code", &["code.exe"][..]),
        ("Claude", "Claude", &["claude.exe"][..]),
        ("Claude Code", "Claude Code", &["claude.exe"][..]),
        (
            "kimi-code-app",
            "Kimi Code",
            &["kimi.exe", "kimi-code-app.exe"][..],
        ),
        (
            "@deepseek-ai/dsh-desktop",
            "DeepSeek DSH",
            &["dsh.exe", "dsh-desktop.exe"][..],
        ),
        ("dsh-desktop", "DSH", &["dsh.exe", "dsh-desktop.exe"][..]),
        (
            "Grok Bot",
            "Grok Bot",
            &["grok-bot.exe", "grok bot.exe"][..],
        ),
        ("grok", "Grok", &["grok.exe"][..]),
        (
            "obsidian",
            crate::i18n::t("msg.c53e392bf1"),
            &["obsidian.exe"][..],
        ),
        (
            "Tencent/WXWork",
            crate::i18n::t("msg.5db3e80fea"),
            &["wxwork.exe", "wxworkweb.exe"][..],
        ),
        ("Codex", "Codex", &["codex.exe", "chatgpt.exe"][..]),
        ("OpenCode", "OpenCode", &["opencode.exe"][..]),
    ] {
        add(
            &roaming,
            rel,
            name,
            "app_data",
            false,
            crate::i18n::t("msg.648f13f904"),
            ps,
        );
    }
    for (rel, name, cat, ps) in [
        (
            ".cursor",
            crate::i18n::t("msg.9e7208f4e0"),
            "extensions",
            &["cursor.exe"][..],
        ),
        (
            ".vscode/extensions",
            crate::i18n::t("msg.175ee6d756"),
            "extensions",
            &["code.exe"][..],
        ),
        (
            ".vscode-insiders/extensions",
            crate::i18n::t("msg.9b311bde5d"),
            "extensions",
            &["code - insiders.exe"][..],
        ),
        (".claude", "Claude CLI", "app_data", &["claude.exe"][..]),
        (".kimi", "Kimi CLI", "app_data", &["kimi.exe"][..]),
        (
            ".kimi-code",
            "Kimi Code CLI",
            "app_data",
            &["kimi.exe", "kimi-code-app.exe"][..],
        ),
        (
            ".dsh",
            "DeepSeek DSH CLI",
            "app_data",
            &["dsh.exe", "dsh-desktop.exe"][..],
        ),
        (".grok", "Grok CLI", "app_data", &["grok.exe"][..]),
        (
            ".grokbot",
            "Grok Bot CLI",
            "app_data",
            &["grok.exe", "node.exe"][..],
        ),
        (
            ".codex",
            "Codex CLI",
            "app_data",
            &["codex.exe", "chatgpt.exe"][..],
        ),
        (
            ".cargo/registry",
            "Cargo registry",
            "build_cache",
            &["cargo.exe", "rustc.exe"][..],
        ),
        (
            ".cargo/git",
            "Cargo git",
            "build_cache",
            &["cargo.exe", "rustc.exe"][..],
        ),
        (
            ".rustup",
            crate::i18n::t("msg.533719c029"),
            "toolchain",
            &["rustup.exe", "rustc.exe", "cargo.exe"][..],
        ),
        (
            "go/pkg/mod",
            crate::i18n::t("msg.111b831194"),
            "build_cache",
            &["go.exe", "gopls.exe"][..],
        ),
        (
            ".cache/huggingface",
            "Hugging Face",
            "models",
            &["python.exe", "pythonw.exe", "hf.exe"][..],
        ),
        (
            ".cache/torch",
            "PyTorch",
            "models",
            &["python.exe", "pythonw.exe"][..],
        ),
        (
            ".cache/modelscope",
            "ModelScope",
            "models",
            &["python.exe"][..],
        ),
        (
            ".cache/hy3dgen",
            "Hunyuan 3D",
            "models",
            &["python.exe"][..],
        ),
        (
            ".cache/uv",
            crate::i18n::t("msg.eca42b8a73"),
            "package_cache",
            &["uv.exe", "python.exe"][..],
        ),
        (
            ".ollama/models",
            crate::i18n::t("msg.7f1420fa44"),
            "models",
            &["ollama.exe", "ollama app.exe"][..],
        ),
        (
            ".bun/install/cache",
            crate::i18n::t("msg.e2a9b53995"),
            "package_cache",
            &["bun.exe"][..],
        ),
        (
            ".gradle/caches",
            crate::i18n::t("msg.655fca04d0"),
            "build_cache",
            &["java.exe", "gradle.exe"][..],
        ),
        (
            ".nuget/packages",
            crate::i18n::t("msg.ad1ce32d91"),
            "package_cache",
            &["dotnet.exe", "msbuild.exe", "devenv.exe"][..],
        ),
        (
            ".android/cache",
            crate::i18n::t("msg.1e9663cd44"),
            "build_cache",
            &["java.exe"][..],
        ),
        (
            "npm-cache",
            crate::i18n::t("msg.a711d68b17"),
            "package_cache",
            &["node.exe"][..],
        ),
        (
            "Documents/WXWork",
            crate::i18n::t("msg.3e1dea08b3"),
            "app_data",
            &["wxwork.exe"][..],
        ),
    ] {
        add(
            home,
            rel,
            name,
            cat,
            matches!(
                cat,
                "package_cache" | "build_cache" | "extensions" | "models"
            ),
            crate::i18n::t("msg.7115c4f581"),
            ps,
        );
    }
    for (rel, name, cat, auto, ps) in [
        (
            "Programs/Cursor",
            crate::i18n::t("msg.f7ef37936e"),
            "application",
            false,
            &["cursor.exe"][..],
        ),
        (
            "Cursor",
            crate::i18n::t("msg.0857763b23"),
            "app_data",
            false,
            &["cursor.exe"][..],
        ),
        (
            "grok",
            crate::i18n::t("msg.33cb7ac506"),
            "app_data",
            false,
            &["grok.exe"][..],
        ),
        (
            "Tencent/WXWork",
            crate::i18n::t("msg.cca151fd16"),
            "app_data",
            false,
            &["wxwork.exe"][..],
        ),
        (
            "wxworkweb",
            crate::i18n::t("msg.74a7258c24"),
            "cache",
            true,
            &["wxwork.exe", "wxworkweb.exe"][..],
        ),
        (
            "go-build",
            "Go build",
            "build_cache",
            true,
            &["go.exe", "gopls.exe"][..],
        ),
        (
            "npm-cache",
            crate::i18n::t("msg.04053ff606"),
            "package_cache",
            true,
            &["node.exe"][..],
        ),
        (
            "pip/Cache",
            crate::i18n::t("msg.ed133740b2"),
            "package_cache",
            true,
            &["python.exe", "pip.exe"][..],
        ),
        (
            "uv/cache",
            crate::i18n::t("msg.eca42b8a73"),
            "package_cache",
            true,
            &["uv.exe", "python.exe"][..],
        ),
        (
            "pnpm-cache",
            crate::i18n::t("msg.5e379bdaa4"),
            "package_cache",
            true,
            &["node.exe"][..],
        ),
        (
            "pnpm/store",
            "pnpm store",
            "package_cache",
            true,
            &["node.exe"][..],
        ),
        (
            "vcpkg/archives",
            crate::i18n::t("msg.13fae839f8"),
            "build_cache",
            true,
            &["vcpkg.exe", "cl.exe", "msbuild.exe"][..],
        ),
        (
            "vcpkg",
            crate::i18n::t("msg.9bbed83fb2"),
            "build_cache",
            true,
            &["vcpkg.exe", "cl.exe"][..],
        ),
        (
            "sccache",
            "sccache",
            "build_cache",
            true,
            &["sccache.exe", "cl.exe", "rustc.exe"][..],
        ),
        (
            "ccache",
            "ccache",
            "build_cache",
            true,
            &["ccache.exe", "cl.exe"][..],
        ),
        (
            "Microsoft/vscode-cpptools",
            crate::i18n::t("msg.3a3e0bb303"),
            "build_cache",
            true,
            &["code.exe", "cpptools.exe"][..],
        ),
        (
            "Microsoft/VisualStudio",
            crate::i18n::t("msg.cd483d61c1"),
            "tool_data",
            false,
            &["devenv.exe", "msbuild.exe", "cl.exe"][..],
        ),
        (
            "Microsoft/MSBuild",
            crate::i18n::t("msg.cd9d413d76"),
            "build_cache",
            true,
            &["msbuild.exe", "cl.exe"][..],
        ),
        (
            "CMakeTools",
            crate::i18n::t("msg.f9a87390f1"),
            "build_cache",
            true,
            &["cmake.exe", "code.exe"][..],
        ),
        (
            "D3DSCache",
            "Direct3D shader cache",
            "build_cache",
            true,
            &["godot.exe", "blender.exe"][..],
        ),
        (
            "NVIDIA/DXCache",
            "NVIDIA shader cache",
            "build_cache",
            false,
            &["godot.exe", "blender.exe"][..],
        ),
    ] {
        add(
            &local,
            rel,
            name,
            cat,
            auto,
            crate::i18n::t("msg.d47ced8426"),
            ps,
        );
    }
    add(
        &roaming,
        "npm",
        crate::i18n::t("msg.8bd90032a7"),
        "tool_data",
        false,
        crate::i18n::t("msg.1f1c643bb2"),
        &["node.exe"],
    );
    drop(add);
    // Updater/download/compile caches discovered by directory names, never entire AppData.
    if let Ok(ds) = fs::read_dir(&local) {
        for d in ds.flatten() {
            let name = d.file_name().to_string_lossy().to_lowercase();
            if name.ends_with("-updater")
                || name.contains("compile-cache")
                || name == "claude-cli-nodejs"
            {
                c.add(
                    d.path(),
                    &crate::i18n::format(
                        "msg.a594ac1e3b",
                        &[format!("{}", d.file_name().to_string_lossy())],
                    ),
                    "cache",
                    true,
                    crate::i18n::t("msg.c004814c99"),
                    &[
                        "update.exe",
                        "cursor.exe",
                        "claude.exe",
                        "kimi.exe",
                        "dsh-desktop.exe",
                        "obsidian.exe",
                    ],
                );
            }
        }
    }
    let envs = [
        ("GOCACHE", crate::i18n::t("msg.286de0e0f2"), "build_cache"),
        (
            "GOMODCACHE",
            crate::i18n::t("msg.bf1e76e262"),
            "build_cache",
        ),
        ("HF_HOME", crate::i18n::t("msg.42d597efac"), "models"),
        ("HF_HUB_CACHE", crate::i18n::t("msg.e8f4cba75e"), "models"),
        (
            "HUGGINGFACE_HUB_CACHE",
            crate::i18n::t("msg.5159f11729"),
            "models",
        ),
        ("TORCH_HOME", crate::i18n::t("msg.f7a08e0e17"), "models"),
        (
            "PIP_CACHE_DIR",
            crate::i18n::t("msg.e0d20aad17"),
            "package_cache",
        ),
        (
            "UV_CACHE_DIR",
            crate::i18n::t("msg.7437f8c3e0"),
            "package_cache",
        ),
        (
            "SCCACHE_DIR",
            crate::i18n::t("msg.a24653837f"),
            "build_cache",
        ),
        (
            "CCACHE_DIR",
            crate::i18n::t("msg.24c469136e"),
            "build_cache",
        ),
        (
            "VCPKG_DEFAULT_BINARY_CACHE",
            crate::i18n::t("msg.ed6158fba6"),
            "build_cache",
        ),
        (
            "npm_config_cache",
            crate::i18n::t("msg.e05cc84ab6"),
            "package_cache",
        ),
    ];
    for (var, n, cat) in envs {
        if let Some(p) = env::var_os(var) {
            c.add(
                p.into(),
                n,
                cat,
                true,
                crate::i18n::t("msg.68c660a81c"),
                &["python.exe", "node.exe", "go.exe", "cargo.exe", "cl.exe"],
            );
        }
    }
    if let Some(root) = env::var_os("VCPKG_ROOT") {
        for sub in ["downloads", "buildtrees", "packages"] {
            c.add(
                PathBuf::from(&root).join(sub),
                &format!("vcpkg {sub}"),
                "build_cache",
                true,
                crate::i18n::t("msg.884f6b0ea2"),
                &["vcpkg.exe", "cl.exe"],
            );
        }
    }
    // Browser cache leaves only: never passwords/cookies/profile databases.
    for (rel, name, proc) in [
        ("Google/Chrome/User Data", "Chrome", "chrome.exe"),
        ("Microsoft/Edge/User Data", "Edge", "msedge.exe"),
        (
            "BraveSoftware/Brave-Browser/User Data",
            "Brave",
            "brave.exe",
        ),
        ("Chromium/User Data", "Chromium", "chrome.exe"),
    ] {
        let root = local.join(rel);
        if let Ok(ds) = fs::read_dir(&root) {
            for d in ds.flatten().filter(|d| {
                let n = d.file_name();
                n == "Default" || n.to_string_lossy().starts_with("Profile ")
            }) {
                for leaf in [
                    "Cache",
                    "Code Cache",
                    "GPUCache",
                    "Service Worker/CacheStorage",
                ] {
                    c.add(
                        d.path().join(leaf),
                        &format!("{name} {} {leaf}", d.file_name().to_string_lossy()),
                        "browser_cache",
                        true,
                        crate::i18n::t("msg.e039f3d6f4"),
                        &[proc],
                    );
                }
            }
        }
        for leaf in ["ShaderCache", "GrShaderCache", "GraphiteDawnCache"] {
            c.add(
                root.join(leaf),
                &format!("{name} {leaf}"),
                "browser_cache",
                true,
                crate::i18n::t("msg.e039f3d6f4"),
                &[proc],
            );
        }
    }
    let ff = local.join("Mozilla/Firefox/Profiles");
    if let Ok(ds) = fs::read_dir(ff) {
        for d in ds.flatten() {
            c.add(
                d.path().join("cache2"),
                &format!("Firefox {}", d.file_name().to_string_lossy()),
                "browser_cache",
                true,
                crate::i18n::t("msg.5fb97ce778"),
                &["firefox.exe"],
            );
        }
    }
    // Obsidian vault locations only, not note contents.
    if let Ok(f) = fs::read(roaming.join("obsidian/obsidian.json")) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&f) {
            if let Some(vaults) = v.get("vaults").and_then(|x| x.as_object()) {
                for v in vaults.values() {
                    if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                        c.add(
                            PathBuf::from(p).join(".obsidian/plugins"),
                            &crate::i18n::format(
                                "msg.f473cabb27",
                                &[format!(
                                    "{}",
                                    Path::new(p)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                )],
                            ),
                            "extensions",
                            true,
                            crate::i18n::t("msg.4f5c25d76a"),
                            &["obsidian.exe"],
                        );
                    }
                }
            }
        }
    }
    // Discover all remaining dot directories without following junctions.
    if let Ok(ds) = fs::read_dir(home) {
        for d in ds.flatten() {
            let name = d.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                && d.file_type()
                    .map(|t| t.is_dir() || t.is_symlink())
                    .unwrap_or(false)
            {
                c.add(
                    d.path(),
                    &name,
                    "hidden",
                    false,
                    crate::i18n::t("msg.97e8411123"),
                    &[],
                );
            }
        }
    }
    for p in extra {
        discover_projects(&mut c, p, 0);
    }
    c.items
}

fn discover_projects(c: &mut Catalog, p: &Path, depth: usize) {
    if depth > 5
        || fs::symlink_metadata(p)
            .map(|m| crate::platform::reparse(&m))
            .unwrap_or(true)
    {
        return;
    }
    for (sub, name, ps) in [
        (
            ".obsidian/plugins",
            crate::i18n::t("msg.101fae7061"),
            &["obsidian.exe"][..],
        ),
        (
            ".vs",
            crate::i18n::t("msg.f38008cb9c"),
            &["devenv.exe", "cl.exe", "msbuild.exe"][..],
        ),
        (
            "target",
            crate::i18n::t("msg.1ca93de350"),
            &["cargo.exe", "rustc.exe"][..],
        ),
    ] {
        if sub != "target" || p.join("Cargo.toml").exists() {
            c.add(
                p.join(sub),
                &format!(
                    "{name} {}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ),
                "project_cache",
                true,
                crate::i18n::t("msg.e01452b8a9"),
                ps,
            );
        }
    }
    if let Ok(ds) = fs::read_dir(p) {
        for d in ds.flatten() {
            let name = d.file_name().to_string_lossy().to_lowercase();
            if !name.starts_with('.')
                && !["target", "node_modules", "vendor", "build", "dist"].contains(&name.as_str())
                && d.file_type().map(|t| t.is_dir()).unwrap_or(false)
            {
                discover_projects(c, &d.path(), depth + 1);
            }
        }
    }
}
