# 构建和验证

产品运行只需 Windows；Rust、Python、MinGW 和 GitHub CLI 都是开发工具，不是产品运行时依赖。当前构建目标为 Windows 7 SP1 x64；验收状态见 [实现状态](implementation-status.md)。

安装 Rust nightly 和 `rust-src`，将采用 MSVCRT 的 MinGW-w64 `gcc`、`windres` 放入 PATH。`x86_64-win7-windows-gnu` 没有官方预编译标准库，因此必须使用 `build-std`，不能用普通 `x86_64-pc-windows-gnu` 构建后就宣称兼容 Windows 7。

```powershell
rustup toolchain install nightly --component rust-src --component rustfmt
python .githooks/setup.py
./scripts/build.ps1
python scripts/smoke.py --cli target/x86_64-win7-windows-gnu/release/fp.exe --gui target/x86_64-win7-windows-gnu/release/freeport.exe
```

`build.ps1` 使用静态 CRT、体积优化、LTO 和符号裁剪，然后检查每份 EXE 的大小、导入 DLL 与已知较新系统 API。默认大小上限为每份 5 MiB；检查通过不等于 Windows 7 实机已通过。发布时必须固定并记录实际通过验收的 Rust、MinGW、Windows SDK、依赖和构建脚本版本；目前 CI 使用 nightly 通道，尚不是最终可复现发布工具链。

边界单元测试不能跳过原生失败项：

```powershell
cargo +nightly test --locked --lib --target x86_64-win7-windows-gnu '-Zbuild-std=std,panic_abort'
./scripts/build.ps1 -Profile dev
./scripts/test-native.ps1 -Cli target/x86_64-win7-windows-gnu/debug/fp.exe
```

最后一条仅在具有管理员权限的专用测试机或 VM 运行。脚本新建唯一名称的测试 VHD，核验映像关联、RAW 状态、容量与虚拟磁盘类型后，只格式化该新 VHD。测试结束卸载映像，保留文件和失败证据。它不选择或格式化已有物理磁盘。`acceptance.py` 也可接收已经挂载好的专用 NTFS 测试目录作为 `--second-volume`，两边必须是不同盘。

原生验收包括每个持久化阶段的子进程中止和重复恢复、手工 junction、共享真实目标、当前数据恢复、ADS、跨卷迁移/恢复。强制关闭句柄测试属于下一目标，默认不运行，仅在 `acceptance.py --test handles` 时执行。缺少第二个测试卷时跨卷用例会失败，不计为跳过或通过。进程中止不能代替 VM 断电；断电、Windows 7 实机、文件映射、PID/句柄复用、介质移除等仍须单独留存证据。

测试文件位于新建的 `.build` 目录或测试 VHD，不自动删除失败现场，不使用用户聊天记录、模型或缓存。`__lock-test`、`__force-test`、`__junction-test` 和环境故障注入只存在于调试构建；危险测试辅助入口要求测试目录标记或显式测试开关。

仅发布开发源码的准备脚本：

```powershell
./scripts/publish-source.ps1 -Push
```

它只配置本仓库身份、安装并执行身份检查、提交允许的源码目录、核验 GitHub `suifei` / `7343085` 后发布 `suifei/freeport`。它不重写历史、不改变全局 Git 配置、不选择项目许可证、不创建稳定版二进制 Release。任何步骤失败都会停止；运行前应检查 [实现状态](implementation-status.md) 中未通过的投产门槛。

参考：[Rust Windows 7 GNU target](https://doc.rust-lang.org/rustc/platform-support/win7-windows-gnu.html)、[MSYS2 Action](https://github.com/msys2/setup-msys2)。
