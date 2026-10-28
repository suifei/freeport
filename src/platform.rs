use crate::{fail, Result};
use serde::{Deserialize, Serialize};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::Diagnostics::ToolHelp::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Identity {
    pub volume: u32,
    pub index: u64,
}
pub fn identity_file(f: &File) -> Result<(Identity, u32)> {
    let mut i: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    unsafe {
        if GetFileInformationByHandle(f.as_raw_handle() as HANDLE, &mut i) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok((
        Identity {
            volume: i.dwVolumeSerialNumber,
            index: ((i.nFileIndexHigh as u64) << 32) | i.nFileIndexLow as u64,
        },
        i.nNumberOfLinks,
    ))
}
pub fn identity(path: &Path) -> Result<Identity> {
    let f = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    Ok(identity_file(&f)?.0)
}
pub fn native_file(path: &Path, access: u32, share: u32, flags: u32) -> Result<File> {
    let h = unsafe {
        CreateFileW(
            path_wide(path).as_ptr(),
            access,
            share,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_handle(h) })
}
pub fn rename(source: &Path, target: &Path) -> Result<()> {
    unsafe {
        if MoveFileExW(
            path_wide(source).as_ptr(),
            path_wide(target).as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
pub fn final_path(f: &File) -> Result<PathBuf> {
    let mut b = vec![0u16; 32768];
    let n = unsafe {
        GetFinalPathNameByHandleW(
            f.as_raw_handle() as HANDLE,
            b.as_mut_ptr(),
            b.len() as u32,
            0,
        )
    };
    if n == 0 || n as usize >= b.len() {
        return Err(std::io::Error::last_os_error().into());
    }
    local_final_path(&String::from_utf16(&b[..n as usize])?)
}
fn local_final_path(value: &str) -> Result<PathBuf> {
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    let path = PathBuf::from(value);
    if value.starts_with("UNC\\")
        || value.starts_with(r"\\")
        || !path.is_absolute()
        || !matches!(path.components().next(),Some(Component::Prefix(p))if matches!(p.kind(),std::path::Prefix::Disk(_)))
    {
        return fail(crate::i18n::t("msg.1f8f68f598"));
    }
    Ok(path)
}

pub fn security(path: &Path) -> Result<Vec<u8>> {
    use windows_sys::Win32::Security::*;
    let bits = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut n = 0;
    unsafe {
        GetFileSecurityW(path_wide(path).as_ptr(), bits, ptr::null_mut(), 0, &mut n);
    }
    if n == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut b = vec![0; n as usize];
    if unsafe {
        GetFileSecurityW(
            path_wide(path).as_ptr(),
            bits,
            b.as_mut_ptr() as _,
            n,
            &mut n,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(b)
}
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn path_wide(p: &Path) -> Vec<u16> {
    let s = p.as_os_str();
    let len = s.encode_wide().count();
    if len >= 248 && !s.to_string_lossy().starts_with(r"\\") {
        let mut pref = std::ffi::OsString::from(r"\\?\");
        pref.push(s);
        wide(pref)
    } else {
        wide(s)
    }
}
pub fn reparse(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
pub fn key(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
pub fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}
pub fn within(p: &Path, root: &Path) -> bool {
    same(p, root) || key(p).starts_with(&(key(root) + "\\"))
}
pub fn overlap(a: &Path, b: &Path) -> bool {
    within(a, b) || within(b, a)
}

pub fn absolute(p: &Path) -> Result<PathBuf> {
    let s = p.to_string_lossy();
    if s.starts_with(r"\\") || !p.is_absolute() || s.contains(['%', '\0']) {
        return fail(crate::i18n::t("msg.1f8f68f598"));
    }
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return fail(crate::i18n::t("msg.a2291a09fe"));
    }
    for c in p.components() {
        if let Component::Normal(n) = c {
            let n = n.to_string_lossy();
            let stem = n
                .split('.')
                .next()
                .unwrap_or("")
                .to_uppercase()
                .replace('¹', "1")
                .replace('²', "2")
                .replace('³', "3");
            let device = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"]
                .contains(&stem.as_str())
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'));
            if device
                || n.trim_end_matches([' ', '.']) != n
                || n.contains([':', '*', '?', '<', '>', '|', '"'])
                || n.chars().any(char::is_control)
            {
                return fail(crate::i18n::t("msg.c488fffda5"));
            }
        }
    }
    let mut existing = p.to_path_buf();
    let mut tail = vec![];
    while fs::symlink_metadata(&existing).is_err() {
        tail.push(
            existing
                .file_name()
                .ok_or(crate::i18n::t("msg.419e02ffd8"))?
                .to_os_string(),
        );
        if !existing.pop() {
            return fail(crate::i18n::t("msg.02ec2c074c"));
        }
    }
    let mut resolved = fs::canonicalize(existing)?;
    for c in tail.into_iter().rev() {
        resolved.push(c);
    }
    local_final_path(&resolved.to_string_lossy())
}

pub fn free_space(path: &Path) -> Result<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return fail(crate::i18n::t("msg.9b4bdab440"));
        }
    }
    let mut available = 0;
    unsafe {
        if GetDiskFreeSpaceExW(
            wide(p).as_ptr(),
            &mut available,
            ptr::null_mut(),
            ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(available)
}

pub fn ntfs(path: &Path) -> Result<()> {
    let p = absolute(path)?;
    let root = format!(
        "{}\\",
        p.components()
            .next()
            .ok_or(crate::i18n::t("msg.e27c8aea75"))?
            .as_os_str()
            .to_string_lossy()
    );
    let mut name = [0u16; 32];
    unsafe {
        if ![2, 3].contains(&GetDriveTypeW(wide(&root).as_ptr())) {
            return fail(crate::i18n::t("msg.a611755bee"));
        }
        if GetVolumeInformationW(
            wide(root).as_ptr(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let fs =
        String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]);
    if fs != "NTFS" {
        return fail(crate::i18n::format("msg.8b4edf812c", &[format!("{}", fs)]));
    }
    Ok(())
}

pub fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = path.with_extension(format!("{}-{stamp}.tmp", std::process::id()));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.sync_all()?;
    drop(f);
    unsafe {
        if MoveFileExW(
            path_wide(&temp).as_ptr(),
            path_wide(path).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

pub fn lock(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .map_err(|e| crate::i18n::format("msg.b95d01d42e", &[crate::i18n::describe_error(&e)]))?)
}
pub fn read_guard(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(path)
        .map_err(|e| {
            crate::i18n::format(
                "msg.4e66c1f6af",
                &[
                    format!("{}", path.display()),
                    crate::i18n::describe_error(&e),
                ],
            )
        })?)
}
pub fn stable_read_guard(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)?)
}
pub fn dir_guard(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| {
            crate::i18n::format(
                "msg.91352f1748",
                &[
                    format!("{}", path.display()),
                    crate::i18n::describe_error(&e),
                ],
            )
        })?)
}
/// Pin existing directory components from the volume root downwards. Never follow a reparse point.
pub fn pin_ancestors(path: &Path) -> Result<Vec<File>> {
    let chain: Vec<_> = path.ancestors().collect();
    let mut held = vec![];
    for parent in chain.into_iter().rev() {
        let meta = match fs::symlink_metadata(parent) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        };
        if !meta.is_dir() || reparse(&meta) {
            return fail(crate::i18n::t("msg.c78ecaa390"));
        }
        let file = native_file(
            parent,
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        if reparse(&file.metadata()?) || identity_file(&file)?.0 != identity(parent)? {
            return fail(crate::i18n::t("msg.c78ecaa390"));
        }
        held.push(file);
    }
    Ok(held)
}

pub fn processes() -> Vec<(u32, String)> {
    let mut out = vec![];
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut p: PROCESSENTRY32W = std::mem::zeroed();
        p.dwSize = std::mem::size_of_val(&p) as u32;
        let mut ok = Process32FirstW(h, &mut p);
        while ok != 0 {
            let n = p
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(p.szExeFile.len());
            out.push((
                p.th32ProcessID,
                String::from_utf16_lossy(&p.szExeFile[..n]).to_lowercase(),
            ));
            ok = Process32NextW(h, &mut p);
        }
        CloseHandle(h);
    }
    out
}
pub fn blockers(names: &[String]) -> Vec<String> {
    processes()
        .into_iter()
        .filter(|(pid, n)| {
            *pid != std::process::id() && names.iter().any(|x| n.eq_ignore_ascii_case(x))
        })
        .map(|(pid, n)| format!("{n} (PID {pid})"))
        .collect()
}
pub fn create_junction(link: &Path, target: &Path) -> Result<()> {
    crate::system_ops::junction(link, target, &crate::engine::Control::default())
}
pub fn check_junction(link: &Path, target: &Path) -> Result<()> {
    if !junction::exists(link)?
        || !same(&absolute(&junction::get_target(link)?)?, &absolute(target)?)
    {
        return fail(crate::i18n::format(
            "msg.6da5c00f50",
            &[format!("{}", link.display())],
        ));
    }
    Ok(())
}
pub fn unlink_junction(link: &Path, target: &Path) -> Result<()> {
    check_junction(link, target)?;
    junction::delete(link)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_paths_must_remain_local() {
        for value in [
            r"\\?\UNC\server\share",
            r"\\server\share",
            r"UNC\server\share",
            r"\\?\Volume{abc}\path",
            r"relative\path",
        ] {
            assert!(local_final_path(value).is_err(), "{value}");
        }
        assert_eq!(
            local_final_path(r"\\?\E:\safe").unwrap(),
            PathBuf::from(r"E:\safe")
        );
    }
}
