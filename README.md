# FreePort — Set your C: drive free

**Move your data. Keep your workflow.**

**English** · [简体中文](README.zh-CN.md)

A Windows tool for moving space-hungry folders to another drive while keeping their original paths. Choose what to move, choose its new home, then start the move. Want it back? Restore it to the original location.

> **Status: development preview.** The current goal is production-ready core migration and restoration. There is no stable release for real data yet; the acceptance criteria are in section 8 of the [requirements](docs/requirements.md) (Chinese), and current progress is in the [implementation status](docs/implementation-status.md) (Chinese).

## “I installed my apps on D:. Why is C: full again?”

You picked D: in the installer. You keep your projects on another drive. You even emptied the Recycle Bin.

C: is still red.

An AI tool downloaded another model. Your editor added extensions and update files. A compiler built its cache. A package manager kept its own copy of everything. Somewhere under `AppData` or a hidden folder in your home directory, a few more gigabytes quietly became a lot more gigabytes.

Then an update fails. A build stops. A model download runs out of room halfway through. You have work to do, and now you're searching for something you can safely delete.

Meanwhile, another drive has plenty of space.

**Your computer has room. Your tools just keep choosing the same place.**

This is why I'm building FreePort.

## You shouldn't need a tutorial for every folder

It makes sense to let an installer use its defaults. It also makes sense to hesitate before touching a folder you don't recognize.

If you're new to this, `AppData`, hidden directories, and environment variables can make a simple storage problem feel like a system repair job. You want your space back without breaking the apps you depend on.

If you've been doing this for years, you may already know how to change cache locations or create directory junctions. You still have to find the folders, check what's using them, move the data, verify it, and remember what went where. Then repeat for the next tool.

**The hesitation is the same: “Will this still work after I move it?”**

FreePort brings discovery, preview, migration, and restoration into one guided workflow, so you can see what will happen before you commit.

## Give the data a new home. Keep the old address.

FreePort's approach is straightforward: move a selected folder to a location you choose, then leave a directory junction at its original path. A junction lets software reach the new location through the path it already uses.

For example:

| | Original path | Where the data lives |
| --- | --- | --- |
| Before | `C:\Users\You\.cache\huggingface` | On C: |
| After | The same path, now a junction | `D:\Dev\huggingface` |

You choose the destination. Your tools keep their familiar address. FreePort's migration workflow preserves the original path without rewriting each app's configuration or environment variables.

C: is the everyday pain point, but the design supports selecting other local NTFS drives too. Moving between two folders on the same drive will not free space on that drive.

## Choose once. Start the move. Bring it back when you need to.

The intended GUI workflow is:

1. **Scan your chosen drives.** See candidate folders, their locations, sizes as they become available, and any reasons they cannot be moved.
2. **Select what you want to move.** Candidates start unchecked. Choose a destination and review each source → destination mapping.
3. **Start migration.** FreePort copies and verifies the data before switching the original path to a junction, and reports each folder's result.
4. **Restore when needed.** Scan the drive containing the junction, select it, review the space required, and restore a real directory at that location.

**One click to start the move after you've reviewed your choices. One click to start restoration after its checks.** Large folders still take time to copy and verify; the goal is to save you the manual work.

The GUI is for people who want to see and choose. The CLI `fp` uses the same core and the same checks.

## Interface language

The current iteration focuses on core migration and restoration, with Simplified Chinese as the maintained interface language. Existing English, Traditional Chinese, Japanese, Korean, and German resources remain but are not maintained for now; missing strings fall back to English.

## Built around the folders that keep growing

The discovery rules are being designed around real sources of disk pressure:

| What keeps growing | Examples in the planned coverage |
| --- | --- |
| AI tools and local models | Cursor, Claude Code, Kimi Code, Hugging Face, Ollama, Torch, ModelScope |
| Compiler and build data | Go caches, Cargo caches, vcpkg downloads and build trees, C++ build caches |
| Package caches | npm, pip, uv, pnpm, NuGet, Gradle |
| Editor extensions | VS Code, Cursor, Obsidian plugins |
| Application data | WeCom data, files, and caches, handled separately from unrelated folders |

