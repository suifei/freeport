//! Read-only NTFS name discovery. No external indexer or raw volume writes.
use crate::{
    catalog::{self},
    fail,
    inventory::{self, Row, Stats},
    platform as p, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Storage::FileSystem::*,
    System::{Ioctl::*, IO::DeviceIoControl},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Volume {
    pub root: PathBuf,
    pub filesystem: String,
    pub free: u64,
}
/// Actual registered/current user profiles on a selected local drive. A portable
/// Users/<name> profile also qualifies if NTUSER.DAT exists. No recursive search.
pub fn profile_roots(volume: &Path) -> Vec<PathBuf> {
    use windows_sys::Win32::{Foundation::ERROR_SUCCESS, System::Registry::*};
    let mut found = vec![];
    if let Some(home) = std::env::var_os("USERPROFILE") {
        found.push(PathBuf::from(home));
    }
    unsafe {
        let mut key = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            p::wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList").as_ptr(),
            0,
            KEY_READ,
            &mut key,
        ) == ERROR_SUCCESS
        {
            let mut index = 0;
            loop {
                let mut name = [0u16; 256];
                let mut length = name.len() as u32;
                let result = RegEnumKeyExW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if result != ERROR_SUCCESS {
                    break;
                }
                index += 1;
                let sid = String::from_utf16_lossy(&name[..length as usize]);
                if !(sid.starts_with("S-1-5-21-") || sid.starts_with("S-1-12-1-")) {
                    continue;
                }
                let mut value = vec![0u16; 32768];
                let mut bytes = (value.len() * 2) as u32;
                if RegGetValueW(
                    key,
                    name.as_ptr(),
                    p::wide("ProfileImagePath").as_ptr(),
                    RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                    std::ptr::null_mut(),
                    value.as_mut_ptr() as _,
                    &mut bytes,
                ) == ERROR_SUCCESS
                {
                    let n = value.iter().position(|c| *c == 0).unwrap_or(value.len());
                    if let Ok(path) = String::from_utf16(&value[..n]) {
                        found.push(PathBuf::from(path));
                    }
                }
            }
            RegCloseKey(key);
        }
    }
    let users = volume.join("Users");
    if fs::symlink_metadata(&users).is_ok_and(|m| m.is_dir() && !p::reparse(&m)) {
        if let Ok(entries) = fs::read_dir(&users) {
            for entry in entries.flatten() {
                if fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir() && !p::reparse(&m))
                    && fs::symlink_metadata(entry.path().join("NTUSER.DAT")).is_ok()
                {
                    found.push(entry.path());
                }
            }
        }
    }
    let mut seen = HashSet::new();
    found.retain(|home| {
        p::within(home, volume)
            && seen.insert(p::key(home))
            && fs::symlink_metadata(home).is_ok_and(|m| m.is_dir() && !p::reparse(&m))
            && home
                .ancestors()
                .all(|a| fs::symlink_metadata(a).is_ok_and(|m| !p::reparse(&m)))
    });
    found.sort_by_key(|h| p::key(h));
    found
}
pub fn volumes() -> Vec<Volume> {
    let mut out = vec![];
    for n in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", n as char));
        let mut fs = [0u16; 32];
        let ok = unsafe {
            GetVolumeInformationW(
                p::wide(&root).as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                32,
            )
        };
        if ok != 0 {
            out.push(Volume {
                free: p::free_space(&root).unwrap_or(0),
                root,
                filesystem: String::from_utf16_lossy(
                    &fs[..fs.iter().position(|c| *c == 0).unwrap_or(32)],
                ),
            });
        }
    }
    out
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub mode: String,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
    pub backends: Vec<String>,
    #[serde(default)]
    pub scope_roots: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
