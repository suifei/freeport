<div align="center">

<img src="resources/savemycdrive.png" width="112" alt="SaveMyCDrive icon">

# SaveMyCDrive

**Set your C: drive free.**<br>
Move your data. Keep your workflow.<br>
**Nothing deleted · One-click migrate · One-click restore**

[![Release](https://img.shields.io/github/v/release/suifei/save-my-c-drive?label=download&color=2563eb)](https://github.com/suifei/save-my-c-drive/releases/latest)
[![CI](https://github.com/suifei/save-my-c-drive/actions/workflows/ci.yml/badge.svg)](https://github.com/suifei/save-my-c-drive/actions/workflows/ci.yml)
![Windows 10/11 x64](https://img.shields.io/badge/Windows-10%20%7C%2011%20x64-0d9488)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-64748b)](#license)

**English** · [简体中文](README.zh-CN.md)

</div>

SaveMyCDrive moves the space-hungry folders under `AppData` and your home directory to another drive, then leaves a directory junction at the original path. Your apps keep using the path they know while the data lives where you chose. Want it back? Restore it.

<p align="center"><img src="docs/images/screenshot.en.png" width="860" alt="SaveMyCDrive main window: candidate folders with size, application and status"></p>

## Download and use

Get `SaveMyCDrive-v<version>-windows-x64.zip` from [Releases](https://github.com/suifei/save-my-c-drive/releases/latest), unzip, and run. No installer and no runtime to install.

| File | Purpose |
| --- | --- |
| `SaveMyCDrive.exe` | Graphical interface |
| `smcd.exe` | Command line, sharing the same core and checks |

**In the GUI:**

1. **Scan.** Tick drives and choose *Scan selected drives*, or *Scan a folder…*. Scanning is read-only.
2. **Select.** Each row is a recognized folder with its size, application and status. Nothing starts checked; grayed rows cannot be moved right now. Select a row to see its notes in the log.
3. **Choose a destination.** Enter a destination root such as `D:\Dev`, choose *Review and run*, and check each `source → destination` and the programs currently using those folders.
4. **Restore when needed.** Switch the mode to restoring links, scan the drive holding the junctions, check them, and restore.

**From the command line:**

```powershell
smcd scan --volumes C: --mode migrate --measure --out scan.json
smcd plan --scan scan.json --select <id> --destination D:\Dev --out plan.json
smcd apply --plan plan.json
smcd restore --source C:\Users\You\.cache\huggingface
smcd history
```

`smcd --help` lists every command; `--state PATH` keeps transaction records in a folder you choose (portable use).

## “I installed my apps on D:. Why is C: full again?”

You picked D: in the installer, keep projects elsewhere, and emptied the Recycle Bin. C: is still red.

An AI tool downloaded another model. Your editor added extensions. A compiler built its cache, and a package manager kept its own copy of everything, all under `AppData` or hidden folders in your home directory. Then an update fails, a build stops, or a download runs out of room halfway.

**Your computer has room. Your tools just keep choosing the same place.** If you're new to this, unfamiliar folders are scary to touch. If you're experienced, you can write `mklink`, but you still have to find folders, check what's using them, move data and remember what went where, every time. SaveMyCDrive puts discovery, review, migration and restoration in one flow.

## Move, don't delete: how it compares

Cleaning only buys time: deleted caches grow back, models have to be downloaded again, and session history or extensions can't be deleted at all. SaveMyCDrive deletes nothing; it gives that data a home on another drive.

| | Disk cleaners<br>(CCleaner, Storage Sense, 360, etc.) | Asking an AI assistant / AI skill each time | **SaveMyCDrive** |
| --- | --- | --- | --- |
| Approach | Delete caches and "junk" | The AI inspects folders, decides and runs commands on the spot | Move whole folders to another drive, junction at the original path |
| AI coding tool data<br>(sessions, models, extensions) | Mostly unrecognized: delete or ignore | Judged on the fly; conclusions can change each time | 124 built-in rules match location and app markers |
| Data loss | Deleted is gone; re-download or rebuild | Possible: important folders mistaken for cache | Nothing deleted; source retired only after a successful copy, stops on leftovers |
| A few weeks later | It grows back; clean again | It grows back; ask the AI again | New files land on the new drive, not on C: |
| Changed your mind | No way back | Depends on what the AI actually ran | One-click restore, including files added since |
| Power loss / interruption | — | Half-run commands are hard to trace | Journaled; continue or roll back from the log |
| Cost per run | Free; some stay resident or show promotions | Many tokens per run, inconsistent results | Zero tokens, repeatable; exits when done, no network |
| Auditable | Closed source | Prompts are readable, but the model decides what runs | Open source; rules and procedure are fixed |

Some cleaners also offer a "move C: data" feature, but the scope is a vendor list that generally doesn't cover AI coding tools.

### Why not just ask an AI to clean up each time?

AI is great for one-off, unfamiliar problems. Freeing C: every few weeks is the opposite: repetitive, and your data is on the line.

- **Inconsistent.** The same folder can be "safe to delete cache" in one session and "keep" in the next, depending on the model and the prompt.
- **Mistaken deletes.** `.claude\projects` sounds like a cache but holds Claude Code's session history. `huggingface\hub` blobs and snapshots reference each other; deleting "the biggest files" corrupts models and costs tens of GB to re-download. One mistyped path in `Remove-Item -Recurse` or `rm -rf` can take out the parent folder.
- **Token cost.** Every run lists folders, sums sizes and looks things up, often tens of thousands of lines of output; two weeks later C: is full again and you pay again.
- **Treats the symptom.** Whatever was deleted grows back.

SaveMyCDrive turns those judgments into reviewable rules and a fixed procedure: the same input gives the same result, you preview before anything runs, you can roll back afterwards, and it costs no tokens.

## How it works

| | Path your apps use | Where the data lives |
| --- | --- | --- |
| Before | `C:\Users\You\.cache\huggingface` | On C: |
| After | Same path, now a junction | `D:\Dev\huggingface` |

It relies on the tools Windows already ships:

1. **Check.** Programs using the folder are listed by product name so you can close them. SaveMyCDrive never ends processes.
2. **Copy.** The system `robocopy` copies, multi-threaded, into a staging folder next to the destination; its exit code decides success.
3. **Switch.** The staged copy takes its place and `robocopy /MOVE` retires the source. If any file is left behind, SaveMyCDrive stops and keeps the data.
4. **Link.** The system `mklink /J` creates the junction at the original path.

Every step is journaled. After a power loss or interruption, *Recover interrupted work* continues or rolls back from the journal and the actual disk state; each folder's result and the batch result are recorded separately.

## Built around the folders that keep growing

A built-in application knowledge pack (124 rules) recognizes folders by location and application markers, not by name alone:

| What keeps growing | Examples |
| --- | --- |
| AI tools and local models | Claude Code, Codex, Kimi Code, Cursor, Hugging Face, Ollama, Torch, ModelScope |
| Compiler and build data | Rust toolchains / Cargo, Go, vcpkg, C++ IntelliSense |
| Package caches | npm, pip, uv, pnpm, NuGet, Gradle, Playwright |
| Editors and app data | VS Code, Cursor, Obsidian, WeCom and more |

See the [directory list](docs/application-storage-list.md) (Chinese). Folders marked for review are listed, but you should read their notes first.

## Safety

- **Scanning is read-only.** It never starts a migration, restoration, deletion or repair.
- **You decide.** Candidates and every optional switch start off; existing destination content is never silently merged or overwritten.
- **Uncertainty stops the operation.** Failed copies, folders still in use, identity changes or unknown link types keep the data in place and explain why.
- **No deleting through links.** Cleanup is `robocopy /MOVE` plus empty-directory removal; never an unverified recursive delete.
- **Restore brings back current files**, including changes made since migration. Junctions you created by hand are found and restorable too.
- **Everything stays local.** No telemetry, no network access, no uploads.

## FAQ

**Do I need administrator rights?**
Whole-drive fast scanning (NTFS MFT) does; SaveMyCDrive asks via UAC when needed and falls back to a normal scan if you decline. Migrating user folders usually doesn't.

**Will every app work after migration?**
Keeping the original path is the method, not a universal compatibility guarantee. Updaters, permissions and folder contents all matter; known restrictions are shown in the list and log.

**Which drives are supported?**
Local NTFS volumes. Network paths and other filesystems are out of scope. The destination needs enough space and must stay available while apps use the data.

**How fast is it?**
Scanning reads NTFS MFT/USN directly; copying uses `robocopy /MT:8`. Real time depends on data size and disk speed.

## Interface language

Simplified Chinese is the maintained interface language. English, Traditional Chinese, Japanese, Korean and German remain available but are not updated for new text; missing strings fall back to English.

## Development

See the [requirements](docs/requirements.md), [technical design](docs/technical-design.md), [system command contract](docs/system-command-backend.md), [build and validation](docs/development.md) and [implementation status](docs/implementation-status.md) (Chinese). Written in Rust; the CLI and the native Win32 GUI share one core. Repository conventions are in [AGENTS.md](AGENTS.md).

If SaveMyCDrive helps, a **Star** helps the next person with a full C: drive find it.

## Contact

- QQ: 410000
- Issues and suggestions: [GitHub Issues](https://github.com/suifei/save-my-c-drive/issues)

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Dependency notices ship with the release archive (`third-party-notices/`).