These are discovery targets, not a claim that every version or folder is safe to move. A familiar name is only a clue: each selected directory still needs its actual path, contents, links, permissions, and usage checked. Protected system locations and unsupported objects must be blocked with an explanation.

## “What if I want to undo it?”

Restoration is part of the design from the start.

Say your model cache lives on D:, reached through a junction on C:. Later, you want the files back on C:. FreePort scans C: for the junction, shows where it points and how much space restoration needs, then lets you confirm the move back.

**It brings back the current files**, including changes made since migration. This is a location change, not a backup or a return to an earlier version of your data.

Discovery also covers supported directory links you created by hand, even without a FreePort history entry. For externally created links, or targets that may be used by other links, restoration keeps the target's contents so other users of that location are not disrupted.

## Trust has to be earned with your data

Some folders hold downloads you can replace. Others hold settings, conversations, and work you can't afford to lose. It makes sense to want more than a reassuring button before moving them.

FreePort's safety requirements put review and verification before cleanup, preserve data when something cannot be confirmed, and keep a record of interrupted work.

<details>
<summary>Safety requirements and optional handling of occupied folders</summary>

These requirements define the checks the product must pass:

- **Scanning is read-only.** It never starts a migration, restoration, deletion, or repair.
- **You review the plan.** Nothing starts preselected, and existing destination content is never silently merged or overwritten.
- **Verification comes before cleanup.** Copying, content verification, and required metadata checks precede removal of original data.
- **Uncertainty stops the operation.** Changing files, identity mismatches, unsupported links, or failed verification require preserving data and reporting the problem.
- **Interrupted work stays accountable.** Recovery uses transaction records and the actual disk state to determine whether work can continue or roll back. Every folder has its own result; partial completion stays partial completion.

Cross-drive work takes multiple steps. These requirements must pass the [production acceptance tests](docs/requirements.md#8-投产验收) before use with real user data. They are acceptance criteria, not a promise that every failure can be instantly undone.

### When another app is using the folder

The first release only detects and lists the occupying apps and reasons; close them yourself and retry. FreePort does not end processes or close other apps' file handles. "Request app exit" and "force-close external handles" are planned for a later release together with elevation, off by default and requiring explicit authorization.

</details>

## A few practical questions

**Will every app work after migration?**  
Compatibility depends on the app, its updater, permissions, and directory contents. FreePort must show known restrictions and block unsupported cases. Keeping the original path is the approach, not a universal compatibility guarantee.

**Do I need administrator access?**  
Fast MFT scanning needs administrator rights; FreePort requests UAC when needed and falls back to a normal scan if declined. Migrating user folders usually needs no elevation; missing permissions stop the operation before any data changes.

**How fast is it?**  
The scanner is being designed directly around Windows NTFS MFT/USN interfaces, without a third-party search client or service. That accelerates discovery. Copying and full verification still depend on data size and drive speed.

**What stays on my machine?**  
Scanning and file operations are local. The design has no telemetry, cloud service, or uploading of file contents.

**What storage is in scope?**  
The first release targets supported directories on local NTFS volumes. Network paths and unsupported filesystems are outside its migration scope. A destination drive needs sufficient space and must remain available while apps use the relocated data.

## Follow the project

If this sounds like your machine, a **Star** helps the next person with a full C: drive find FreePort. Tell us which folder keeps filling yours, and which app you need to keep working afterward.

For engineering details, see the [requirements](docs/requirements.md), [technical design](docs/technical-design.md), the [robocopy / `mklink /J` command contract](docs/system-command-backend.md), the [storage research](docs/application-storage-research.md) and [directory list](docs/application-storage-list.md), and the [repository conventions](AGENTS.md). Windows CLI and native GUI development target Rust and share one core.

**You bought that extra storage to use it. Your tools should be able to use it too.**

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