struct Name {
    id: u64,
    parent: u64,
    name: Vec<u16>,
    attrs: u32,
}
fn u16_at(b: &[u8], n: usize) -> u16 {
    u16::from_le_bytes(b[n..n + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], n: usize) -> u32 {
    u32::from_le_bytes(b[n..n + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(b[n..n + 8].try_into().unwrap())
}
fn parse(b: &[u8]) -> Result<(u64, Vec<Name>)> {
    if b.len() < 8 {
        return fail(crate::i18n::t("msg.8d04ca6e68"));
    }
    let mut at = 8;
    let mut out = vec![];
    while at < b.len() {
        if at + 60 > b.len() {
            return fail(crate::i18n::t("msg.3772a37384"));
        }
        let len = u32_at(b, at) as usize;
        if len < 60 || len % 8 != 0 || at + len > b.len() {
            return fail(crate::i18n::t("msg.8e82d3336e"));
        }
        if u16_at(b, at + 4) != 2 {
            return fail(crate::i18n::t("msg.973f7e8a7c"));
        }
        let attrs = u32_at(b, at + 52);
        let nl = u16_at(b, at + 56) as usize;
        let no = u16_at(b, at + 58) as usize;
        if nl % 2 != 0 || no < 60 || no + nl > len {
            return fail(crate::i18n::t("msg.1305738e8b"));
        }
        if attrs & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            let name = b[at + no..at + no + nl]
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            out.push(Name {
                id: u64_at(b, at + 8),
                parent: u64_at(b, at + 16),
                name,
                attrs,
            });
        }
        at += len;
    }
    Ok((u64_at(b, 0), out))
}
/// NTFS always stores the volume root directory in MFT record 5. FSCTL_ENUM_USN_DATA never
/// returns that record itself, so a parent reference to it has to be recognised directly.
const ROOT_RECORD: u64 = 5;
fn resolve(
    id: u64,
    map: &HashMap<u64, Name>,
    cache: &mut HashMap<u64, PathBuf>,
    root: &Path,
    depth: usize,
) -> Option<PathBuf> {
    if depth > 256 {
        return None;
    }
    if id & 0xFFFF_FFFF_FFFF == ROOT_RECORD {
        return Some(root.to_path_buf());
    }
    if let Some(v) = cache.get(&id) {
        return Some(v.clone());
    }
    let n = map.get(&id)?;
    let path = if n.id == n.parent || n.name == vec![b'.' as u16] {
        root.to_path_buf()
    } else {
        resolve(n.parent, map, cache, root, depth + 1)?.join(String::from_utf16(&n.name).ok()?)
    };
    cache.insert(id, path.clone());
    Some(path)
}
fn mft(
    root: &Path,
    progress: &dyn Fn(String),
    control: &crate::engine::Control,
) -> Result<Vec<(PathBuf, u32)>> {
    let s = root.to_string_lossy();
    if s.len() < 2 {
        return fail(crate::i18n::t("msg.45200e48e4"));
    }
    let device = PathBuf::from(format!(r"\\.\{}", &s[..2]));
    let f = p::native_file(&device, 0x80000000, FILE_SHARE_READ | FILE_SHARE_WRITE, 0)?;
    let mut cursor = 0u64;
    let mut map = HashMap::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        control.check()?;
        let input = [
            cursor.to_le_bytes(),
            0i64.to_le_bytes(),
            i64::MAX.to_le_bytes(),
        ]
        .concat();
        let mut n = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle() as _,
                FSCTL_ENUM_USN_DATA,
                input.as_ptr() as _,
                input.len() as u32,
                buf.as_mut_ptr() as _,
                buf.len() as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(38) {
                break;
            }
            return Err(e.into());
        }
        let (next, names) = parse(&buf[..n as usize])?;
        if next <= cursor {
            return fail(crate::i18n::t("msg.085096d0a0"));
        }
        cursor = next;
        for item in names {
            map.insert(item.id, item);
        }
        if map.len() % 10000 < 1000 {
            progress(crate::i18n::format(
                "msg.895b04c7e7",
                &[format!("{}", map.len())],
            ));
        }
    }
    let mut cache = HashMap::new();
    let mut out = vec![];
    for n in map.values() {
        control.check()?;
        if let Some(path) = resolve(n.id, &map, &mut cache, root, 0) {
            out.push((path, n.attrs));
        }
    }
    Ok(out)
}
fn fallback(
    root: &Path,
    progress: &dyn Fn(String),
    control: &crate::engine::Control,
) -> Result<(Vec<(PathBuf, u32)>, usize)> {
    let mut out = vec![];
    let mut errors = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        control.check()?;
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                use std::os::windows::fs::MetadataExt;
                let a = m.file_attributes();
                if m.is_dir() || p::reparse(&m) {
                    out.push((path.clone(), a));
                }
                if m.is_dir() && !p::reparse(&m) {
                    match fs::read_dir(&path) {
                        Ok(ds) => {
                            for d in ds {
                                match d {
                                    Ok(d) => {
                                        if d.file_type()
                                            .map(|t| t.is_dir() || t.is_symlink())
                                            .unwrap_or(false)
                                        {
                                            stack.push(d.path());
                                        }
                                    }
                                    Err(_) => errors += 1,
                                }
                            }
                        }
                        Err(_) => errors += 1,
                    }
                }
            }
            Err(_) => errors += 1,
        }
        if out.len() % 10000 == 0 {
            progress(crate::i18n::format(
                "msg.165a9490d9",
                &[format!("{}", out.len())],
            ));
        }
    }
    Ok((out, errors))
}
pub fn run(roots: &[PathBuf], mode: &str, progress: &dyn Fn(String)) -> Result<Report> {
    run_with_control(roots, mode, progress, &crate::engine::Control::default())
}
pub fn run_with_control(
    roots: &[PathBuf],
    mode: &str,
    progress: &dyn Fn(String),
    control: &crate::engine::Control,
) -> Result<Report> {
    run_with_store(
        roots,
        mode,
        progress,
        control,
        &crate::engine::default_store(),
    )
}
pub fn run_with_store(
    roots: &[PathBuf],
    mode: &str,
    progress: &dyn Fn(String),
    control: &crate::engine::Control,
    store: &Path,
) -> Result<Report> {
    control.check()?;
    if roots.is_empty() || !["migrate", "restore"].contains(&mode) {
        return fail(crate::i18n::t("msg.d08f350885"));
    }
    let mut paths = vec![];
    let mut warnings = vec![];
    let mut backends = vec![];
    let mut scope_roots: Vec<PathBuf> = vec![];
    let mut seen = HashSet::new();
    for input in roots {
        let root = p::absolute(input)?;
        if p::reparse(&fs::symlink_metadata(&root)?) {
            return fail(crate::i18n::t("msg.b9f5d6aa1c"));
        }
        progress(crate::i18n::format(
            "msg.66a30391b5",
            &[format!("{}", root.display())],
        ));
        let scopes = if mode == "migrate" && root.parent().is_none() {
            profile_roots(&root)
        } else {
            vec![root.clone()]
        };
        if scopes.is_empty() {
            warnings.push(crate::i18n::format(
                "scan.no_profiles",
                &[root.display().to_string()],
            ));
            continue;
        }
        for scope in &scopes {
            if !scope_roots.iter().any(|p| p::same(p, scope)) {
                scope_roots.push(scope.clone());
            }
        }
        let list = if root.parent().is_none() {
            match mft(&root, progress, control) {
                Ok(v) => {
                    backends.push(format!("{}: NTFS MFT/USN", root.display()));
                    warnings.push(crate::i18n::t("msg.ac53280a0d").into());
                    v.into_iter()
                        .filter(|(path, _)| scopes.iter().any(|scope| p::within(path, scope)))
                        .collect()
                }
                Err(e) => {
                    control.check()?;
                    let warning = crate::i18n::format(
                        "msg.94008b276b",
                        &[
                            format!("{}", root.display()),
                            crate::i18n::describe_error(e.as_ref()),
                        ],
                    );
                    progress(warning.clone());
                    warnings.push(warning);
                    let mut v = vec![];
                    let mut n = 0;
                    for scope in &scopes {
                        let (part, errors) = fallback(scope, progress, control)?;
                        v.extend(part);
                        n += errors;
                    }
                    if n > 0 {
                        warnings.push(crate::i18n::format(
                            "msg.fd0c0bf5a8",
                            &[format!("{}", root.display()), format!("{}", n)],
                        ));
                    }
                    backends.push(format!("{}: Win32 fallback", root.display()));
                    v
                }
            }
        } else {
            let (v, n) = fallback(&root, progress, control)?;
            backends.push(format!("{}: scoped Win32", root.display()));
            if n > 0 {
                warnings.push(crate::i18n::format("msg.d220bb16a7", &[format!("{}", n)]));
            }
            v
        };
        paths.extend(list);
    }
    let mut candidates = vec![];
    let mut pack = if mode == "restore" {
        crate::rules::builtin()
    } else {
        crate::rules::active(store)?
    };
    if mode == "migrate" && pack.schema_version == 1 {
        warnings.push(crate::i18n::t("rules.legacy").into());
        pack = crate::rules::builtin();
    }
    if mode == "restore" {
        // Whole-volume scans list what can be moved back; links that stay on the same
        // drive free nothing and are usually owned by an application (pnpm, shared skills).
        let volumes: Vec<_> = roots.iter().filter(|r| r.parent().is_none()).collect();
        let (mut managed, mut packages, mut local) = (0usize, 0usize, 0usize);
        for (path, a) in &paths {
            if a & FILE_ATTRIBUTE_REPARSE_POINT == 0
                || a & FILE_ATTRIBUTE_DIRECTORY == 0
                || container_layer(path)
            {
                continue;
            }
            let target = if junction::exists(path).unwrap_or(false) {
                junction::get_target(path).ok()
            } else if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                fs::read_link(path).ok()
            } else {
                continue;
            };
            if a & FILE_ATTRIBUTE_SYSTEM != 0 || windows_managed(path) {
                managed += 1;
            } else if path
                .components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("node_modules"))
            {
                packages += 1;
            } else if volumes.iter().any(|v| p::within(path, v))
                && target.is_some_and(|t| same_drive(path, &t))
            {
                local += 1;
            } else {
                candidates.push(catalog::custom(
                    path,
                    crate::i18n::t("msg.578847e2b1"),
                    "link",
                ));
            }
        }
        if managed + packages + local > 0 {
            warnings.push(crate::i18n::format(
                "scan.skipped_links",
                &[managed.to_string(), packages.to_string(), local.to_string()],
            ));
        }
    } else {
        for root in roots {
            if root.parent().is_some() {
                candidates.push(catalog::custom(
                    root,
                    crate::i18n::t("msg.73fc5c2ef0"),
                    "custom",
                ));
            }
        }
        let mut homes = HashSet::new();
        if let Some(h) = std::env::var_os("USERPROFILE") {
            homes.insert(PathBuf::from(h));
        }
        for root in roots {
            if root.parent().is_none() {
                homes.extend(profile_roots(root));
            }
        }
        let homes: Vec<_> = homes.into_iter().collect();
        let indexed: Vec<_> = paths.iter().map(|(p, _)| p.clone()).collect();
        candidates.extend(
            crate::rules::discover(&pack, &homes, &indexed)
                .into_iter()
                .filter(|c| roots.iter().any(|r| p::within(&c.source, r))),
        );
    }
    let ps = p::processes();
    let mut rows = vec![];
    for candidate in candidates {
        control.check()?;
        if !seen.insert(p::key(&candidate.source)) {
            continue;
        }
        let linked = fs::symlink_metadata(&candidate.source)
            .map(|m| p::reparse(&m))
            .unwrap_or(false);
        let link_target = if linked {
            junction::get_target(&candidate.source)
                .or_else(|_| fs::read_link(&candidate.source))
                .ok()
        } else {
            None
        };
        let busy = ps
            .iter()
            .filter(|(pid, n)| {
                *pid != std::process::id()
                    && candidate
                        .processes
                        .iter()
                        .any(|x| n.eq_ignore_ascii_case(x))
            })
            .map(|(pid, n)| format!("{n} (PID {pid})"))
            .collect();
        rows.push(Row {
            candidate,
            stats: Stats::default(),
            status: if linked { "linked" } else { "unmeasured" }.into(),
            link_target,
            busy,
        });
    }
    rows.sort_by(|a, b| a.candidate.source.cmp(&b.candidate.source));
    Ok(Report {
        version: 1,
        mode: mode.into(),
        rows,
        warnings,
        backends,
        scope_roots,
    })
}
/// Links Windows creates and owns: legacy profile aliases carry the system attribute;
/// Store app moves, Defender platform versions and anything under \Windows do not.
fn windows_managed(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    parts.first().is_some_and(|c| c == "windows")
        || parts.iter().any(|c| c == "windowsapps")
        || parts.starts_with(&[
            "programdata".into(),
            "microsoft".into(),
            "windows defender".into(),
        ])
}
/// Windows Sandbox/container image layers hold ~100k non-link reparse directories;
/// opening each one dominated whole-volume restore scans.
fn container_layer(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .take(4)
        .collect();
    parts == ["programdata", "microsoft", "windows", "containers"]
}
fn same_drive(link: &Path, target: &Path) -> bool {
    let drive = |p: &Path| match p.components().next() {
        Some(std::path::Component::Prefix(x)) => match x.kind() {
            std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => {
                Some(d.to_ascii_uppercase())
            }
            _ => None,
        },
        _ => None,
    };
    drive(link).is_some() && drive(link) == drive(target)
}
pub fn measure(rows: &mut [Row], progress: &dyn Fn(String)) {
    let _ = measure_with_control(rows, progress, &crate::engine::Control::default());
}
pub fn measure_with_control(
    rows: &mut [Row],
    progress: &dyn Fn(String),
    control: &crate::engine::Control,
) -> Result<()> {
    for row in rows {
        if row.status == "linked" {
            continue;
        }
        progress(crate::i18n::format(
            "msg.09382fcf4c",
            &[format!("{}", row.candidate.name)],
        ));
        row.stats = inventory::measure_for_display(&row.candidate.source, control)?;
        row.status = if row.stats.errors > 0 {
            "unreadable"
        } else if row.stats.links > 0 {
            "nested_links"
        } else {
            "ready"
        }
        .into();
    }
    Ok(())
}
pub fn relabel(rows: &mut [Row]) {
    let pack = crate::rules::active(&crate::engine::default_store())
        .unwrap_or_else(|_| crate::rules::builtin());
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let known = home.map(|h| catalog::discover(&h, &[])).unwrap_or_default();
    for row in rows {
        if let Some(rule) = pack.rules.iter().find(|r| {
            row.candidate.knowledge.is_some() && crate::rules::matches(&row.candidate.source, r)
        }) {
            let candidate = crate::rules::candidate(&row.candidate.source, rule);
            row.candidate.name = candidate.name;
            row.candidate.note = candidate.note;
        } else if let Some(candidate) = known
            .iter()
            .find(|c| p::same(&c.source, &row.candidate.source))
        {
            row.candidate.name = candidate.name.clone();
            row.candidate.note = candidate.note.clone();
        } else if row.status == "linked" {
            row.candidate.name = crate::i18n::t("msg.578847e2b1").into();
        } else if row.candidate.category == "custom" {
            row.candidate.name = crate::i18n::t("msg.73fc5c2ef0").into();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_scope_requires_a_real_user_profile() {
        let root = std::env::temp_dir().join(format!("fp-profiles-{}", std::process::id()));
        fs::create_dir_all(root.join("Users/alice/AppData/Local")).unwrap();
        fs::create_dir_all(root.join("Users/not-a-profile/AppData")).unwrap();
        fs::create_dir_all(root.join("Windows/cache")).unwrap();
        fs::write(root.join("Users/alice/NTUSER.DAT"), b"metadata only").unwrap();
        let profiles = profile_roots(&root);
        assert_eq!(profiles.len(), 1);
        assert!(p::same(&profiles[0], &root.join("Users/alice")));
        fs::remove_file(root.join("Users/alice/NTUSER.DAT")).unwrap();
        for dir in [
            "Users/alice/AppData/Local",
            "Users/alice/AppData",
            "Users/alice",
            "Users/not-a-profile/AppData",
            "Users/not-a-profile",
            "Users",
            "Windows/cache",
            "Windows",
            "",
        ] {
            fs::remove_dir(root.join(dir)).unwrap();
        }
    }
    #[test]
    fn windows_owned_links_are_not_restore_candidates() {
        for path in [
            r"C:\Windows\ServiceProfiles\x\Application Data",
            r"C:\Program Files\WindowsApps\Vendor.App_1.0_x64__id",
            r"C:\ProgramData\Microsoft\Windows Defender\Platform\CurrentVersion",
        ] {
            assert!(windows_managed(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\Users\x\AppData\Roaming\Cursor",
            r"D:\Windows-backup\data",
            r"C:\Users\x\Windows\link",
        ] {
            assert!(!windows_managed(Path::new(path)), "{path}");
        }
        assert!(container_layer(Path::new(
            r"C:\ProgramData\Microsoft\Windows\Containers\BaseImages\x\y"
        )));
        assert!(!container_layer(Path::new(
            r"C:\ProgramData\Microsoft\Windows"
        )));
        assert!(same_drive(Path::new(r"C:\a"), Path::new(r"c:\b")));
        assert!(same_drive(Path::new(r"C:\a"), Path::new(r"\\?\C:\b")));
        assert!(!same_drive(Path::new(r"C:\a"), Path::new(r"D:\b")));
        assert!(!same_drive(Path::new(r"C:\a"), Path::new(r"..\b")));
        assert!(!same_drive(
            Path::new(r"C:\a"),
            Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\b")
        ));
    }
    #[test]
    fn rejects_malformed_records() {
        assert!(parse(&[0; 7]).is_err());
        let mut x = vec![0; 68];
        x[8..12].copy_from_slice(&60u32.to_le_bytes());
        assert!(parse(&x).is_err());
    }
    #[test]
    fn parses_v2_directory() {
        let mut x = vec![0u8; 72];
        x[..8].copy_from_slice(&9u64.to_le_bytes());
        x[8..12].copy_from_slice(&64u32.to_le_bytes());
        x[12..14].copy_from_slice(&2u16.to_le_bytes());
        x[60..64].copy_from_slice(&FILE_ATTRIBUTE_DIRECTORY.to_le_bytes());
        x[64..66].copy_from_slice(&2u16.to_le_bytes());
        x[66..68].copy_from_slice(&60u16.to_le_bytes());
        x[68..70].copy_from_slice(&('a' as u16).to_le_bytes());
        assert_eq!(parse(&x).unwrap().1.len(), 1);
    }
    #[test]
    fn paths_resolve_without_an_enumerated_root_record() {
        // Real volumes: children of the root point at record 5 (with a sequence number in the
        // high 16 bits), and the enumeration never contains record 5 itself.
        let root_ref = (5u64 << 48) | ROOT_RECORD;
        let name = |id, parent, s: &str| Name {
            id,
            parent,
            name: s.encode_utf16().collect(),
            attrs: FILE_ATTRIBUTE_DIRECTORY,
        };
        let map: HashMap<u64, Name> = [
            (
                0x10_0000_0000_1001,
                name(0x10_0000_0000_1001, root_ref, "Users"),
            ),
            (
                0x20_0000_0000_2002,
                name(0x20_0000_0000_2002, 0x10_0000_0000_1001, "me"),
            ),
            (
                0x30_0000_0000_3003,
                name(0x30_0000_0000_3003, 0x77_0000_0000_9999, "orphan"),
            ),
        ]
        .into_iter()
        .collect();
        let root = Path::new(r"C:\");
        let mut cache = HashMap::new();
        assert_eq!(
            resolve(0x20_0000_0000_2002, &map, &mut cache, root, 0).unwrap(),
            Path::new(r"C:\Users\me")
        );
        assert!(resolve(0x30_0000_0000_3003, &map, &mut cache, root, 0).is_none());
    }
    #[test]
    fn twenty_thousand_malformed_buffers_do_not_panic() {
        let mut seed = 0x12345678u32;
        for n in 0..20_000 {
            let mut b = vec![0u8; n % 513];
            for v in &mut b {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *v = (seed >> 24) as u8;
            }
            assert!(std::panic::catch_unwind(|| parse(&b)).is_ok());
        }
    }
    #[test]
    fn cancel_prevents_disk_discovery() {
        let c = crate::engine::Control::default();
        c.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(run_with_control(
            &[PathBuf::from(r"E:\")],
            "migrate",
            &|_| panic!("must not scan"),
            &c
        )
        .is_err());
    }
}
