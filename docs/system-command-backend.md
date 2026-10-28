# Windows 系统命令搬运后端

FreePort / fp 共用一个后端。文件搬运使用 Windows 自带的 `robocopy.exe`，目录连接使用 `cmd.exe` 的 `mklink /J`。不实现第二套复制器，不分发这两个系统程序，也不加入第三方 DLL。

## 原批处理逐条对照

| 原命令 | 原语义 | FreePort 执行方式 |
| --- | --- | --- |
| `mkdir "目标"` | 创建目标目录 | 标准库创建目标父目录和本事务的暂存目录；目标已存在时拒绝合并 |
| `robocopy "源" "目标" /E /MOVE` | 包含空目录；搬运后删除源文件和目录 | robocopy 先复制到暂存目录，校验并刷盘后再执行 `/E /MOVE` 收尾；实际数据只复制一遍 |
| `rd /s /q "源"` | 递归、静默删除源目录，不区分是否残留未搬走的数据 | 仅按深度从后往前移除确认身份的空目录，使用标准库 `remove_dir`；任何残留文件使操作停止 |
| `mklink /J "源" "目标"` | 在原路径创建指向真实目标的目录 Junction | 最后调用系统 `cmd.exe /d /v:off /u /s /c` 执行相同的 `mklink /J`；验证真实目标后记成功 |

最终建连接前，仍按“搬运 → 清理源 → 建连接”顺序。增加准备副本是为了满足已经确定的“删除源数据前完整校验并持久化”要求：不把没有校验的单次 `/MOVE` 冒称为完整事务。

## 实际执行顺序和参数

这里 `S` 是原路径，`T` 是用户确认的最终路径，`T.stage` 是目标同卷暂存目录，`S.hold` 是源同卷保留目录。名字由事务 ID 唯一生成。

1. 预检路径、卷、空间、占用及连接，落盘事务记录；创建目标父目录和 `T.stage`。
2. 调用系统工具：

   ```bat
   robocopy "S" "T.stage" /E /COPY:DAT /R:0 /W:0 /XJ /SL /NFL /NDL /NJH /NJS /NP /UNICODE
   ```

3. 以 robocopy 返回码判断复制成功；刷盘。不另做文件比对。
4. 将 `T.stage` 同卷改名为 `T`；将 `S` 同卷改名为 `S.hold`。这些是两个独立的、各有日志的重命名操作。
5. 重新核对两侧内容和身份，保持源/目标文件的稳定读取句柄，再调用：

   ```bat
   robocopy "S.hold" "T" /E /COPY:DAT /R:0 /W:0 /XJ /SL /NFL /NDL /NJH /NJS /NP /UNICODE /MOVE /IS /NOCOPY /XL /XC /XN /XO
   ```

6. 复查目标完整，源只能剩原有空目录（`/MOVE` 通常已自行移除），依深度移除剩余空目录。源有文件、未知目录、重解析点或身份变化时停止。
7. 调用 `mklink /J "S" "T"`；核验连接指向和真实目标身份，落盘完成状态。失败则尝试从已验证目标补齐源并恢复原名；遇到冲突保留现场。

参数逐项说明：

| 参数 | 目的 |
| --- | --- |
| `/E` | 保留子目录和空目录，不能换成会排除空目录的 `/S` |
| `/COPY:DAT` | 显式采用原脚本默认语义：数据、属性、文件时间；不加入安全描述符、所有者、审计复制 |
| `/R:0 /W:0` | 遇到错误立即返回，由 FreePort 保留现场并报告；避免系统默认的大量重试和长时间等待 |
| `/XJ /SL` | 不跟随 Junction 或 symlink 进入别处；这只是防线；内部 Junction 在复制前的目录遍历中已被拒绝 |
| `/NFL /NDL /NJH /NJS /NP` | 减少文件、目录、表头、摘要和百分比日志；错误仍保留，FreePort 显示阶段进度 |
| `/UNICODE` | 请求 Unicode 输出；兼容部分版本仍输出 OEM 错误行的情况，按实际编码解码，避免报错乱码 |
| `/MOVE` | 搬运并移除源侧已处理名称，包括目录；不能换成仅文件的 `/MOV` |
| `/IS /NOCOPY` | 在已验证副本存在后处理相同文件；`/IS` 防止相同文件被跳过，`/NOCOPY` 禁止收尾覆盖已验证目标，也避免再复制一次数据 |
| `/IM`（支持时追加） | 某些新版 robocopy 将只读属性切换后的副本识别为 change-time 不同的 modified 文件，`/IS` 不包含它；仅在本机 `robocopy /?` 明确支持时追加，旧版不传 |
| `/XL` | 源中新出现且目标没有的对象保留在源侧，不纳入收尾 |
| `/XC /XN /XO` | 源/目标大小、时间发生变化时保留源，不覆盖已有目标；这是收尾时判断“源未变”的依据，大小与时间都相同的静默损坏不识别 |

不使用 `/COPYALL`、`/SEC`、`/MIR`、`/PURGE`、`/B`、`/ZB`。不显式传新版扩展的目录复制标志，采用系统目录属性默认值；不承诺目录时间与源一致。上述收尾命令在能力检查确认支持时另加 `/IM`。新的普通目标目录继承目标父目录权限，GUI 和 CLI 确认前均明确说明；不会强行调用 SetFileSecurityW 写回 owner/group。

## 命令成功与事务成功

robocopy 的 0–7 作为可继续检查的返回码，8 及以上作为错误；它们不是 FreePort 成功条件的替代。已实测：单独 `/MOVE` 在已有相同副本时会跳过文件；带 `/IS /NOCOPY` 才完成源名称收尾。收尾后必须核对残留再移除，不能只看返回码。

系统程序从 `GetSystemDirectoryW` 指定的位置启动，避免 PATH 中同名程序；robocopy 参数通过进程参数直接传入，不拼进 shell。`mklink` 路径逐个引号包裹，关闭 AutoRun 和延迟扩展；带 `%`、引号或换行的命令路径在预检时拒绝，避免 cmd 展开。取消仅终止本次工具子进程，不扩大到外部进程。

校验与收尾都不打开文件内容、不持有逐文件句柄，耗时主要是 robocopy 本身；不能把 MFT 的秒级发现速度等同于搬运耗时。

## 恢复与中断

“恢复”从盘上实际连接开始：robocopy 将真实目录的当前内容复制到连接旁，校验后将连接移到保留名，再把暂存普通目录切回原路径，最后仅移除旧连接条目。真实目标始终保留，其他连接和直接引用继续有效。这里不能对真实目标执行 `/MOVE` 或 `rd /s`。

迁移在源清理途中或 `mklink` 失败时：只补齐缺失源内容，不覆盖已有的变更文件；检验完整后再恢复原名。重建源目录本身中断也有 `rollback_copy_intent` / `rollback_rename_intent` 阶段，重新执行不重复删除。

验证状态见[实现状态](implementation-status.md)。

参数依据：[Microsoft robocopy](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy)、[Microsoft mklink](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/mklink)。
