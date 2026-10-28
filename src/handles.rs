//! User-mode handle discovery and explicit remote file-handle closure.
use crate::{fail, platform as p, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    os::windows::{
        io::{AsRawHandle, FromRawHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{LibraryLoader::*, Threading::*, IO::CancelSynchronousIo},
};

pub fn warning() -> &'static str {
    crate::i18n::force_warning()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub request_app_exit: bool,
    #[serde(default)]
    pub force_close_handles: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenHandle {
    pub pid: u32,
    pub created: u64,
    pub handle: u64,
    pub path: PathBuf,
    pub identity: p::Identity,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    object: usize,
    pid: usize,
    handle: usize,
    access: u32,
    trace: u16,
    kind: u16,
    attrs: u32,
    reserved: u32,
}
type NtQuery = unsafe extern "system" fn(u32, *mut std::ffi::c_void, u32, *mut u32) -> i32;
fn entries() -> Result<Vec<Entry>> {
    let h = unsafe { GetModuleHandleW(p::wide("ntdll.dll").as_ptr()) };
    let symbol = unsafe { GetProcAddress(h, c"NtQuerySystemInformation".as_ptr() as _) }
        .ok_or(crate::i18n::t("msg.ddafa0a53a"))?;
    let query: NtQuery = unsafe { std::mem::transmute(symbol) };
    let mut size = 1024 * 1024;
    loop {
        if size > 256 * 1024 * 1024 {
            return fail(crate::i18n::t("msg.e399792f89"));
        }
        let mut b = vec![0u8; size];
        let mut returned = 0;
        let status = unsafe { query(64, b.as_mut_ptr() as _, size as u32, &mut returned) };
        if status as u32 == 0xc0000004 {
            size = (size * 2).max(returned as usize + 65536);
            continue;
        }
        if status < 0 {
            return fail(crate::i18n::format(
                "msg.4c2e098bb1",
                &[format!("{:#x}", status)],
            ));
        }
        let word = std::mem::size_of::<usize>();
        if (returned as usize) < word * 2 || returned as usize > b.len() {
            return fail(crate::i18n::t("msg.1d19df0b98"));
        }
        let count = usize::from_ne_bytes(b[..word].try_into().unwrap());
        let stride = std::mem::size_of::<Entry>();
        if count > (returned as usize - word * 2) / stride {
            return fail(crate::i18n::t("msg.ae2258e026"));
        }
        return Ok((0..count)
            .map(|i| unsafe {
                std::ptr::read_unaligned(b.as_ptr().add(word * 2 + i * stride) as *const Entry)
            })
            .collect());
    }
}
/// Critical processes are never opened. System-folder binaries (Explorer, shells) are
/// still listed as holders, but `closable` refuses them for remote handle closure.
fn process(pid: u32, closable: bool) -> Result<(File, u64)> {
    let h = unsafe {
        OpenProcess(
            PROCESS_DUP_HANDLE | PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if h.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let f = unsafe { File::from_raw_handle(h) };
    let mut a = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut b = a;
    let mut c = a;
    let mut d = a;
    if unsafe { GetProcessTimes(h, &mut a, &mut b, &mut c, &mut d) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut critical = 0;
    if unsafe { IsProcessCritical(h, &mut critical) } == 0 || critical != 0 {
        return fail(crate::i18n::t("msg.4dfff48e3e"));
    }
    let created = ((a.dwHighDateTime as u64) << 32) | a.dwLowDateTime as u64;
    if !closable {
        return Ok((f, created));
    }
    // Keep system binaries excluded in addition to checking critical-process state.
    let mut image = vec![0u16; 32768];
    let mut len = image.len() as u32;
    if unsafe { QueryFullProcessImageNameW(h, 0, image.as_mut_ptr(), &mut len) } == 0 {
        return fail(crate::i18n::t("msg.9921ddfeaf"));
    }
    let image = String::from_utf16(&image[..len as usize])?.to_lowercase();
    let system = std::env::var("SystemRoot")
        .unwrap_or_else(|_| r"C:\Windows".into())
        .to_lowercase();
    if image.starts_with(&(system + "\\")) {
        return fail(crate::i18n::t("msg.7cf7a3bbfa"));
    }
    Ok((f, created))
}
fn duplicate(proc: &File, value: u64, close: bool) -> Result<File> {
    let mut h = std::ptr::null_mut();
    let flags = DUPLICATE_SAME_ACCESS | if close { DUPLICATE_CLOSE_SOURCE } else { 0 };
    let ok = unsafe {
        DuplicateHandle(
            proc.as_raw_handle() as _,
            value as usize as _,
            GetCurrentProcess(),
            &mut h,
            0,
            0,
            flags,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_handle(h) })
}
fn name(f: &File) -> Result<(PathBuf, p::Identity)> {
    if unsafe { GetFileType(f.as_raw_handle() as _) } != FILE_TYPE_DISK {
        return fail(crate::i18n::t("msg.ddeb1eb7f0"));
    }
    let path = p::final_path(f)?;
    if path.to_string_lossy().starts_with(r"\\") {
        return fail(crate::i18n::t("msg.dfa93a5017"));
    }
    Ok((path, p::identity_file(f)?.0))
}
fn resolve(proc: &File, pid: u32, created: u64, handle: u64) -> Result<OpenHandle> {
    let (path, identity) = name(&duplicate(proc, handle, false)?)?;
    Ok(OpenHandle {
        pid,
        created,
        handle,
        path,
        identity,
    })
}

/// Name queries can block indefinitely on some handles (synchronous pipes, cloud-sync
/// files held under oplock), so each runs on a worker with a deadline. A stuck worker
/// gets CancelSynchronousIo and, if still stuck, is abandoned until the helper exits.
struct Resolver {
    jobs: mpsc::Sender<File>,
    done: mpsc::Receiver<Option<(PathBuf, p::Identity)>>,
    worker: std::thread::JoinHandle<()>,
}
impl Resolver {
    const DEADLINE: Duration = Duration::from_millis(500);
    fn new() -> Self {
        let (jobs, queue) = mpsc::channel::<File>();
        let (reply, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            for f in queue {
                if reply.send(name(&f).ok()).is_err() {
                    break;
                }
            }
        });
        Self { jobs, done, worker }
    }
    /// `None` means the query did not finish in time; the handle stays unresolved.
    fn query(&mut self, f: File) -> Option<Option<(PathBuf, p::Identity)>> {
        if self.jobs.send(f).is_ok() {
            if let Ok(r) = self.done.recv_timeout(Self::DEADLINE) {
                return Some(r);
            }
            unsafe { CancelSynchronousIo(self.worker.as_raw_handle() as _) };
            if self.done.recv_timeout(Duration::from_millis(100)).is_ok() {
                return None;
            }
        }
        *self = Self::new();
        None
    }
}

/// Child query process bounds potentially blocking file-name queries. It never closes remote handles.
pub fn maybe_helper() -> bool {
    if std::env::args().nth(1).as_deref() != Some("__handles-query") {
        return false;
    }
    let _ = crate::i18n::set(&std::env::var("FP_INTERNAL_LANG").unwrap_or_else(|_| "en".into()));
    let result = (|| -> Result<()> {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let roots: Vec<PathBuf> = serde_json::from_str(&input)?;
        let probe = File::open(std::env::current_exe()?)?;
        let es = entries()?;
        let own = std::process::id();
        let kind = es
            .iter()
            .find(|e| e.pid == own as usize && e.handle == probe.as_raw_handle() as usize)
            .map(|e| e.kind)
            .ok_or(crate::i18n::t("msg.59103ea0f6"))?;
        let mut current = 0;
        let mut proc = None;
        let mut resolver = Resolver::new();
        // After two stalls in one process the rest of its handles are counted, not queried.
        let (mut stalls, mut unresolved) = (0, 0usize);
        for e in es
            .into_iter()
            .filter(|e| e.kind == kind && e.pid > 4 && e.pid != own as usize)
        {
            let pid = e.pid as u32;
            if pid != current {
                current = pid;
                proc = process(pid, false).ok();
                stalls = 0;
            }
            let Some((f, created)) = &proc else { continue };
            if stalls >= 2 {
                unresolved += 1;
                continue;
            }
            let Ok(d) = duplicate(f, e.handle as u64, false) else {
                continue;
            };
            match resolver.query(d) {
                Some(Some((path, identity))) if roots.iter().any(|r| p::within(&path, r)) => {
                    let h = OpenHandle {
                        pid,
                        created: *created,
                        handle: e.handle as u64,
                        path,
                        identity,
                    };
                    println!("{}", serde_json::to_string(&h)?);
                    std::io::stdout().flush()?;
                }
                Some(_) => {}
                None => {
                    stalls += 1;
                    unresolved += 1;
                }
            }
        }
        if unresolved > 0 {
            eprintln!(
                "{}",
                crate::i18n::format("handles.unresolved", &[unresolved.to_string()])
            );
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("{}", crate::i18n::describe_error(e.as_ref()));
        std::process::exit(2);
    }
    true
}
pub fn discover(roots: &[PathBuf]) -> Result<Vec<OpenHandle>> {
    Ok(query(roots)?.0)
}
/// Open handles plus the helper's note on handles it could not resolve in time.
pub fn query(roots: &[PathBuf]) -> Result<(Vec<OpenHandle>, String)> {
    let roots: Vec<_> = roots
        .iter()
        .map(|r| p::absolute(r))
        .collect::<Result<_>>()?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("__handles-query")
        .env("FP_INTERNAL_LANG", crate::i18n::language())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(crate::i18n::t("msg.4dcf78b408"))?
        .write_all(serde_json::to_string(&roots)?.as_bytes())?;
    let output = child
        .stdout
        .take()
        .ok_or(crate::i18n::t("msg.af8dd8da1c"))?;
    let diagnostic = child
        .stderr
        .take()
        .ok_or(crate::i18n::t("msg.af8dd8da1c"))?;
    let errors = std::thread::spawn(move || {
        let mut message = String::new();
        let _ = diagnostic.take(65536).read_to_string(&mut message);
        message
    });
    let reader = std::thread::spawn(move || -> Result<Vec<OpenHandle>> {
        let mut result = vec![];
        for line in BufReader::new(output).lines() {
            let line = line?;
            if line.len() > 262144 {
                return fail(crate::i18n::t("msg.2f90b78c06"));
            }
            result.push(serde_json::from_str(&line)?);
        }
        Ok(result)
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let rows = reader
                .join()
                .map_err(|_| crate::i18n::t("msg.9d0a0c8d30"))??;
            if !status.success() {
                return fail(format!(
                    "{}\n{}",
                    crate::i18n::t("msg.95899ff2ea"),
                    errors.join().unwrap_or_default().trim()
                ));
            }
            let note = errors.join().unwrap_or_default().trim().to_string();
            return Ok((
                rows.into_iter()
                    .filter(|h| h.pid != std::process::id())
                    .collect(),
                note,
            ));
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = errors.join();
            return fail(crate::i18n::t("msg.de711cb46b"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A program using a selected directory: it holds a handle inside it, or its executable
/// was started from it. Detection only; nothing is closed.
#[derive(Clone, Debug)]
pub struct User {
    pub root: PathBuf,
    pub pid: u32,
    pub exe: String,
    pub app: String,
    pub runs_inside: bool,
}
fn image(pid: u32) -> Option<PathBuf> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return None;
    }
    let f = unsafe { File::from_raw_handle(h) };
    let mut b = vec![0u16; 32768];
    let mut len = b.len() as u32;
    if unsafe { QueryFullProcessImageNameW(f.as_raw_handle() as _, 0, b.as_mut_ptr(), &mut len) }
        == 0
    {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&b[..len as usize])))
}
/// The product's own description ("Visual Studio Code"), falling back to the file name.
fn app_name(exe: &Path) -> String {
    let fallback = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let wide = p::wide(exe.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return fallback;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr() as _) } == 0 {
        return fallback;
    }
    let query = |key: &str| -> Option<&[u8]> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let key = p::wide(key);
        if unsafe { VerQueryValueW(data.as_ptr() as _, key.as_ptr(), &mut ptr, &mut len) } == 0
            || ptr.is_null()
        {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize * 2) })
    };
    let Some(t) = query(r"\VarFileInfo\Translation").filter(|t| t.len() >= 4) else {
        return fallback;
    };
    let (lang, page) = (
        u16::from_le_bytes([t[0], t[1]]),
        u16::from_le_bytes([t[2], t[3]]),
    );
    query(&format!(
        r"\StringFileInfo\{lang:04x}{page:04x}\FileDescription"
    ))
    .map(|b| {
        let w: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        String::from_utf16_lossy(&w).trim().to_string()
    })
    .filter(|s| !s.is_empty())
    .unwrap_or(fallback)
}
pub fn users(roots: &[PathBuf]) -> Vec<User> {
    let own = std::process::id();
    let mut found: Vec<(PathBuf, u32, bool)> = vec![];
    for (pid, _) in p::processes() {
        if pid <= 4 || pid == own {
            continue;
        }
        if let Some(path) = image(pid) {
            if let Some(root) = roots.iter().find(|r| p::within(&path, r)) {
                found.push((root.clone(), pid, true));
            }
        }
    }
    // A failed or partial handle query only narrows the advice; it never blocks.
    for h in discover(roots).unwrap_or_default() {
        if let Some(root) = roots.iter().find(|r| p::within(&h.path, r)) {
            if !found.iter().any(|(r, pid, _)| *pid == h.pid && r == root) {
                found.push((root.clone(), h.pid, false));
            }
        }
    }
    let mut names = std::collections::HashMap::new();
    found
        .into_iter()
        .filter_map(|(root, pid, runs_inside)| {
            let exe = image(pid)?;
            let app = names
                .entry(exe.clone())
                .or_insert_with(|| app_name(&exe))
                .clone();
            Some(User {
                root,
                pid,
                exe: exe.file_name()?.to_string_lossy().into_owned(),
                app,
                runs_inside,
            })
        })
        .collect()
}
/// Per directory, which programs to quit first, grouped by product.
pub fn advice(users: &[User]) -> String {
    let mut roots: Vec<&PathBuf> = vec![];
    for u in users {
        if !roots.contains(&&u.root) {
            roots.push(&u.root);
        }
    }
    let mut text = String::new();
    for root in roots {
        text.push_str(&crate::i18n::format(
            "handles.users",
            &[root.display().to_string()],
        ));
        let mut groups: Vec<(&str, &str, usize, bool)> = vec![];
        for u in users.iter().filter(|u| &u.root == root) {
            match groups
                .iter_mut()
                .find(|g| g.0 == u.app.as_str() && g.1 == u.exe.as_str())
            {
                Some(g) => {
                    g.2 += 1;
                    g.3 |= u.runs_inside;
                }
                None => groups.push((&u.app, &u.exe, 1, u.runs_inside)),
            }
        }
        for (app, exe, count, inside) in groups {
            text.push_str(&crate::i18n::format(
                if inside {
                    "handles.user_inside"
                } else {
                    "handles.user"
                },
                &[app.into(), exe.into(), count.to_string()],
            ));
        }
    }
    text
}

pub fn force_close(roots: &[PathBuf], audit: &Path, progress: &dyn Fn(String)) -> Result<usize> {
    use std::fs::OpenOptions;
    let mut log = OpenOptions::new().create(true).append(true).open(audit)?;
    let mut closed = 0;
    for h in discover(roots)? {
        let (proc, created) = match process(h.pid, true) {
            Ok(p) => p,
            Err(e) => {
                progress(crate::i18n::format(
                    "msg.af8d941d0e",
                    &[
                        format!("{}", h.pid),
                        crate::i18n::describe_error(e.as_ref()),
                    ],
                ));
                continue;
            }
        };
        if created != h.created {
            continue;
        }
        // Refresh the object immediately before the irreversible close; this narrows, but
        // cannot atomically eliminate, the remote handle-number reuse race.
        let fresh = match resolve(&proc, h.pid, created, h.handle) {
            Ok(h) => h,
            Err(_) => continue,
        };
        if fresh.identity != h.identity || !roots.iter().any(|r| p::within(&fresh.path, r)) {
            continue;
        }
        writeln!(
            log,
            "{}",
            serde_json::json!({"event":"HandleCloseIntent","handle":fresh})
        )?;
        log.sync_all()?;
        let result = duplicate(&proc, h.handle, true);
        let success = result.is_ok();
        drop(result);
        // DUPLICATE_CLOSE_SOURCE can close the original even when the call fails.
        writeln!(
            log,
            "{}",
            serde_json::json!({"event":if success{"HandleCloseReturnedSuccess"}else{"OutcomeUnknown"},"pid":h.pid,"created":h.created,"handle":h.handle})
        )?;
        log.sync_all()?;
        progress(format!(
            "{} PID {} {}",
            if success {
                crate::i18n::t("msg.1e2a84c6f9")
            } else {
                crate::i18n::t("msg.fd20201a48")
            },
            h.pid,
            h.path.display()
        ));
        if success {
            closed += 1;
        }
    }
    Ok(closed)
}
pub fn release(
    roots: &[PathBuf],
    options: &Options,
    store: &Path,
    progress: &dyn Fn(String),
) -> Result<()> {
    std::fs::create_dir_all(store)?;
    // Deferred to the next milestone (docs/requirements.md 2.1). Refuse here as
    // well so a crafted plan or worker request cannot enable them.
    if options.request_app_exit || options.force_close_handles {
        return fail(
            "Requesting app exit and force-closing handles are not available in this release.",
        );
    }
    if options.request_app_exit {
        request_exit(roots, progress)?;
    }
    if options.force_close_handles {
        progress(warning().into());
        force_close(roots, &store.join("handles.jsonl"), progress)?;
    }
    Ok(())
}
fn request_exit(roots: &[PathBuf], progress: &dyn Fn(String)) -> Result<()> {
    use windows_sys::Win32::System::RestartManager::*;
    let hs = discover(roots)?;
    if hs.is_empty() {
        return Ok(());
    }
    let mut session = 0;
    let mut key = [0u16; 33];
    let rc = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if rc != 0 {
        return fail(crate::i18n::format("msg.c6d957325b", &[format!("{}", rc)]));
    }
    struct End(u32);
    impl Drop for End {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }
    let _end = End(session);
    let mut seen = std::collections::HashSet::new();
    let procs: Vec<RM_UNIQUE_PROCESS> = hs
        .iter()
        .filter(|h| seen.insert((h.pid, h.created)))
        .map(|h| RM_UNIQUE_PROCESS {
            dwProcessId: h.pid,
            ProcessStartTime: FILETIME {
                dwLowDateTime: h.created as u32,
                dwHighDateTime: (h.created >> 32) as u32,
            },
        })
        .collect();
    let rc = unsafe {
        RmRegisterResources(
            session,
            0,
            std::ptr::null(),
            procs.len() as u32,
            procs.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return fail(crate::i18n::format("msg.5090caa01e", &[format!("{}", rc)]));
    }
    progress(crate::i18n::t("msg.3aef03afab").into());
    let rc = unsafe { RmShutdown(session, RmShutdownOnlyRegistered as u32, None) };
    if rc != 0 {
        progress(crate::i18n::format("msg.183475d929", &[format!("{}", rc)]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn users_name_programs_started_from_a_directory() {
        let dir = std::env::current_dir()
            .unwrap()
            .join(".build")
            .join("unit-tests")
            .join(format!("users-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let system =
            std::path::PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32");
        std::fs::copy(system.join("PING.EXE"), dir.join("ping.exe")).unwrap();
        // Handle holders need the fp.exe query helper and are covered by acceptance P04.
        let mut child = Command::new(dir.join("ping.exe"))
            .args(["-n", "6", "127.0.0.1"])
            .current_dir(&dir)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let found = users(std::slice::from_ref(&dir));
        let _ = child.kill();
        let _ = child.wait();
        assert!(found
            .iter()
            .any(|u| u.exe.eq_ignore_ascii_case("ping.exe") && u.runs_inside));
        let text = advice(&found);
        assert!(text.contains(&dir.display().to_string()) && text.contains("ping.exe"));
    }
}
