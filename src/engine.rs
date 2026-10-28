//! Recoverable directory transactions. No recursive deletion or cross-volume atomicity claims.
use crate::{fail, handles, inventory::Row, platform as p, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::{fs::MetadataExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*};

pub type Progress<'a> = &'a dyn Fn(String);
#[derive(Clone, Default)]
pub struct Control {
    pub cancel: Arc<AtomicBool>,
    pub options: handles::Options,
}
impl Control {
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return fail(crate::i18n::t("msg.0c626b7ea7"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub items: Vec<PlanItem>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub id: String,
    pub kind: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub stage: PathBuf,
    pub hold: PathBuf,
    pub source_id: p::Identity,
    pub origin_id: p::Identity,
    pub destination_id: Option<p::Identity>,
    pub phase: String,
    pub error: Option<String>,
}
pub fn default_store() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("smcd.exe"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("savemycdrive-state")
}
fn now() -> Result<String> {
    Ok(format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    ))
}
pub(crate) fn record_path(store: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return fail(crate::i18n::t("msg.c03028c320"));
    }
    Ok(store.join(format!("{id}.json")))
}
pub(crate) fn checked_write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let payload = serde_json::to_value(value)?;
    let bytes = serde_json::to_vec(&payload)?;
    let envelope = serde_json::json!({"version":1,"sha256":format!("{:x}",Sha256::digest(&bytes)),"payload":payload});
    p::atomic_json(path, &envelope)
}
pub(crate) fn checked_read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let payload = v.get("payload").ok_or(crate::i18n::t("msg.1c49643bba"))?;
    if v.get("version").and_then(|v| v.as_u64()) != Some(1)
        || v.get("sha256").and_then(|v| v.as_str())
            != Some(&format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(payload)?)
            ))
    {
        return fail(crate::i18n::format(
            "msg.459c3fc3ac",
            &[format!("{}", path.display())],
        ));
    }
    Ok(serde_json::from_value(payload.clone())?)
}
fn save(store: &Path, r: &Record) -> Result<()> {
    checked_write(&record_path(store, &r.id)?, r)
}
fn phase(store: &Path, r: &mut Record, s: &str) -> Result<()> {
    r.phase = s.into();
    r.error = None;
    save(store, r)?;
    fault(s);
    Ok(())
}
fn fault(s: &str) {
    #[cfg(debug_assertions)]
    if std::env::var("SMCD_TEST_FAIL_AT").ok().as_deref() == Some(s) {
        std::process::abort();
    }
    let _ = s;
}
fn lock_store(store: &Path) -> Result<File> {
    fs::create_dir_all(store)?;
    p::lock(&store.join(".lock"))
}
fn absent(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err()
}
fn ensure_identity(path: &Path, id: p::Identity) -> Result<()> {
    if p::identity(path)? != id {
        return fail(crate::i18n::format(
            "msg.2b26fe99be",
            &[format!("{}", path.display())],
        ));
    }
    Ok(())
}
fn protected(path: &Path) -> bool {
    let root = path
        .components()
        .next()
        .map(|c| PathBuf::from(format!("{}\\", c.as_os_str().to_string_lossy())))
        .unwrap_or_default();
    if path.parent().is_none() {
        return true;
    }
    let mut blocked = vec![
        root.join("Windows"),
        root.join("Program Files"),
        root.join("Program Files (x86)"),
        root.join("ProgramData"),
        root.join("System Volume Information"),
        root.join("$Recycle.Bin"),
    ];
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let h = PathBuf::from(home);
        if [
            h.clone(),
            h.join("AppData"),
            h.join("AppData/Local"),
            h.join("AppData/Roaming"),
            h.join("Documents"),
            h.join("Desktop"),
            h.join("Downloads"),
        ]
        .iter()
        .any(|x| p::same(path, x))
        {
            return true;
        }
        blocked.extend([
            h.join("AppData/Local/Packages"),
            h.join("AppData/Local/Package Cache"),
        ]);
    }
    blocked.iter().any(|b| p::overlap(b, path))
        || path
            .file_name()
            .is_some_and(|n| n == "AppData" || n == "Users" || n == "ProgramData")
}
fn shared(paths: &[&Path]) -> Result<()> {
    use windows_sys::Win32::NetworkManagement::NetManagement::*;
    let mut ptr = std::ptr::null_mut();
    let mut count = 0;
    let mut total = 0;
    let mut resume = 0;
    loop {
        let rc = unsafe {
            NetShareEnum(
                std::ptr::null(),
                2,
                &mut ptr,
                65536,
                &mut count,
                &mut total,
                &mut resume,
            )
        };
        if rc != 0 && rc != 234 {
            return fail(crate::i18n::format("msg.43f5448abe", &[format!("{}", rc)]));
        }
        if !ptr.is_null() {
            let entries =
                unsafe { std::slice::from_raw_parts(ptr as *const SHARE_INFO_2, count as usize) };
            for e in entries {
                if e.shi2_path.is_null() {
                    continue;
                }
                let read = |p: *const u16| -> String {
                    if p.is_null() {
                        return String::new();
                    }
                    let mut n = 0;
                    unsafe {
                        while n < 32768 && *p.add(n) != 0 {
                            n += 1;
                        }
                        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
                    }
                };
                let name = read(e.shi2_netname);
                if name == "ADMIN$" || name == "IPC$" || (name.len() == 2 && name.ends_with('$')) {
                    continue;
                }
                let path = PathBuf::from(read(e.shi2_path));
                if path.is_absolute() && paths.iter().any(|p| p::overlap(p, &path)) {
                    unsafe {
                        NetApiBufferFree(ptr as _);
                    }
                    return fail(crate::i18n::format(
                        "msg.3e8b1ca80e",
                        &[format!("{}", name)],
                    ));
                }
            }
            unsafe {
                NetApiBufferFree(ptr as _);
            }
            ptr = std::ptr::null_mut();
        }
        if rc == 0 {
            break;
        }
    }
    Ok(())
}
fn validate_paths(source: &Path, target: &Path, store: &Path, allow_link: bool) -> Result<()> {
    if !source.is_absolute() || !target.is_absolute() {
        return fail(crate::i18n::t("msg.0aede92896"));
    }
    if !allow_link {
        crate::system_ops::validate_junction_paths(source, target)?;
    }
    let actual_target = p::absolute(target)?;
    let actual_source = if allow_link {
        source.to_path_buf()
    } else {
        p::absolute(source)?
    };
    if !p::same(&actual_target, target) || !p::same(&actual_source, source) {
        return fail(crate::i18n::t("msg.48d3c20ba3"));
    }
    if protected(source)
        || protected(target)
        || p::overlap(source, target)
        || p::overlap(source, store)
        || p::overlap(target, store)
        || [source, target]
            .iter()
            .any(|p| p::within(&std::env::current_exe().unwrap_or_default(), p))
    {
        return fail(crate::i18n::t("msg.e196dbf7e9"));
    }
    p::ntfs(source.parent().ok_or(crate::i18n::t("msg.4d24fe25b3"))?)?;
    p::ntfs(target)?;
    shared(&[source, target])?;
    Ok(())
}
pub fn make_plan(rows: &[Row], ids: &[String], destination: &Path) -> Result<Plan> {
    make_plan_with_store(rows, ids, destination, &default_store())
}
pub fn make_plan_with_store(
    rows: &[Row],
    ids: &[String],
    destination: &Path,
    store: &Path,
) -> Result<Plan> {
    if ids.is_empty() {
        return fail(crate::i18n::t("msg.c42783dc92"));
    }
    let destination = p::absolute(destination)?;
    let mut items = vec![];
    let selected = crate::selection::Selection::from_ids(rows, ids)?;
    let roots = crate::selection::roots(selected.include);
    for root in &roots {
        let row = rows
            .iter()
            .find(|r| p::same(&r.candidate.source, root))
            .ok_or(crate::i18n::t("msg.5f5c222eab"))?;
        let id = &row.candidate.id;
        if row.stats.links > 0 || row.status == "nested_links" {
            return fail(crate::i18n::t("selection.link"));
        }
        let source = p::absolute(&row.candidate.source)?;
        if p::reparse(&fs::symlink_metadata(&row.candidate.source)?) {
            return fail(crate::i18n::t("msg.2d7b3f7d01"));
        }
        let leaf = source.file_name().ok_or(crate::i18n::t("msg.08e1181799"))?;
        let target = destination.join(format!("{}-{}", leaf.to_string_lossy(), row.candidate.id));
        items.push(PlanItem {
            id: id.clone(),
            name: row.candidate.name.clone(),
            source,
            target,
            bytes: row.stats.bytes,
        });
    }
    let plan = Plan { version: 1, items };
    validate_plan(&plan, store)?;
    Ok(plan)
}
pub fn custom_plan(source: &Path, target: &Path, store: &Path) -> Result<Plan> {
    if p::reparse(&fs::symlink_metadata(source)?) {
        return fail(crate::i18n::t("msg.2d7b3f7d01"));
    }
    let source = p::absolute(source)?;
    let target = p::absolute(target)?;
    let plan = Plan {
        version: 1,
        items: vec![PlanItem {
            id: "custom".into(),
            name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            source,
            target,
            bytes: 0,
        }],
    };
    validate_plan(&plan, store)?;
    Ok(plan)
}
pub fn validate_plan(plan: &Plan, store: &Path) -> Result<()> {
    if plan.version != 1 || plan.items.is_empty() {
        return fail(crate::i18n::t("msg.1a0054c0c4"));
    }
    for (i, item) in plan.items.iter().enumerate() {
        validate_paths(&item.source, &item.target, store, false)?;
        if !absent(&item.target) {
            return fail(crate::i18n::format(
                "msg.44ea769e92",
                &[format!("{}", item.target.display())],
            ));
        }
        let m = fs::symlink_metadata(&item.source)?;
        if !m.is_dir() || p::reparse(&m) {
            return fail(crate::i18n::t("msg.7dabf0cead"));
        }
        for prior in &plan.items[..i] {
            if [&item.source, &item.target].iter().any(|a| {
                [&prior.source, &prior.target]
                    .iter()
                    .any(|b| p::overlap(a, b))
            }) {
                return fail(crate::i18n::t("msg.beb3f75e44"));
            }
        }
    }
    Ok(())
}

