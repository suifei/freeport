//! Recoverable directory transactions. No recursive deletion or cross-volume atomicity claims.
use crate::{fail, handles, inventory::Row, platform as p, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    os::windows::{
        fs::{symlink_dir, symlink_file, MetadataExt},
        io::AsRawHandle,
    },
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stream {
    name: String,
    bytes: u64,
    hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Directory,
    File { streams: Vec<Stream>, links: u32 },
    Link { target: PathBuf, directory: bool },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    id: p::Identity,
    kind: Kind,
    attrs: u32,
    created: u64,
    modified: u64,
    security: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Tree {
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    source: Tree,
    destination: Tree,
}

pub fn default_store() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("fp.exe"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("freeport-state")
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
    if std::env::var("FP_TEST_FAIL_AT").ok().as_deref() == Some(s) {
        std::process::abort();
    }
    let _ = s;
}
fn lock_store(store: &Path) -> Result<File> {
    fs::create_dir_all(store)?;
    p::lock(&store.join(".lock"))
}
fn manifest_path(store: &Path, id: &str) -> PathBuf {
    store.join(format!("{id}.manifest"))
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
    if ids.is_empty() {
        return fail(crate::i18n::t("msg.c42783dc92"));
    }
    let destination = p::absolute(destination)?;
    let mut items = vec![];
    for id in ids {
        let row = rows
            .iter()
            .find(|r| &r.candidate.id == id)
            .ok_or(crate::i18n::t("msg.5f5c222eab"))?;
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
    validate_plan(&plan, &default_store())?;
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

fn hash(f: &mut File) -> Result<String> {
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
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
    names.sort();
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
fn file_kind_checked(
    path: &Path,
    keep: bool,
    expected: Option<p::Identity>,
    stable: bool,
) -> Result<(Kind, Vec<File>)> {
    let mut streams = vec![];
    let mut guards = vec![];
    let mut links = 1;
    for name in stream_names(path)? {
        if !name.ends_with(":$DATA") {
            return fail(crate::i18n::t("msg.405e40251e"));
        }
        let mut f = if stable {
            p::stable_read_guard(&stream_path(path, &name))?
        } else {
            p::read_guard(&stream_path(path, &name))?
        };
        let identity = p::identity_file(&f)?;
        if expected.is_some_and(|id| id != identity.0) {
            return fail(crate::i18n::t("msg.ea3fff43e3"));
        }
        let m = f.metadata()?;
        links = identity.1;
        let sha = hash(&mut f)?;
        streams.push(Stream {
            name,
            bytes: m.len(),
            hash: sha,
        });
        if keep {
            guards.push(f);
        }
    }
    Ok((Kind::File { streams, links }, guards))
}
fn file_kind(path: &Path, keep: bool) -> Result<(Kind, Vec<File>)> {
    file_kind_checked(path, keep, None, false)
}
fn tree(root: &Path, control: &Control, progress: Progress) -> Result<Tree> {
    let mut entries = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        control.check()?;
        if path.to_str().is_none() {
            return fail(crate::i18n::t("msg.c48c45a82d"));
        }
        let m = fs::symlink_metadata(&path)?;
        let attrs = m.file_attributes();
        let rel = path.strip_prefix(root)?.to_path_buf();
        let id = p::identity(&path)?;
        let kind = if p::reparse(&m) {
            if !m.file_type().is_symlink() {
                return fail(crate::i18n::format(
                    "msg.df3664d356",
                    &[format!("{}", path.display())],
                ));
            }
            let target = fs::read_link(&path)?;
            if target.is_absolute() || !p::within(&p::absolute(&path)?, &p::absolute(root)?) {
                return fail(crate::i18n::t("msg.2e8cdf90ca"));
            }
            Kind::Link {
                target,
                directory: fs::metadata(&path)?.is_dir(),
            }
        } else if attrs
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
            for entry in fs::read_dir(&path)? {
                stack.push(entry?.path());
            }
            Kind::Directory
        } else {
            file_kind(&path, false)?.0
        };
        let security = if matches!(kind, Kind::Link { .. }) {
            vec![]
        } else {
            p::security(&path)?
        };
        let after = fs::symlink_metadata(&path)?;
        if m.last_write_time() != after.last_write_time() || p::identity(&path)? != id {
            return fail(crate::i18n::t("msg.f96a3a6e3e"));
        }
        entries.push(Entry {
            path: rel,
            id,
            kind,
            attrs,
            created: m.creation_time(),
            modified: m.last_write_time(),
            security,
        });
        if entries.len() % 1000 == 0 {
            progress(crate::i18n::format(
                "msg.3e664d9989",
                &[format!("{}", entries.len())],
            ));
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut groups = BTreeMap::new();
    for e in &entries {
        if matches!(e.kind, Kind::File { .. }) {
            *groups.entry(e.id).or_insert(0u32) += 1;
        }
    }
    for e in &entries {
        if let Kind::File { links, .. } = &e.kind {
            if *links != groups[&e.id] {
                return fail(crate::i18n::format(
                    "msg.b62943c6a9",
                    &[format!("{}", root.join(&e.path).display())],
                ));
            }
        }
    }
    Ok(Tree { entries })
}
fn same_kind(a: &Kind, b: &Kind) -> bool {
    match (a, b) {
        (
            Kind::File {
                streams: a,
                links: x,
            },
            Kind::File {
                streams: b,
                links: y,
            },
        ) => a == b && x == y,
        _ => a == b,
    }
}
fn same_content(a: &Kind, b: &Kind) -> bool {
    match (a, b) {
        (Kind::File { streams: a, .. }, Kind::File { streams: b, .. }) => a == b,
        _ => a == b,
    }
}
fn security_equal(a: &[u8], b: &[u8]) -> bool {
    let normalize = |x: &[u8]| {
        let mut v = x.to_vec();
        if v.len() >= 4 {
            let mut c = u16::from_le_bytes([v[2], v[3]]);
            c &= !(0x0100 | 0x0400);
            v[2..4].copy_from_slice(&c.to_le_bytes());
        }
        v
    };
    normalize(a) == normalize(b)
}
fn verify(a: &Tree, b: &Tree) -> Result<()> {
    if a.entries.len() != b.entries.len() {
        return fail(crate::i18n::t("msg.24703c1fd9"));
    }
    for (x, y) in a.entries.iter().zip(&b.entries) {
        if x.path != y.path
            || !same_kind(&x.kind, &y.kind)
            || !security_equal(&x.security, &y.security)
            || x.created != y.created
            || x.modified != y.modified
            || (x.attrs & FILE_ATTRIBUTE_READONLY) != (y.attrs & FILE_ATTRIBUTE_READONLY)
        {
            return fail(crate::i18n::format(
                "msg.0b439ec61c",
                &[format!("{}", x.path.display())],
            ));
        }
    }
    Ok(())
}
struct CopyContext<'a> {
    control: &'a Control,
    progress: Progress<'a>,
    last: std::time::Instant,
}
unsafe extern "system" fn copy_progress(
    total: i64,
    done: i64,
    _: i64,
    _: i64,
    _: u32,
    _: u32,
    _: HANDLE,
    _: HANDLE,
    data: *const std::ffi::c_void,
) -> u32 {
    let c = &mut *(data as *mut CopyContext);
    if c.control.cancel.load(Ordering::Relaxed) {
        return PROGRESS_CANCEL;
    }
    if c.last.elapsed() > std::time::Duration::from_millis(250) {
        (c.progress)(crate::i18n::format(
            "msg.29d1d66a2b",
            &[
                format!("{}", crate::human(done.max(0) as u64)),
                format!("{}", crate::human(total.max(0) as u64)),
            ],
        ));
        c.last = std::time::Instant::now();
    }
    PROGRESS_CONTINUE
}
fn times(path: &Path, e: &Entry) -> Result<()> {
    let f = p::native_file(
        path,
        FILE_WRITE_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let mut info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    info.CreationTime = e.created as i64;
    info.LastWriteTime = e.modified as i64;
    info.FileAttributes = e.attrs
        & (FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_SYSTEM
            | FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_TEMPORARY
            | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED);
    if info.FileAttributes == 0 {
        info.FileAttributes = FILE_ATTRIBUTE_NORMAL;
    }
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
    Ok(())
}
fn bytes(tree: &Tree) -> u64 {
    let mut ids = std::collections::HashSet::new();
    tree.entries
        .iter()
        .filter(|e| ids.insert(e.id))
        .map(|e| match &e.kind {
            Kind::File { streams, .. } => streams.iter().map(|s| s.bytes).sum(),
            _ => 0,
        })
        .sum()
}
fn copy(root: &Path, to: &Path, tree: &Tree, control: &Control, progress: Progress) -> Result<()> {
    fs::create_dir(to)?;
    let mut context = CopyContext {
        control,
        progress,
        last: std::time::Instant::now(),
    };
    let mut groups = BTreeMap::new();
    for (i, e) in tree.entries.iter().enumerate() {
        control.check()?;
        let from = root.join(&e.path);
        let dest = to.join(&e.path);
        ensure_identity(&from, e.id)?;
        match &e.kind {
            Kind::Directory => {
                if !e.path.as_os_str().is_empty() {
                    fs::create_dir(&dest)?;
                }
            }
            Kind::Link { target, directory } => {
                if *directory {
                    symlink_dir(target, &dest)?;
                } else {
                    symlink_file(target, &dest)?;
                }
            }
            Kind::File { .. } => {
                if let Some(prior) = groups.get(&e.id) {
                    fs::hard_link(prior, &dest)?;
                } else {
                    let mut cancel = 0;
                    let ok = unsafe {
                        CopyFileExW(
                            p::path_wide(&from).as_ptr(),
                            p::path_wide(&dest).as_ptr(),
                            Some(copy_progress),
                            &mut context as *mut _ as _,
                            &mut cancel,
                            COPY_FILE_FAIL_IF_EXISTS,
                        )
                    };
                    if ok == 0 {
                        return fail(crate::i18n::format(
                            "msg.fef50b418b",
                            &[
                                format!("{}", from.display()),
                                crate::i18n::describe_error(&std::io::Error::last_os_error()),
                            ],
                        ));
                    }
                    groups.insert(e.id, dest.clone());
                    let original = fs::metadata(&dest)?.permissions();
                    if original.readonly() {
                        let mut writable = original.clone();
                        writable.set_readonly(false);
                        fs::set_permissions(&dest, writable)?;
                    }
                    for stream in stream_names(&dest)? {
                        OpenOptions::new()
                            .write(true)
                            .open(stream_path(&dest, &stream))?
                            .sync_all()?;
                    }
                    fs::set_permissions(&dest, original)?;
                }
            }
        }
        if !matches!(e.kind, Kind::Link { .. }) {
            p::set_security(&dest, &e.security)?;
        }
        if i % 100 == 0 {
            progress(crate::i18n::format(
                "msg.760b1c2616",
                &[
                    format!("{}", i + 1),
                    format!("{}", tree.entries.len()),
                    format!("{}", e.path.display()),
                ],
            ));
        }
    }
    for e in tree.entries.iter().rev() {
        times(&to.join(&e.path), e)?;
    }
    Ok(())
}
fn guards(root: &Path, relative_parent: Option<&Path>, expected: &Tree) -> Result<Vec<File>> {
    let mut result = vec![];
    let mut paths = vec![];
    if let Some(parent) = relative_parent {
        let mut at = PathBuf::new();
        paths.push(at.clone());
        for c in parent.components() {
            at.push(c);
            paths.push(at.clone());
        }
    }
    for rel in paths {
        let path = root.join(&rel);
        let expected = expected
            .entries
            .iter()
            .find(|e| e.path == rel)
            .ok_or(crate::i18n::t("msg.46592371dc"))?;
        let f = p::native_file(
            &path,
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        if p::identity_file(&f)?.0 != expected.id {
            return fail(crate::i18n::t("msg.c78ecaa390"));
        }
        result.push(f);
    }
    Ok(result)
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
fn cleanup(
    root: &Path,
    reference: &Path,
    manifest: &Manifest,
    control: &Control,
    progress: Progress,
) -> Result<()> {
    let mut entries = manifest.source.entries.clone();
    entries.sort_by_key(|e| std::cmp::Reverse(e.path.components().count()));
    for e in entries {
        control.check()?;
        let source = root.join(&e.path);
        if absent(&source) {
            continue;
        }
        let target = reference.join(&e.path);
        let dest = manifest
            .destination
            .entries
            .iter()
            .find(|d| d.path == e.path)
            .ok_or(crate::i18n::t("msg.4bf410bf83"))?;
        let _parents = guards(root, e.path.parent(), &manifest.source)?;
        let _target_parents = guards(reference, e.path.parent(), &manifest.destination)?;
        ensure_identity(&source, e.id)?;
        ensure_identity(&target, dest.id)?;
        match &e.kind {
            Kind::File { .. } => {
                let (actual, _source_guards) = file_kind_checked(&source, true, Some(e.id), false)?;
                let (copied, _target_guards) =
                    file_kind_checked(&target, true, Some(dest.id), true)?;
                if !same_content(&actual, &copied)
                    || !same_content(&actual, &e.kind)
                    || !security_equal(&p::security(&source)?, &e.security)
                    || !security_equal(&p::security(&target)?, &dest.security)
                {
                    return fail(crate::i18n::format(
                        "msg.c140938b38",
                        &[format!("{}", source.display())],
                    ));
                }
                delete_object(&source, e.id, e.attrs & FILE_ATTRIBUTE_READONLY != 0)?;
            }
            Kind::Link {
                target: expected, ..
            } => {
                if fs::read_link(&source)? != *expected || fs::read_link(&target)? != *expected {
                    return fail(crate::i18n::t("msg.425e9971c1"));
                }
                delete_object(&source, e.id, false)?;
            }
            Kind::Directory => {
                delete_object(&source, e.id, e.attrs & FILE_ATTRIBUTE_READONLY != 0)?;
            }
        }
    }
    progress(crate::i18n::t("msg.0853e24b6c").into());
    Ok(())
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
                .join(format!(".freeport-{id}.stage")),
            hold: parent.join(format!(".freeport-{id}.hold")),
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
            let before = tree(&r.source, control, progress)?;
            if before.entries.len() < 2 {
                return fail(crate::i18n::t("msg.bd053e736a"));
            }
            if p::free_space(&r.target)? < bytes(&before).saturating_add(64 * 1024 * 1024) {
                return fail(crate::i18n::t("msg.4e024f5f24"));
            }
            fs::create_dir_all(r.target.parent().unwrap())?;
            phase(store, &mut r, "copying")?;
            copy(&r.source, &r.stage, &before, control, progress)?;
            let copied = tree(&r.stage, control, progress)?;
            verify(&before, &copied)?;
            verify(&before, &tree(&r.source, control, progress)?)?;
            let manifest = Manifest {
                source: before,
                destination: copied,
            };
            checked_write(&manifest_path(store, &r.id), &manifest)?;
            r.destination_id = Some(p::identity(&r.stage)?);
            phase(store, &mut r, "verified")?;
            phase(store, &mut r, "target_commit_intent")?;
            p::rename(&r.stage, &r.target)?;
            phase(store, &mut r, "target_ready")?;
            ensure_identity(&r.source, r.source_id)?;
            phase(store, &mut r, "source_rename_intent")?;
            p::rename(&r.source, &r.hold)?;
            phase(store, &mut r, "source_held")?;
            verify(&manifest.source, &tree(&r.hold, control, progress)?)?;
            phase(store, &mut r, "link_intent")?;
            p::create_junction(&r.source, &r.target)?;
            p::check_junction(&r.source, &r.target)?;
            phase(store, &mut r, "linked")?;
            cleanup(&r.hold, &r.target, &manifest, control, progress)?;
            phase(store, &mut r, "completed")?;
            Ok(())
        })();
        if let Err(e) = result {
            r.error = Some(crate::i18n::describe_error(e.as_ref()));
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
        stage: parent.join(format!(".freeport-{id}.stage")),
        hold: parent.join(format!(".freeport-{id}.hold")),
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
        let before = tree(&target, control, progress)?;
        if p::free_space(parent)? < bytes(&before).saturating_add(64 * 1024 * 1024) {
            return fail(crate::i18n::t("msg.79e02a1c25"));
        }
        phase(store, &mut r, "copying")?;
        copy(&target, &r.stage, &before, control, progress)?;
        let dest = tree(&r.stage, control, progress)?;
        verify(&before, &dest)?;
        verify(&before, &tree(&target, control, progress)?)?;
        checked_write(
            &manifest_path(store, &r.id),
            &Manifest {
                source: before,
                destination: dest,
            },
        )?;
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
    if !p::same(&r.hold, &parent.join(format!(".freeport-{}.hold", r.id)))
        || protected(&r.source)
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
            if !absent(&r.hold) {
                ensure_identity(&r.hold, r.source_id)?;
                if !absent(&r.source) {
                    p::check_junction(&r.source, &r.target)?;
                    ensure_identity(
                        &r.target,
                        r.destination_id.ok_or(crate::i18n::t("msg.fc6987862b"))?,
                    )?;
                    let manifest: Manifest = checked_read(&manifest_path(store, id))?;
                    cleanup(&r.hold, &r.target, &manifest, control, progress)?;
                    phase(store, &mut r, "completed")?;
                    return Ok(());
                }
                p::rename(&r.hold, &r.source)?;
                phase(store, &mut r, "rolled_back")?;
                progress(crate::i18n::format(
                    "msg.833b31a910",
                    &[format!("{}", r.target.display())],
                ));
                return Ok(());
            }
            if !absent(&r.source) && junction::exists(&r.source).unwrap_or(false) {
                p::check_junction(&r.source, &r.target)?;
                ensure_identity(
                    &r.target,
                    r.destination_id.ok_or(crate::i18n::t("msg.fc6987862b"))?,
                )?;
                phase(store, &mut r, "completed")?;
                return Ok(());
            }
            ensure_identity(&r.source, r.source_id)?;
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
            let m: Manifest = checked_read(&manifest_path(store, id))?;
            verify(&m.destination, &tree(&r.source, control, progress)?)?;
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
    fn detects_same_size_damage() {
        let a = temp();
        let b = temp();
        fs::write(a.join("a"), b"abcd").unwrap();
        fs::write(b.join("a"), b"abce").unwrap();
        let c = Control::default();
        assert!(verify(
            &tree(&a, &c, &|_| {}).unwrap(),
            &tree(&b, &c, &|_| {}).unwrap()
        )
        .is_err());
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
    fn canceled_before_snapshot_preserves_source() {
        let root = temp();
        fs::write(root.join("a"), b"safe").unwrap();
        let c = Control::default();
        c.cancel.store(true, Ordering::Relaxed);
        assert!(tree(&root, &c, &|_| {}).is_err());
        assert_eq!(fs::read(root.join("a")).unwrap(), b"safe");
    }
    fn copy_fixture() -> (PathBuf, PathBuf, Manifest) {
        let src = temp();
        fs::create_dir(src.join("empty")).unwrap();
        fs::write(src.join("样本.txt"), b"original").unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("dest-{}", now().unwrap()));
        let c = Control::default();
        let original = tree(&src, &c, &|_| {}).unwrap();
        copy(&src, &dest, &original, &c, &|_| {}).unwrap();
        let target = tree(&dest, &c, &|_| {}).unwrap();
        verify(&original, &target).unwrap();
        (
            src,
            dest,
            Manifest {
                source: original,
                destination: target,
            },
        )
    }
    #[test]
    fn native_copy_preserves_content_empty_dirs_acl_times() {
        let (src, dest, _) = copy_fixture();
        assert_eq!(
            fs::read(src.join("样本.txt")).unwrap(),
            fs::read(dest.join("样本.txt")).unwrap()
        );
        assert!(dest.join("empty").is_dir());
    }
    #[test]
    fn cleanup_preserves_new_unknown_file() {
        let (src, dest, m) = copy_fixture();
        fs::write(src.join("new.txt"), b"new data").unwrap();
        assert!(cleanup(&src, &dest, &m, &Control::default(), &|_| {}).is_err());
        assert_eq!(fs::read(src.join("new.txt")).unwrap(), b"new data");
        assert_eq!(fs::read(dest.join("样本.txt")).unwrap(), b"original");
    }
    #[test]
    fn cleanup_refuses_changed_destination() {
        let (src, dest, m) = copy_fixture();
        fs::write(dest.join("样本.txt"), b"modified").unwrap();
        assert!(cleanup(&src, &dest, &m, &Control::default(), &|_| {}).is_err());
        assert_eq!(fs::read(src.join("样本.txt")).unwrap(), b"original");
    }
    #[test]
    fn cleanup_refuses_changed_source() {
        let (src, dest, m) = copy_fixture();
        fs::write(src.join("样本.txt"), b"modified").unwrap();
        assert!(cleanup(&src, &dest, &m, &Control::default(), &|_| {}).is_err());
        assert_eq!(fs::read(src.join("样本.txt")).unwrap(), b"modified");
    }
    #[test]
    fn copied_ads_are_verified() {
        let src = temp();
        let f = src.join("a");
        fs::write(&f, b"default").unwrap();
        fs::write(stream_path(&f, ":metadata:$DATA"), b"alternate data").unwrap();
        let c = Control::default();
        let a = tree(&src, &c, &|_| {}).unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("ads-{}", now().unwrap()));
        copy(&src, &dest, &a, &c, &|_| {}).unwrap();
        let b = tree(&dest, &c, &|_| {}).unwrap();
        verify(&a, &b).unwrap();
        fs::write(stream_path(&dest.join("a"), ":metadata:$DATA"), b"corrupt").unwrap();
        assert!(verify(&a, &tree(&dest, &c, &|_| {}).unwrap()).is_err());
    }
    #[test]
    fn readonly_files_can_be_copied_and_verified() {
        let src = temp();
        let f = src.join("a");
        fs::write(&f, b"readonly").unwrap();
        let mut perm = fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&f, perm).unwrap();
        let c = Control::default();
        let a = tree(&src, &c, &|_| {}).unwrap();
        let dest = src
            .parent()
            .unwrap()
            .join(format!("readonly-{}", now().unwrap()));
        copy(&src, &dest, &a, &c, &|_| {}).unwrap();
        verify(&a, &tree(&dest, &c, &|_| {}).unwrap()).unwrap();
        assert!(fs::metadata(dest.join("a"))
            .unwrap()
            .permissions()
            .readonly());
    }
    #[test]
    fn external_hardlinks_are_blocked() {
        let a = temp();
        let b = temp();
        fs::write(a.join("a"), b"shared").unwrap();
        fs::hard_link(a.join("a"), b.join("outside")).unwrap();
        assert!(tree(&a, &Control::default(), &|_| {}).is_err());
    }
    #[test]
    fn internal_hardlinks_keep_identity() {
        let a = temp();
        fs::write(a.join("a"), b"shared").unwrap();
        fs::hard_link(a.join("a"), a.join("b")).unwrap();
        let c = Control::default();
        let before = tree(&a, &c, &|_| {}).unwrap();
        let dest = a
            .parent()
            .unwrap()
            .join(format!("hardlinks-{}", now().unwrap()));
        copy(&a, &dest, &before, &c, &|_| {}).unwrap();
        verify(&before, &tree(&dest, &c, &|_| {}).unwrap()).unwrap();
        assert_eq!(
            p::identity(&dest.join("a")).unwrap(),
            p::identity(&dest.join("b")).unwrap()
        );
    }
    #[test]
    fn nested_junction_is_not_traversed() {
        let a = temp();
        let outside = temp();
        fs::write(outside.join("safe"), b"untouched").unwrap();
        p::create_junction(&a.join("link"), &outside).unwrap();
        assert!(tree(&a, &Control::default(), &|_| {}).is_err());
        assert_eq!(fs::read(outside.join("safe")).unwrap(), b"untouched");
    }
}