fn stream_names(path: &Path) -> Result<Vec<String>> {
    let mut d: WIN32_FIND_STREAM_DATA = unsafe { std::mem::zeroed() };
    let h = unsafe {
        FindFirstStreamW(
            p::path_wide(path).as_ptr(),
            FindStreamInfoStandard,
            &mut d as *mut _ as _,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut names = vec![];
    loop {
        let n = d
            .cStreamName
            .iter()
            .position(|c| *c == 0)
            .ok_or(crate::i18n::t("msg.4de6b45259"))?;
        names.push(String::from_utf16(&d.cStreamName[..n])?);
        if unsafe { FindNextStreamW(h, &mut d as *mut _ as _) } == 0 {
            let e = std::io::Error::last_os_error();
            unsafe {
                FindClose(h);
            }
            if e.raw_os_error() != Some(38) {
                return Err(e.into());
            }
            break;
        }
    }
    Ok(names)
}
fn stream_path(path: &Path, name: &str) -> PathBuf {
    if name == "::$DATA" {
        path.to_path_buf()
    } else {
        let mut s = path.as_os_str().to_os_string();
        s.push(name);
        s.into()
    }
}
/// One walk that never opens files: directory entries already carry size and attributes.
/// Returns (bytes, objects) and rejects what robocopy cannot carry faithfully. Copy
/// correctness itself is robocopy's exit code; source retirement is robocopy /MOVE.
fn survey(root: &Path, control: &Control, progress: Progress) -> Result<(u64, u64)> {
    let (mut bytes, mut count) = (0u64, 0u64);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            control.check()?;
            let entry = entry?;
            let path = entry.path();
            if path.to_str().is_none() {
                return fail(crate::i18n::t("msg.c48c45a82d"));
            }
            let m = entry.metadata()?;
            if p::reparse(&m) {
                if !m.file_type().is_symlink() {
                    return fail(crate::i18n::format(
                        "msg.df3664d356",
                        &[format!("{}", path.display())],
                    ));
                }
                if fs::read_link(&path)?.is_absolute() {
                    return fail(crate::i18n::t("msg.2e8cdf90ca"));
                }
            } else if m.file_attributes()
                & (FILE_ATTRIBUTE_ENCRYPTED
                    | FILE_ATTRIBUTE_OFFLINE
                    | FILE_ATTRIBUTE_SPARSE_FILE
                    | FILE_ATTRIBUTE_COMPRESSED)
                != 0
            {
                return fail(crate::i18n::format(
                    "msg.ec65c77b65",
                    &[format!("{}", path.display())],
                ));
            } else if m.is_dir() {
                stack.push(path);
            } else {
                bytes += m.len();
            }
            count += 1;
            if count % 10000 == 0 {
                progress(crate::i18n::format("msg.3e664d9989", &[format!("{count}")]));
            }
        }
    }
    Ok((bytes, count))
}
// Windows robocopy copies data, streams, attributes and times; its exit code is the check.
fn copy(root: &Path, to: &Path, control: &Control, progress: Progress) -> Result<()> {
    fs::create_dir(to)?;
    progress(crate::i18n::format(
        "system.copy",
        &[root.display().to_string(), to.display().to_string()],
    ));
    crate::system_ops::robocopy(root, to, crate::system_ops::CopyMode::Prepare, control)?;
    // Finish durability before /MOVE is allowed to remove a source name. One volume
    // flush needs administrator rights; otherwise each copied file is flushed.
    if p::flush_volume(to).is_ok() {
        return Ok(());
    }
    let mut stack = vec![to.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            control.check()?;
            let entry = entry?;
            let path = entry.path();
            let m = entry.metadata()?;
            if p::reparse(&m) {
                continue;
            }
            if m.is_dir() {
                stack.push(path);
                continue;
            }
            let original = m.permissions();
            if original.readonly() {
                let mut writable = original.clone();
                writable.set_readonly(false);
                fs::set_permissions(&path, writable)?;
            }
            let result = (|| -> Result<()> {
                for stream in stream_names(&path)? {
                    OpenOptions::new()
                        .write(true)
                        .open(stream_path(&path, &stream))?
                        .sync_all()?;
                }
                Ok(())
            })();
            let restored = fs::set_permissions(&path, original);
            result?;
            restored?;
        }
    }
    Ok(())
}
fn delete_object(path: &Path, id: p::Identity, readonly: bool) -> Result<()> {
    let f = p::native_file(
        path,
        0x00010000 | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
        FILE_SHARE_READ,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    if p::identity_file(&f)?.0 != id {
        return fail(crate::i18n::t("msg.ea3fff43e3"));
    }
    if readonly {
        let mut info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                f.as_raw_handle() as _,
                FileBasicInfo,
                &mut info as *mut _ as _,
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        info.FileAttributes &= !FILE_ATTRIBUTE_READONLY;
        if unsafe {
            SetFileInformationByHandle(
                f.as_raw_handle() as _,
                FileBasicInfo,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            f.as_raw_handle() as _,
            FileDispositionInfo,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
/// Retires the source with robocopy /MOVE /IS /NOCOPY /XC /XN /XO: it removes only source
/// files equal in size and time to the committed copy and never writes the copy. Anything
/// left behind (not copied, changed or new) stops the migration before the link exists.
fn cleanup_system(
    root: &Path,
    reference: &Path,
    control: &Control,
    progress: Progress,
) -> Result<()> {
    progress(crate::i18n::format(
        "system.move",
        &[root.display().to_string(), reference.display().to_string()],
    ));
    crate::system_ops::robocopy(
        root,
        reference,
        crate::system_ops::CopyMode::FinishMove,
        control,
    )?;
    let mut directories = leftovers(root)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        control.check()?;
        match fs::symlink_metadata(&path) {
            Ok(_) => crate::system_ops::remove_empty(&path, control)?,
            Err(e) if gone(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    progress(crate::i18n::t("msg.0853e24b6c").into());
    Ok(())
}
fn gone(e: &(dyn std::error::Error + 'static)) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|e| matches!(e.raw_os_error(), Some(2 | 3)))
}
/// Directories robocopy /MOVE left behind. It removes the files and the directories it
/// emptied, which may still be vanishing while other processes close their handles, so a
/// name that is already gone counts as removed. Any file or link left is residue.
fn leftovers(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let children = match fs::read_dir(&dir) {
            Ok(c) => c,
            Err(e) if gone(&e) => continue,
            Err(e) => return Err(e.into()),
        };
        for child in children {
            let child = match child {
                Ok(c) => c,
                Err(e) if gone(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            let m = child.metadata()?;
            if !m.is_dir() || p::reparse(&m) {
                return fail(crate::i18n::format(
                    "system.residue_at",
                    &[child.path().display().to_string()],
                ));
            }
            stack.push(child.path());
        }
        dirs.push(dir);
    }
    Ok(dirs)
}
/// Rebuilds the original name from the committed copy. Held source content is never
/// overwritten: robocopy only fills names missing from the held directory.
fn restore_system_source(
    store: &Path,
    r: &mut Record,
    control: &Control,
    progress: Progress,
) -> Result<()> {
    if !absent(&r.source) {
        return fail(crate::i18n::t("system.changed"));
    }
    ensure_identity(
        &r.target,
        r.destination_id.ok_or(crate::i18n::t("system.changed"))?,
    )?;
    if absent(&r.hold) {
        phase(store, r, "rollback_copy_intent")?;
        copy(&r.target, &r.hold, control, progress)?;
    } else {
        // An interrupted rollback copy has a fresh directory ID; only that phase may
        // continue it. Otherwise the held directory must be the original.
        if r.phase != "rollback_copy_intent" {
            ensure_identity(&r.hold, r.source_id)?;
        }
        crate::system_ops::robocopy(
            &r.target,
            &r.hold,
            crate::system_ops::CopyMode::FillMissing,
            control,
        )?;
    }
    r.source_id = p::identity(&r.hold)?;
    phase(store, r, "rollback_rename_intent")?;
    p::rename(&r.hold, &r.source)?;
    phase(store, r, "rolled_back")
}

pub fn history(store: &Path) -> Result<Vec<Record>> {
    let mut out = vec![];
    if !store.exists() {
        return Ok(out);
    }
    for f in fs::read_dir(store)? {
        let p = f?.path();
        if p.extension().is_some_and(|e| e == "json")
            && p.file_stem().is_some_and(|s| {
                let s = s.to_string_lossy();
                s.split('-').count() == 2 && s.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            })
        {
            out.push(checked_read::<Record>(&p)?);
        }
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}
/// `.savemycdrive-<id>.stage/.hold` directories beside known sources and targets that no
/// open transaction owns: interrupted runs from other state directories, or copies kept
/// on purpose after a rollback. Listed for the user to inspect; never deleted here.
pub fn leftover_dirs(store: &Path) -> Result<Vec<PathBuf>> {
    let records = history(store)?;
    let mut parents: Vec<PathBuf> = vec![];
    for r in &records {
        for p in [&r.stage, &r.hold] {
            if let Some(parent) = p.parent() {
                if !parents.iter().any(|x| p::same(x, parent)) {
                    parents.push(parent.to_path_buf());
                }
            }
        }
    }
    let mut found = vec![];
    for parent in parents {
        let Ok(entries) = fs::read_dir(&parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name
                .strip_prefix(".savemycdrive-")
                .and_then(|n| n.strip_suffix(".stage").or_else(|| n.strip_suffix(".hold")))
            else {
                continue;
            };
            let open = records.iter().any(|r| {
                r.id == id && !["completed", "restored", "rolled_back"].contains(&r.phase.as_str())
            });
            if !open && entry.file_type().is_ok_and(|t| t.is_dir()) {
                found.push(entry.path());
            }
        }
    }
    Ok(found)
}
fn pending(store: &Path) -> Result<()> {
    for r in history(store)? {
        if !["completed", "restored", "rolled_back"].contains(&r.phase.as_str()) {
            return fail(crate::i18n::format(
                "msg.5734e7084d",
                &[format!("{}", r.id), format!("{}", r.phase)],
            ));
        }
    }
    Ok(())
}
pub fn migrate(
    plan: &Plan,
    store: &Path,
    control: &Control,
    progress: Progress,
) -> Result<Vec<Record>> {
    let _lock = lock_store(store)?;
    let _store_pins = p::pin_ancestors(store)?;
    pending(store)?;
    validate_plan(plan, store)?;
    let mut out = vec![];
    let mut batch = crate::batch::Tracker::start(
        store,
        "migrate",
        plan.items.iter().map(|i| i.source.clone()),
    )?;
    for (index, item) in plan.items.iter().enumerate() {
        batch.begin(index)?;
        control.check()?;
        validate_paths(&item.source, &item.target, store, false)?;
        handles::release(&[item.source.clone()], &control.options, store, progress)?;
        let parent = item
            .source
            .parent()
            .ok_or(crate::i18n::t("msg.4d24fe25b3"))?;
        let _source_pins = p::pin_ancestors(parent)?;
        let target_parent = item
            .target
            .parent()
            .ok_or(crate::i18n::t("msg.cbe405d02b"))?;
        let _target_existing_pins = p::pin_ancestors(target_parent)?;
        fs::create_dir_all(target_parent)?;
        let _target_pins = p::pin_ancestors(target_parent)?;
        validate_paths(&item.source, &item.target, store, false)?;
        let id = now()?;
        let mut r = Record {
            version: 1,
            id: id.clone(),
            kind: "migrate".into(),
            source: item.source.clone(),
            target: item.target.clone(),
            stage: item
                .target
                .parent()
                .ok_or(crate::i18n::t("msg.cbe405d02b"))?
                .join(format!(".savemycdrive-{id}.stage")),
            hold: parent.join(format!(".savemycdrive-{id}.hold")),
            source_id: p::identity(&item.source)?,
            origin_id: p::identity(&item.source)?,
            destination_id: None,
            phase: "prepared".into(),
            error: None,
        };
        save(store, &r)?;
        batch.transaction(index, &r.id)?;
        fault("prepared");
        let result = (|| -> Result<()> {
            // A directory still in use fails at the switch after a full copy; stop first.
            let users = handles::users(std::slice::from_ref(&r.source));
            if !users.is_empty() {
                return fail(format!(
                    "{}{}",
                    handles::advice(&users),
                    crate::i18n::t("handles.users_stop")
                ));
            }
            let (bytes, count) = survey(&r.source, control, progress)?;
            if count == 0 {
                return fail(crate::i18n::t("msg.bd053e736a"));
            }
            if p::free_space(&r.target)? < bytes.saturating_add(64 * 1024 * 1024) {
                return fail(crate::i18n::t("msg.4e024f5f24"));
            }
            fs::create_dir_all(r.target.parent().unwrap())?;
            phase(store, &mut r, "copying")?;
            copy(&r.source, &r.stage, control, progress)?;
            r.destination_id = Some(p::identity(&r.stage)?);
            phase(store, &mut r, "verified")?;
            phase(store, &mut r, "target_commit_intent")?;
            p::rename(&r.stage, &r.target)?;
            phase(store, &mut r, "target_ready")?;
            ensure_identity(&r.source, r.source_id)?;
            phase(store, &mut r, "source_rename_intent")?;
            p::rename(&r.source, &r.hold)?;
            phase(store, &mut r, "source_held")?;
            phase(store, &mut r, "source_cleanup_intent")?;
            cleanup_system(&r.hold, &r.target, control, progress)?;
            phase(store, &mut r, "source_cleaned")?;
            phase(store, &mut r, "link_intent")?;
            crate::system_ops::junction(&r.source, &r.target, control)?;
            p::check_junction(&r.source, &r.target)?;
            phase(store, &mut r, "linked")?;
            phase(store, &mut r, "completed")?;
            Ok(())
        })();
        if let Err(e) = result {
            let primary = crate::i18n::describe_error(e.as_ref());
            if !control.cancel.load(Ordering::Relaxed)
                && absent(&r.source)
                && r.destination_id.is_some()
            {
                if let Err(rollback) = restore_system_source(store, &mut r, control, progress) {
                    r.error = Some(format!(
                        "{primary}; {}",
                        crate::i18n::describe_error(rollback.as_ref())
                    ));
                }
            }
            if r.error.is_none() {
                r.error = Some(primary);
            }
            // Preflight rejections (nested links, external hard links, free space) happen
            // before any copy starts; leaving them open would block every later transaction.
            if r.phase == "prepared" {
                r.phase = "rolled_back".into();
            }
            save(store, &r)?;
            return fail(crate::i18n::format(
                "msg.868a502327",
                &[crate::i18n::describe_error(e.as_ref()), format!("{}", r.id)],
            ));
        }
        progress(crate::i18n::format(
            "msg.b6aedcabee",
            &[format!("{}", item.name)],
        ));
        batch.finish(index)?;
        out.push(r);
    }
    batch.complete()?;
    Ok(out)
}
fn link_target(source: &Path) -> Result<PathBuf> {
    let m = fs::symlink_metadata(source)?;
    if !p::reparse(&m) || !(junction::exists(source)? || m.file_type().is_symlink()) {
        return fail(crate::i18n::t("msg.5e097b7fc5"));
    }
    if m.file_attributes() & FILE_ATTRIBUTE_SYSTEM != 0 {
        return fail(crate::i18n::t("msg.6f113d89f1"));
    }
    let raw = junction::get_target(source).or_else(|_| fs::read_link(source))?;
    if raw.to_string_lossy().contains("Volume{") {
        return fail(crate::i18n::t("msg.03d3db0cfc"));
    }
    let t = p::absolute(source)?;
    if !t.is_dir() {
        return fail(crate::i18n::t("msg.9a05cf980f"));
    }
    Ok(t)
}
/// Programs using the selected directories. A link's real directory is checked too,
/// because handles opened through a link resolve to the real path.
pub fn in_use(sources: &[PathBuf]) -> Vec<handles::User> {
    let mut roots = sources.to_vec();
    roots.extend(sources.iter().filter_map(|s| junction::get_target(s).ok()));
    handles::users(&roots)
}
pub fn restore(
    source: &Path,
    store: &Path,
    control: &Control,
    progress: Progress,
) -> Result<Record> {
    restore_tracked(source, store, control, progress, &mut |_| Ok(()))
}
fn restore_tracked(
    source: &Path,
    store: &Path,
    control: &Control,
    progress: Progress,
    on_prepared: &mut dyn FnMut(&Record) -> Result<()>,
) -> Result<Record> {
    let _lock = lock_store(store)?;
    let _store_pins = p::pin_ancestors(store)?;
    pending(store)?;
    let source = source.to_path_buf();
    let target = link_target(&source)?;
    validate_paths(&source, &target, store, true)?;
    handles::release(&[target.clone()], &control.options, store, progress)?;
    let id = now()?;
    let parent = source.parent().ok_or(crate::i18n::t("msg.469ea31255"))?;
    let _source_pins = p::pin_ancestors(parent)?;
    let _target_pins = p::pin_ancestors(&target)?;
    validate_paths(&source, &target, store, true)?;
    let mut r = Record {
        version: 1,
        id: id.clone(),
        kind: "restore".into(),
        source: source.clone(),
        target: target.clone(),
        stage: parent.join(format!(".savemycdrive-{id}.stage")),
        hold: parent.join(format!(".savemycdrive-{id}.hold")),
        source_id: p::identity(&source)?,
        origin_id: p::identity(&target)?,
        destination_id: None,
        phase: "prepared".into(),
        error: None,
    };
    save(store, &r)?;
    on_prepared(&r)?;
    fault("prepared");
    let result = (|| -> Result<()> {
        let users = in_use(std::slice::from_ref(&source));
        if !users.is_empty() {
            return fail(format!(
                "{}{}",
                handles::advice(&users),
                crate::i18n::t("handles.users_stop")
            ));
        }
        let (bytes, _) = survey(&target, control, progress)?;
        if p::free_space(parent)? < bytes.saturating_add(64 * 1024 * 1024) {
            return fail(crate::i18n::t("msg.79e02a1c25"));
        }
        phase(store, &mut r, "copying")?;
        copy(&target, &r.stage, control, progress)?;
        r.destination_id = Some(p::identity(&r.stage)?);
        phase(store, &mut r, "verified")?;
        ensure_identity(&source, r.source_id)?;
        ensure_identity(&target, r.origin_id)?;
        if !p::same(&link_target(&source)?, &target) {
            return fail(crate::i18n::t("msg.f20abebaf8"));
        }
        phase(store, &mut r, "source_rename_intent")?;
        p::rename(&source, &r.hold)?;
        phase(store, &mut r, "source_held")?;
        phase(store, &mut r, "materialize_intent")?;
        p::rename(&r.stage, &source)?;
        phase(store, &mut r, "materialized")?;
        ensure_identity(&source, r.destination_id.unwrap())?;
        ensure_identity(&r.hold, r.source_id)?;
        delete_object(&r.hold, r.source_id, false)?;
        phase(store, &mut r, "restored")?;
        progress(crate::i18n::format(
            "msg.349df44a6e",
            &[format!("{}", target.display())],
        ));
        Ok(())
    })();
    if let Err(e) = result {
        r.error = Some(crate::i18n::describe_error(e.as_ref()));
        // Nothing was copied or renamed yet; an open record would block later work.
        if r.phase == "prepared" {
            r.phase = "rolled_back".into();
        }
        save(store, &r)?;
        return fail(crate::i18n::format(
            "msg.45d9c68408",
            &[crate::i18n::describe_error(e.as_ref()), format!("{}", r.id)],
        ));
    }
    Ok(r)
}
pub fn restore_many(
    sources: &[PathBuf],
    store: &Path,
    control: &Control,
    progress: Progress,
) -> Result<Vec<Record>> {
    if sources.is_empty() {
        return fail(crate::i18n::t("msg.c42783dc92"));
    }
    let mut batch = crate::batch::Tracker::start(store, "restore", sources.iter().cloned())?;
    let mut records = vec![];
    for (index, source) in sources.iter().enumerate() {
        batch.begin(index)?;
        control.check()?;
        let record = restore_tracked(source, store, control, progress, &mut |r| {
            batch.transaction(index, &r.id)
        })?;
        batch.finish(index)?;
        records.push(record);
    }
    batch.complete()?;
    Ok(records)
}
pub fn recover(store: &Path, id: &str, control: &Control, progress: Progress) -> Result<Record> {
    let _lock = lock_store(store)?;
    let _store_pins = p::pin_ancestors(store)?;
    let mut r: Record = checked_read(&record_path(store, id)?)?;
    if r.id != id || r.version != 1 {
        return fail(crate::i18n::t("msg.5a57d85fdb"));
    }
    if ["completed", "restored", "rolled_back"].contains(&r.phase.as_str()) {
        return Ok(r);
    }
    // Never replay handle closures, nor use recorded PIDs/handle values after restart.
    let parent = r.source.parent().ok_or(crate::i18n::t("msg.bcc73003f9"))?;
    if !p::same(
        &r.hold,
        &parent.join(format!(".savemycdrive-{}.hold", r.id)),
    ) || protected(&r.source)
        || protected(&r.target)
        || p::overlap(&r.source, store)
        || p::overlap(&r.target, store)
    {
        return fail(crate::i18n::t("msg.c1a31f740e"));
    }
    let _source_pins = p::pin_ancestors(parent)?;
    let target_parent = r.target.parent().ok_or(crate::i18n::t("msg.cbe405d02b"))?;
    let _target_pins = p::pin_ancestors(target_parent)?;
    p::ntfs(parent)?;
    p::ntfs(target_parent)?;
    shared(&[&r.source, &r.target])?;
    control.check()?;
    let result = (|| -> Result<()> {
        if r.kind == "migrate" {
            if !absent(&r.source) && junction::exists(&r.source).unwrap_or(false) {
                p::check_junction(&r.source, &r.target)?;
                ensure_identity(
                    &r.target,
                    r.destination_id.ok_or(crate::i18n::t("msg.fc6987862b"))?,
                )?;
                if !absent(&r.hold) {
                    return fail(crate::i18n::t("system.residue"));
                }
                phase(store, &mut r, "completed")?;
                return Ok(());
            }
            if absent(&r.source) {
                return restore_system_source(store, &mut r, control, progress);
            }
            ensure_identity(&r.source, r.source_id)?;
            if !absent(&r.hold) {
                return fail(crate::i18n::t("system.changed"));
            }
            phase(store, &mut r, "rolled_back")?;
            progress(crate::i18n::format(
                "msg.f9387e589a",
                &[
                    format!("{}", r.stage.display()),
                    format!("{}", r.target.display()),
                ],
            ));
            return Ok(());
        }
        if r.kind != "restore" {
            return fail(crate::i18n::t("msg.ff6b6487ca"));
        }
        if !absent(&r.hold) {
            ensure_identity(&r.hold, r.source_id)?;
            if absent(&r.source) {
                p::rename(&r.hold, &r.source)?;
                phase(store, &mut r, "rolled_back")?;
                return Ok(());
            }
            ensure_identity(
                &r.source,
                r.destination_id.ok_or(crate::i18n::t("msg.a750c6eaf8"))?,
            )?;
            delete_object(&r.hold, r.source_id, false)?;
            phase(store, &mut r, "restored")?;
            return Ok(());
        }
        if p::identity(&r.source)? == r.source_id {
            if !p::same(&link_target(&r.source)?, &r.target) {
                return fail(crate::i18n::t("msg.a5d4f2d5ae"));
            }
            phase(store, &mut r, "rolled_back")?;
            return Ok(());
        }
        ensure_identity(
            &r.source,
            r.destination_id.ok_or(crate::i18n::t("msg.430dd363ef"))?,
        )?;
        phase(store, &mut r, "restored")?;
        Ok(())
    })();
    if let Err(e) = result {
        r.error = Some(crate::i18n::describe_error(e.as_ref()));
        save(store, &r)?;
        return Err(e);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::current_dir()
            .unwrap()
            .join(".build/unit-tests")
            .join(format!("fp-test-{}", now().unwrap()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn rule_documents_do_not_pollute_transaction_history() {
        let root = temp();
        fs::write(root.join("rules.json"), b"not a transaction").unwrap();
        assert!(history(&root).unwrap().is_empty());
        fs::write(root.join("123-456.json"), b"corrupt transaction").unwrap();
        assert!(history(&root).is_err());
    }
    #[test]
    fn incomplete_batch_is_not_success() {
        let root = temp();
        {
            let mut tracker =
                crate::batch::Tracker::start(&root, "migrate", [root.join("a"), root.join("b")])
                    .unwrap();
            tracker.begin(0).unwrap();
            tracker.finish(0).unwrap();
        }
        let batches = crate::batch::history(&root).unwrap();
        assert_eq!(batches[0].phase, "interrupted");
        assert_eq!(batches[0].items[0].status, "completed");
        assert_eq!(batches[0].items[1].status, "pending");
    }
    #[test]
    fn parent_pins_prevent_rename() {
        let root = temp();
        let child = root.join("child");
        fs::create_dir(&child).unwrap();
        let pins = p::pin_ancestors(&child).unwrap();
        assert!(fs::rename(&child, root.join("other")).is_err());
        drop(pins);
        fs::rename(&child, root.join("other")).unwrap();
    }
    #[test]
    fn checksum_detects_tampering() {
        let p = temp().join("record");
        checked_write(&p, &vec![1, 2, 3]).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        v["payload"] = serde_json::json!([9]);
        fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(checked_read::<Vec<u32>>(&p).is_err());
    }
    #[test]
    fn read_guard_prevents_writes() {
        let p = temp().join("a");
        fs::write(&p, b"safe").unwrap();
        let _g = p::read_guard(&p).unwrap();
        assert!(fs::write(&p, b"bad").is_err());
    }
    #[test]
    fn path_boundary() {
        assert!(!p::within(Path::new(r"C:\foobar"), Path::new(r"C:\foo")));
        assert!(p::within(Path::new(r"C:\foo\bar"), Path::new(r"c:\FOO")));
    }
    #[test]
    fn invalid_record_id() {
        assert!(record_path(Path::new(r"E:\state"), "../bad").is_err());
    }
    #[test]
    fn options_are_off() {
        let o = handles::Options::default();
        assert!(!o.force_close_handles && !o.request_app_exit);
    }
    #[test]
    fn rejects_reserved_paths() {
        for s in [
            r"E:\safe\NUL",
            "E:\\safe\\bad\u{1f}name",
            "E:\\safe\\bad\"name",
            r"E:\safe\CON.txt",
            r"E:\safe\com1",
            r"E:\safe\LPT9.log",
            r"E:\safe\COM¹.txt",
            r"E:\safe\name.",
            r"E:\safe\name ",
            r"E:\safe\x:y",
            r"\\server\share",
            r"\\?\E:\safe",
            r"E:\safe\..\outside",
        ] {
            assert!(p::absolute(Path::new(s)).is_err(), "{s}");
        }
    }
    #[test]
    fn protected_directory_boundaries() {
        for s in [
            r"C:\Windows",
            r"D:\Windows\System32",
            r"C:\Program Files\tool",
            r"E:\ProgramData",
            r"C:\Users",
        ] {
            assert!(protected(Path::new(s)), "{s}");
        }
        assert!(!protected(Path::new(r"E:\some\WindowsCache")));
    }
    #[test]
    fn canceled_before_survey_preserves_source() {
        let root = temp();
        fs::write(root.join("a"), b"safe").unwrap();
        let c = Control::default();
        c.cancel.store(true, Ordering::Relaxed);
        assert!(survey(&root, &c, &|_| {}).is_err());
        assert_eq!(fs::read(root.join("a")).unwrap(), b"safe");
    }
    fn copy_fixture() -> (PathBuf, PathBuf) {
        let src = temp();
        fs::create_dir(src.join("empty")).unwrap();
        fs::write(src.join("样本.txt"), b"original").unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("dest-{}", now().unwrap()));
        copy(&src, &dest, &Control::default(), &|_| {}).unwrap();
        (src, dest)
    }
    const DATA: &str = "样本.txt";
    #[test]
    fn robocopy_preserves_content_empty_dirs_and_file_times() {
        let (src, dest) = copy_fixture();
        assert_eq!(
            fs::read(src.join(DATA)).unwrap(),
            fs::read(dest.join(DATA)).unwrap()
        );
        assert_eq!(
            fs::metadata(src.join(DATA)).unwrap().last_write_time(),
            fs::metadata(dest.join(DATA)).unwrap().last_write_time()
        );
        assert!(dest.join("empty").is_dir());
    }
    #[test]
    fn system_move_keeps_copy_and_removes_only_empty_source() {
        let (src, dest) = copy_fixture();
        cleanup_system(&src, &dest, &Control::default(), &|_| {}).unwrap();
        assert!(absent(&src));
        assert_eq!(fs::read(dest.join(DATA)).unwrap(), b"original");
        assert!(dest.join("empty").is_dir());
    }
    #[test]
    fn system_move_cancel_before_command_preserves_source() {
        let (src, dest) = copy_fixture();
        let control = Control::default();
        let marker = crate::i18n::format(
            "system.move",
            &[src.display().to_string(), dest.display().to_string()],
        );
        let progress = |message: String| {
            if message == marker {
                control.cancel.store(true, Ordering::Relaxed);
            }
        };
        assert!(cleanup_system(&src, &dest, &control, &progress).is_err());
        assert_eq!(fs::read(src.join(DATA)).unwrap(), b"original");
        assert_eq!(fs::read(dest.join(DATA)).unwrap(), b"original");
    }
    #[test]
    fn system_copy_locked_source_reports_error_without_deleting_data() {
        let src = temp();
        let file = src.join("locked.txt");
        fs::write(&file, b"retain locked source").unwrap();
        let _guard = p::native_file(&file, GENERIC_READ, 0, FILE_FLAG_OPEN_REPARSE_POINT).unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("locked-{}", now().unwrap()));
        assert!(copy(&src, &dest, &Control::default(), &|_| {}).is_err());
        drop(_guard);
        assert_eq!(fs::read(&file).unwrap(), b"retain locked source");
    }
    #[test]
    fn system_move_keeps_changed_new_and_uncopied_source() {
        for change in 0..3 {
            let (src, dest) = copy_fixture();
            match change {
                0 => fs::write(src.join("new.txt"), b"retain new source").unwrap(),
                1 => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    fs::write(src.join(DATA), b"changed source").unwrap()
                }
                _ => fs::remove_file(dest.join(DATA)).unwrap(),
            }
            assert!(cleanup_system(&src, &dest, &Control::default(), &|_| {}).is_err());
            // Unchanged files may be retired; whatever robocopy could not match stays.
            match change {
                0 => {
                    assert_eq!(fs::read(src.join("new.txt")).unwrap(), b"retain new source");
                    assert_eq!(fs::read(dest.join(DATA)).unwrap(), b"original");
                }
                1 => {
                    assert_eq!(fs::read(src.join(DATA)).unwrap(), b"changed source");
                    assert_eq!(fs::read(dest.join(DATA)).unwrap(), b"original");
                }
                _ => assert_eq!(fs::read(src.join(DATA)).unwrap(), b"original"),
            }
        }
    }
    #[test]
    fn system_move_retains_late_changed_and_unknown_source() {
        let (src, dest) = copy_fixture();
        let marker = crate::i18n::format(
            "system.move",
            &[src.display().to_string(), dest.display().to_string()],
        );
        let invoked = std::cell::Cell::new(false);
        let progress = |message: String| {
            if message == marker {
                invoked.set(true);
                std::thread::sleep(std::time::Duration::from_millis(20));
                fs::write(src.join(DATA), b"late change").unwrap();
                fs::write(src.join("late.txt"), b"retain late source").unwrap();
            }
        };
        assert!(cleanup_system(&src, &dest, &Control::default(), &progress).is_err());
        assert!(invoked.get());
        assert_eq!(
            fs::read(src.join("late.txt")).unwrap(),
            b"retain late source"
        );
        assert_eq!(fs::read(src.join(DATA)).unwrap(), b"late change");
        assert_eq!(fs::read(dest.join(DATA)).unwrap(), b"original");
    }
    fn rollback_fixture() -> (Record, PathBuf) {
        let (src, dest) = copy_fixture();
        let id = now().unwrap();
        let record = Record {
            version: 1,
            id: id.clone(),
            kind: "migrate".into(),
            source: src.clone(),
            target: dest.clone(),
            stage: dest
                .parent()
                .unwrap()
                .join(format!(".savemycdrive-{id}.stage")),
            hold: src
                .parent()
                .unwrap()
                .join(format!(".savemycdrive-{id}.hold")),
            source_id: p::identity(&src).unwrap(),
            origin_id: p::identity(&src).unwrap(),
            destination_id: Some(p::identity(&dest).unwrap()),
            phase: "source_held".into(),
            error: None,
        };
        fs::rename(&src, &record.hold).unwrap();
        let store = temp();
        save(&store, &record).unwrap();
        (record, store)
    }
    #[test]
    fn rollback_refills_partial_source_and_keeps_new_source_data() {
        let (mut record, store) = rollback_fixture();
        fs::remove_file(record.hold.join(DATA)).unwrap();
        fs::write(record.hold.join("extra.txt"), b"retain extra").unwrap();
        restore_system_source(&store, &mut record, &Control::default(), &|_| {}).unwrap();
        assert_eq!(record.phase, "rolled_back");
        assert_eq!(fs::read(record.source.join(DATA)).unwrap(), b"original");
        assert_eq!(
            fs::read(record.source.join("extra.txt")).unwrap(),
            b"retain extra"
        );
        assert_eq!(fs::read(record.target.join(DATA)).unwrap(), b"original");
        assert_eq!(
            recover(&store, &record.id, &Control::default(), &|_| {})
                .unwrap()
                .phase,
            "rolled_back"
        );
    }
    #[test]
    fn rollback_never_overwrites_changed_source() {
        let (mut record, store) = rollback_fixture();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(record.hold.join(DATA), b"changed source").unwrap();
        restore_system_source(&store, &mut record, &Control::default(), &|_| {}).unwrap();
        assert_eq!(
            fs::read(record.source.join(DATA)).unwrap(),
            b"changed source"
        );
        assert_eq!(fs::read(record.target.join(DATA)).unwrap(), b"original");
    }
    #[test]
    fn rollback_rebuilds_source_after_removal_and_interrupted_reconstruction() {
        for interrupted in [false, true] {
            let (mut record, store) = rollback_fixture();
            cleanup_system(&record.hold, &record.target, &Control::default(), &|_| {}).unwrap();
            phase(&store, &mut record, "source_cleaned").unwrap();
            if interrupted {
                phase(&store, &mut record, "rollback_copy_intent").unwrap();
                fs::create_dir(&record.hold).unwrap();
                fs::create_dir(record.hold.join("empty")).unwrap();
            }
            restore_system_source(&store, &mut record, &Control::default(), &|_| {}).unwrap();
            assert_eq!(record.phase, "rolled_back");
            assert_eq!(
                recover(&store, &record.id, &Control::default(), &|_| {})
                    .unwrap()
                    .phase,
                "rolled_back"
            );
            assert_eq!(fs::read(record.source.join(DATA)).unwrap(), b"original");
            assert!(record.source.join("empty").is_dir());
            assert_eq!(fs::read(record.target.join(DATA)).unwrap(), b"original");
        }
    }
    #[test]
    fn alternate_streams_are_copied() {
        let src = temp();
        let f = src.join("a");
        fs::write(&f, b"default").unwrap();
        fs::write(stream_path(&f, ":metadata:$DATA"), b"alternate data").unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("ads-{}", now().unwrap()));
        copy(&src, &dest, &Control::default(), &|_| {}).unwrap();
        assert_eq!(
            fs::read(stream_path(&dest.join("a"), ":metadata:$DATA")).unwrap(),
            b"alternate data"
        );
    }
    #[test]
    fn readonly_files_are_copied_and_retired() {
        let src = temp();
        let f = src.join("a");
        fs::write(&f, b"readonly").unwrap();
        let mut perm = fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&f, perm).unwrap();
        let c = Control::default();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("readonly-{}", now().unwrap()));
        copy(&src, &dest, &c, &|_| {}).unwrap();
        cleanup_system(&src, &dest, &c, &|_| {}).unwrap();
        assert!(absent(&src));
        assert_eq!(fs::read(dest.join("a")).unwrap(), b"readonly");
        assert!(fs::metadata(dest.join("a"))
            .unwrap()
            .permissions()
            .readonly());
    }
    #[test]
    fn nested_junction_is_rejected_and_not_traversed() {
        let a = temp();
        let outside = temp();
        fs::write(outside.join("safe"), b"untouched").unwrap();
        p::create_junction(&a.join("link"), &outside).unwrap();
        assert!(survey(&a, &Control::default(), &|_| {}).is_err());
        assert_eq!(fs::read(outside.join("safe")).unwrap(), b"untouched");
    }
    #[test]
    fn survey_counts_bytes_without_opening_files() {
        let a = temp();
        fs::create_dir(a.join("d")).unwrap();
        fs::write(a.join("d").join("x"), b"12345").unwrap();
        let locked = a.join("locked");
        fs::write(&locked, b"123").unwrap();
        let _guard =
            p::native_file(&locked, GENERIC_READ, 0, FILE_FLAG_OPEN_REPARSE_POINT).unwrap();
        assert_eq!(survey(&a, &Control::default(), &|_| {}).unwrap(), (8, 3));
    }
}
